//! Merge groups: the `extend` machinery. A top-level component authored as
//! several fragments (one `component` root plus `extend`s) resolves as one
//! flat namespace; this module owns the group bookkeeping, seats each
//! group's canonical fragment, and reports orphans and cross-fragment
//! duplicates.

use std::collections::HashMap;

use crate::dsl::ast;
use crate::dsl::diagnostics::{OtherDeclaration, Problem, SpliceClash};
use crate::dsl::span::{FileId, Span};

use super::{ComponentScope, DefId, Resolver};

/// Sets of same-named definition fragments — one `component` root plus its
/// `extend` fragments — that elaborate as a single component sharing a flat
/// namespace. Each group keeps its canonical id (the lone non-`extend` root)
/// first. A definition in no group is a singleton: its own canonical.
#[derive(Default)]
pub struct MergeGroups {
    groups: Vec<Vec<DefId>>,
    of: HashMap<DefId, usize>,
}

impl MergeGroups {
    /// The representative id for `d`'s merged component (itself if unmerged).
    pub fn canonical(&self, d: DefId) -> DefId {
        match self.of.get(&d) {
            Some(&g) => self.groups[g][0],
            None => d,
        }
    }

    /// Every fragment id of `d`'s component, canonical first (just `[d]` when
    /// `d` belongs to no merge group).
    pub fn fragments(&self, d: DefId) -> Vec<DefId> {
        match self.of.get(&d) {
            Some(&g) => self.groups[g].clone(),
            None => vec![d],
        }
    }

    /// Union `a` and `b` into one group, creating or joining as needed.
    pub(super) fn link(&mut self, a: DefId, b: DefId) {
        match (self.of.get(&a).copied(), self.of.get(&b).copied()) {
            (None, None) => {
                let g = self.groups.len();
                self.groups.push(vec![a, b]);
                self.of.insert(a, g);
                self.of.insert(b, g);
            }
            (Some(g), None) => {
                self.groups[g].push(b);
                self.of.insert(b, g);
            }
            (None, Some(g)) => {
                self.groups[g].push(a);
                self.of.insert(a, g);
            }
            (Some(ga), Some(gb)) if ga != gb => {
                let moved = std::mem::take(&mut self.groups[gb]);
                for &d in &moved {
                    self.of.insert(d, ga);
                }
                self.groups[ga].extend(moved);
            }
            _ => {}
        }
    }
}

/// Move `id` to the front of `group`, preserving the relative order of the
/// rest. Used to seat a merge group's canonical fragment first.
fn move_first(group: &mut Vec<DefId>, id: DefId) {
    group.retain(|&d| d != id);
    group.insert(0, id);
}

impl<'a> Resolver<'a> {
    /// Settle every merge group's canonical (the lone root `component`) into
    /// first position, and report groups that lack a root (`OrphanFragment`)
    /// or carry two (`DuplicateType`).
    pub(super) fn finalize_groups(&mut self) {
        let mut groups = std::mem::take(&mut self.groups.groups);
        for group in &mut groups {
            if group.is_empty() {
                continue;
            }
            let roots: Vec<DefId> = group
                .iter()
                .copied()
                .filter(|&d| self.defs[d].kind == ast::DefKind::Component)
                .collect();
            match roots.as_slice() {
                // Rootless groups (pure `extend`s) are reported by
                // `report_orphan_fragments`; leave their order untouched.
                [] => {}
                [root] => move_first(group, *root),
                [first, rest @ ..] => {
                    for &dup in rest {
                        self.problems.push(Problem::DuplicateType {
                            name: self.defs[dup].name.to_string(),
                            src: self.project.source(self.defs[dup].file),
                            at: self.defs[dup].ast.name.span.into(),
                            first: self.defs[*first].ast.name.span.into(),
                        });
                    }
                    move_first(group, *first);
                }
            }
        }
        self.groups.groups = groups;
    }

    /// Report every top-level `extend` that never found a `component` to
    /// anchor it — a lone fragment, or a group of fragments with no root.
    /// One diagnostic per orphaned component (keyed on the group's canonical).
    pub(super) fn report_orphan_fragments(&mut self) {
        let mut orphans = Vec::new();
        for d in 0..self.defs.len() {
            if self.defs[d].kind != ast::DefKind::Extend || self.defs[d].parent.is_some() {
                continue;
            }
            let canon = self.groups.canonical(d);
            // A merged fragment canonicalizes onto its root `component`; if the
            // representative is still an `extend`, the component is missing.
            if d == canon && self.defs[canon].kind == ast::DefKind::Extend {
                orphans.push(d);
            }
        }
        for d in orphans {
            self.problems.push(Problem::OrphanFragment {
                name: self.defs[d].name.to_string(),
                src: self.project.source(self.defs[d].file),
                at: self.defs[d].ast.name.span.into(),
            });
        }
    }

