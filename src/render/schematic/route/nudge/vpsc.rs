//! One-dimensional separation-constraint projection — the `satisfy_VPSC`
//! step of paper §6.2.
//!
//! Given variables with desired positions and weights, and separation
//! constraints `x[right] − x[left] ≥ gap`, find positions that satisfy
//! every constraint while minimising `Σ wᵢ (xᵢ − dᵢ)²`.
//!
//! Method: the block-merge active-set algorithm of Dwyer, Marriott &
//! Stuckey, *Fast Node Overlap Removal* (LNCS 3843). Each variable
//! starts in its own block at its desired position. Repeatedly take the
//! most-violated constraint joining two different blocks and merge them,
//! making that constraint tight; a block's position is always the
//! weighted average that minimises its members' cost. Merge-only (no
//! split) matches the paper's approximate projection and is optimal for
//! the chain/tree constraints nudging produces.

/// A variable to place: pulled toward `desired` with strength `weight`.
pub(super) struct Var {
    pub(super) desired: f64,
    pub(super) weight: f64,
}

/// `position[right] − position[left] ≥ gap`.
pub(super) struct Constraint {
    pub(super) left: usize,
    pub(super) right: usize,
    pub(super) gap: f64,
}

const TOL: f64 = 1e-9;

struct Block {
    /// Σ weight over members.
    wsum: f64,
    /// Σ weight·(desired − offset) over members; block position = wpos/wsum.
    wpos: f64,
    members: Vec<usize>,
}

impl Block {
    fn position(&self) -> f64 {
        self.wpos / self.wsum
    }
}

/// Solve the projection, returning a position per input variable (same
/// order as `vars`).
pub(super) fn solve(vars: &[Var], constraints: &[Constraint]) -> Vec<f64> {
    let n = vars.len();
    let mut blocks: Vec<Block> = (0..n)
        .map(|i| Block {
            wsum: vars[i].weight,
            wpos: vars[i].weight * vars[i].desired,
            members: vec![i],
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
        let (r_wsum, r_wpos) = (blocks[br].wsum, blocks[br].wpos);
        for &m in &moved {
            offset[m] += shift;
            block_of[m] = bl;
        }
        blocks[bl].members.extend(moved);
        blocks[bl].wsum += r_wsum;
        blocks[bl].wpos += r_wpos - shift * r_wsum;
        blocks[br].wsum = 0.0;
        blocks[br].wpos = 0.0;
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
            weight: 1.0,
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
    fn heavy_variable_barely_moves() {
        // Pin var 0 with a large weight; var 1 must sit 10 to its right.
        let vars = [
            Var {
                desired: 0.0,
                weight: 1e6,
            },
            var(0.0),
        ];
        let cons = [Constraint {
            left: 0,
            right: 1,
            gap: 10.0,
        }];
        let x = solve(&vars, &cons);
        assert!(x[0].abs() < 1e-3, "pinned var moved: {}", x[0]);
        assert!(close(x[1] - x[0], 10.0), "{x:?}");
    }
}
