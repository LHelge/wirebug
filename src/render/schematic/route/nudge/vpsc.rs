//! One-dimensional separation-constraint projection — the `satisfy_VPSC`
//! step of paper §6.2.
//!
//! Given variables with desired positions and strengths, and separation
//! constraints `x[right] − x[left] ≥ gap`, find positions that satisfy
//! every constraint while minimising `Σ (xᵢ − dᵢ)²` over the strongest
//! variables first, then the next tier, and so on.
//!
//! Method: the block-merge active-set algorithm of Dwyer, Marriott &
//! Stuckey, *Fast Node Overlap Removal* (LNCS 3843). Each variable
//! starts in its own block at its desired position. Repeatedly take the
//! most-violated constraint joining two different blocks and merge them,
//! making that constraint tight; a block's position is always the
//! average of the desired positions of its strongest tier present
//! ([`Strength`]: `Wall` over `Pin` over `Pull`), offset by each member's
//! place in the block, so weaker members yield entirely. Merge-only (no
//! split) matches the paper's approximate projection and is optimal for
//! the chain/tree constraints nudging produces.

/// How firmly a variable holds its desired position. A block of
/// variables sits where the members of its strongest tier want it on
/// average; weaker members yield to them entirely. That is the limit of
/// weights growing apart without bound, solved exactly: large finite
/// weights would leave a held position off by `(d − held) / weight`,
/// enough to put a wire at `360.00000002` instead of on its grid line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Strength {
    /// An interior segment, free to be nudged.
    Pull,
    /// A segment anchored at a port.
    Pin,
    /// An obstacle boundary.
    Wall,
}

impl Strength {
    const COUNT: usize = 3;

    fn tier(self) -> usize {
        self as usize
    }
}

/// A variable to place: held at `desired` with `strength`.
pub(super) struct Var {
    pub(super) desired: f64,
    pub(super) strength: Strength,
}

/// `position[right] − position[left] ≥ gap`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Constraint {
    pub(super) left: usize,
    pub(super) right: usize,
    pub(super) gap: f64,
}

const TOL: f64 = 1e-9;

/// The members of one strength tier in a block.
#[derive(Clone, Copy, Default)]
struct Tier {
    count: f64,
    /// Σ (desired − offset) over the tier's members.
    sum: f64,
}

struct Block {
    /// Indexed by [`Strength::tier`].
    tiers: [Tier; Strength::COUNT],
    members: Vec<usize>,
}

impl Block {
    /// The average reference position of the strongest tier present.
    fn position(&self) -> f64 {
        self.tiers
            .iter()
            .rev()
            .find(|tier| tier.count > 0.0)
            .map_or(0.0, |tier| tier.sum / tier.count)
    }
}

/// Solve the projection, returning a position per input variable (same
/// order as `vars`).
pub(super) fn solve(vars: &[Var], constraints: &[Constraint]) -> Vec<f64> {
    let n = vars.len();
    let mut blocks: Vec<Block> = vars
        .iter()
        .enumerate()
        .map(|(i, var)| {
            let mut tiers = [Tier::default(); Strength::COUNT];
            tiers[var.strength.tier()] = Tier {
                count: 1.0,
                sum: var.desired,
            };
            Block {
                tiers,
                members: vec![i],
            }
        })
        .collect();
    let mut block_of: Vec<usize> = (0..n).collect();
    // Offset of each variable from its block's reference position.
    let mut offset = vec![0.0_f64; n];

    let position = |blocks: &[Block], block_of: &[usize], offset: &[f64], v: usize| {
        blocks[block_of[v]].position() + offset[v]
    };

    loop {
        // Most-violated constraint whose endpoints are in different
        // blocks. Same-block constraints are already tight (or, if the
        // input is infeasible, left at best effort).
        let mut worst: Option<(usize, f64)> = None;
        for (ci, c) in constraints.iter().enumerate() {
            if block_of[c.left] == block_of[c.right] {
                continue;
            }
            let v = position(&blocks, &block_of, &offset, c.left) + c.gap
                - position(&blocks, &block_of, &offset, c.right);
            if v > TOL && worst.is_none_or(|(_, w)| v > w) {
                worst = Some((ci, v));
            }
        }

        let Some((ci, _)) = worst else { break };
        let c = &constraints[ci];
        let bl = block_of[c.left];
        let br = block_of[c.right];

        // Rebase the right block into the left block's frame, making the
        // constraint tight: x[right] = x[left] + gap.
        let shift = (offset[c.left] + c.gap) - offset[c.right];
        let moved = std::mem::take(&mut blocks[br].members);
        let right = std::mem::take(&mut blocks[br].tiers);
        for &m in &moved {
            offset[m] += shift;
            block_of[m] = bl;
        }
        blocks[bl].members.extend(moved);
        for (tier, r) in blocks[bl].tiers.iter_mut().zip(right) {
            tier.count += r.count;
            tier.sum += r.sum - shift * r.count;
        }
    }

    (0..n)
        .map(|i| position(&blocks, &block_of, &offset, i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(desired: f64) -> Var {
        Var {
            desired,
            strength: Strength::Pull,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn coincident_points_separate_symmetrically() {
        // Three points all wanting 0, each pair ≥ 10 apart.
        let vars = [var(0.0), var(0.0), var(0.0)];
        let cons = [
            Constraint {
                left: 0,
                right: 1,
                gap: 10.0,
            },
            Constraint {
                left: 1,
                right: 2,
                gap: 10.0,
            },
        ];
        let x = solve(&vars, &cons);
        assert!(
            close(x[0], -10.0) && close(x[1], 0.0) && close(x[2], 10.0),
            "{x:?}"
        );
    }

    #[test]
    fn satisfied_constraints_leave_desired_untouched() {
        let vars = [var(0.0), var(100.0)];
        let cons = [Constraint {
            left: 0,
            right: 1,
            gap: 10.0,
        }];
        let x = solve(&vars, &cons);
        assert!(close(x[0], 0.0) && close(x[1], 100.0), "{x:?}");
    }

    #[test]
    fn a_stronger_variable_holds_exactly_while_a_weaker_one_yields() {
        // Pin var 0; var 1 must sit 10 to its right.
        let vars = [
            Var {
                desired: 0.0,
                strength: Strength::Pin,
            },
            var(0.0),
        ];
        let cons = [Constraint {
            left: 0,
            right: 1,
            gap: 10.0,
        }];
        let x = solve(&vars, &cons);
        assert_eq!(x, [0.0, 10.0]);
    }

    #[test]
    fn a_segment_pushed_against_a_wall_lands_exactly_on_it() {
        // A segment that wants 380 and a neighbour that wants 360 must be
        // 40 apart, below a wall at 360: the wall holds, both yield to it.
        let vars = [
            var(380.0),
            var(360.0),
            Var {
                desired: 360.0,
                strength: Strength::Wall,
            },
        ];
        let cons = [
            Constraint {
                left: 1,
                right: 0,
                gap: 40.0,
            },
            Constraint {
                left: 0,
                right: 2,
                gap: 0.0,
            },
        ];
        let x = solve(&vars, &cons);
        assert_eq!(x, [360.0, 320.0, 360.0]);
    }

    #[test]
    fn a_wall_outweighs_a_pin() {
        let vars = [
            Var {
                desired: 20.0,
                strength: Strength::Pin,
            },
            Var {
                desired: 0.0,
                strength: Strength::Wall,
            },
        ];
        let cons = [Constraint {
            left: 0,
            right: 1,
            gap: 0.0,
        }];
        assert_eq!(solve(&vars, &cons), [0.0, 0.0]);
    }
}