    /// Flat-namespace guards: within a merged component, an instance or port
    /// name declared in two fragments is a duplicate, reported once.
    pub(super) fn validate_merge_groups(&mut self) {
        let groups = std::mem::take(&mut self.groups.groups);
        let mut problems = Vec::new();
        for group in &groups {
            if group.len() < 2 {
                continue;
            }
            let mut instances: HashMap<String, Span> = HashMap::new();
            let mut ports: HashMap<String, Span> = HashMap::new();
            for &f in group {
                let file = self.defs[f].file;
                // Instances and inlines share one namespace across fragments.
                let named = self.defs[f]
                    .instances
                    .iter()
                    .map(|(name, facts)| (name, facts.ast.name.span))
                    .chain(
                        self.defs[f]
                            .inlines
                            .iter()
                            .map(|(name, facts)| (name, facts.ast.name.span)),
                    );
                for (name, span) in named {
                    if let Some(first) = instances.get(name.as_str()).copied() {
                        problems.push(Problem::DuplicateInstance {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: span.into(),
                            first: first.into(),
                        });
                    } else {
                        instances.insert(name.to_string(), span);
                    }
                }
                for (name, facts) in &self.defs[f].ports {
                    if let Some(first) = ports.get(name.as_str()).copied() {
                        problems.push(Problem::DuplicatePort {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: facts.span.into(),
                            first: first.into(),
                        });
                    } else {
                        ports.insert(name.to_string(), facts.span);
                    }
                }
            }
        }
        self.groups.groups = groups;
        self.problems.extend(problems);
    }

    /// A splice's name is unique in its merged component: no port (bare
    /// endpoints name either), instance, inline connector, connector,
    /// cable or other splice of any fragment has it. One problem per
    /// splice that clashes, on the splice, naming the first clash.
    pub(super) fn check_splice_names(&mut self) {
        let mut problems = Vec::new();
        for d in 0..self.defs.len() {
            if self.groups.canonical(d) != d {
                continue;
            }
            let fragments = self.groups.fragments(d);
            let mut taken: Vec<(&str, SpliceClash, Span)> = Vec::new();
            for &f in &fragments {
                let info = &self.defs[f];
                taken.extend(
                    info.ports
                        .iter()
                        .map(|(name, facts)| (name.as_str(), SpliceClash::Port, facts.span)),
                );
                taken.extend(info.instances.iter().map(|(name, facts)| {
                    (name.as_str(), SpliceClash::Instance, facts.ast.name.span)
                }));
                taken.extend(info.inlines.iter().map(|(name, facts)| {
                    (
                        name.as_str(),
                        SpliceClash::InlineConnector,
                        facts.ast.name.span,
                    )
                }));
                for member in &info.ast.members {
                    let (name, what) = match member {
                        ast::Member::Connector(c) => (&c.name, SpliceClash::Connector),
                        ast::Member::ConnectorInstance(c) => (&c.name, SpliceClash::Connector),
                        ast::Member::Cable(c) => (&c.name, SpliceClash::Cable),
                        _ => continue,
                    };
                    taken.push((name.node.as_str(), what, name.span));
                }
            }
            for &f in &fragments {
                for splice in self.defs[f].ast.splices() {
                    let name = splice.name.node.as_str();
                    let at = splice.name.span;
                    if let Some(&(_, other, first)) = taken.iter().find(|(n, ..)| *n == name) {
                        let file = self.defs[f].file;
                        let same_file = first.file == file;
                        problems.push(Problem::DuplicateSpliceName {
                            name: name.to_string(),
                            other,
                            component: self.defs[d].name.to_string(),
                            src: self.project.source(file),
                            at: at.into(),
                            here: same_file.then(|| first.into()),
                            elsewhere: (!same_file)
                                .then(|| OtherDeclaration {
                                    what: other,
                                    name: name.to_string(),
                                    src: self.project.source(first.file),
                                    at: first.into(),
                                })
                                .into_iter()
                                .collect(),
                        });
                    }
                    taken.push((name, SpliceClash::Splice, at));
                }
            }
        }
        self.problems.extend(problems);
    }

    /// Rewrite every type-scope entry to its group's canonical id, so an
    /// instance of a merged type resolves to the one representative def.
    pub(super) fn canonicalize_scopes(&self, scopes: &mut HashMap<FileId, ComponentScope>) {
        for scope in scopes.values_mut() {
            for id in scope.values_mut() {
                *id = self.groups.canonical(*id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dsl::resolve::testkit::{LEAF, codes, has, problems, with_resolved};

    #[test]
    fn extend_fragment_merges_into_the_root_component() {
        with_resolved(
            &[
                (
                    "main.wb",
                    "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                     component vehicle { pack: leaf \"Pack\"; }\n",
                ),
                (
                    "traction.wb",
                    "use leaf from \"leaf.wb\";\n\
                     extend vehicle { inv: leaf \"Inv\"; wire red 1 [pack.a, inv.a]; }\n",
                ),
                ("leaf.wb", LEAF),
            ],
            |r| {
                assert!(r.problems.is_empty(), "problems: {:?}", codes(&r.problems));
                let root = r.root.expect("vehicle is the root");
                // The merged component owns instances from both fragments,
                // flat in one namespace.
                let names: Vec<&str> = r
                    .fragments(root)
                    .into_iter()
                    .flat_map(|f| r.defs[f].instances.keys())
                    .map(|n| n.as_str())
                    .collect();
                assert!(names.contains(&"pack"), "{names:?}");
                assert!(names.contains(&"inv"), "{names:?}");
            },
        );
    }

    #[test]
    fn extend_without_a_root_component_is_orphan() {
        // `main` is its own root (`shell`); the imported `extend vehicle` has
        // no `component vehicle` anywhere.
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"frag.wb\";\ncomponent shell { }\n",
            ),
            ("frag.wb", "extend vehicle { }\n"),
        ]);
        assert!(has(&p, "wirebug::orphan_fragment"), "{:?}", codes(&p));
    }

    /// The code and the message of each problem of a one-file component
    /// `m` with `members`.
    fn clashes(members: &str) -> Vec<String> {
        problems(&[(
            "main.wb",
            &format!("component leaf {{ pub port a \"A\"; }}\ncomponent m {{ {members} }}\n"),
        )])
        .iter()
        .map(ToString::to_string)
        .collect()
    }

    #[test]
    fn a_splice_takes_a_name_nothing_else_in_its_component_has() {
        for (members, other) in [
            ("pub port sp \"SP\"; splice sp;", "a port"),
            ("splice sp; port sp \"SP\";", "a port"),
            ("sp: leaf; splice sp;", "an instance"),
            (
                "inline sp { port a \"A\" pin 1; } splice sp;",
                "an inline connector",
            ),
            (
                "connector sp { port a \"A\" pin 1; } splice sp;",
                "a connector",
            ),
            ("cable sp { } splice sp;", "a cable"),
            ("splice sp; splice sp;", "another splice"),
        ] {
            let found = clashes(members);
            assert!(
                found.contains(&format!("splice `sp` has the same name as {other} of `m`")),
                "{members}: {found:?}"
            );
        }
        assert!(clashes("splice sp; splice sq;").is_empty());
    }

    #[test]
    fn a_splice_clashing_with_a_port_of_another_fragment_points_at_both() {
        use miette::Diagnostic;
        let p = problems(&[
            (
                "main.wb",
                "fragment \"loom.wb\";\ncomponent m { pub port gnd \"GND\"; }\n",
            ),
            ("loom.wb", "extend m { splice gnd; }\n"),
        ]);
        let [problem] = p.as_slice() else {
            panic!("one problem: {:?}", codes(&p));
        };
        assert!(has(&p, "wirebug::duplicate_splice_name"));
        // One label, on the splice; the port is a related note in its file.
        assert_eq!(problem.labels().into_iter().flatten().count(), 1);
        let related: Vec<String> = problem
            .related()
            .into_iter()
            .flatten()
            .map(ToString::to_string)
            .collect();
        assert_eq!(related, ["a port `gnd` is declared here"]);
    }

    #[test]
    fn instance_declared_in_two_fragments_is_duplicate() {
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { dup: leaf \"D\"; }\n",
            ),
            (
                "traction.wb",
                "use leaf from \"leaf.wb\";\nextend vehicle { dup: leaf \"D2\"; }\n",
            ),
            ("leaf.wb", LEAF),
        ]);
        assert!(has(&p, "wirebug::duplicate_instance"), "{:?}", codes(&p));
    }
}
