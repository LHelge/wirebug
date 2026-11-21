//! Per-file name scopes: each file sees its own top-level definitions and
//! connector types plus whatever its `use` declarations import. A same-name
//! component collision is a merge (when either side is an `extend`) — that
//! discovery happens here, feeding the merge machinery in [`super::merge`].
//! A `fragment` imports no name: it merges the target file's `extend`s with
//! the components of their names already in scope. Once every file's scope
//! is built, each line is judged for whether it adds anything.

use std::collections::{HashMap, HashSet};

use crate::dsl::ast;
use crate::dsl::diagnostics::{Problem, UnusedFragmentReason};
use crate::dsl::project::Project;
use crate::dsl::span::{FileId, Span};

use super::{ComponentScope, ConnectorTypeId, ConnectorTypeScope, DefId, Resolver};

impl<'a> Resolver<'a> {
    /// Build each file's type-name scope: its own top-level definitions
    /// plus the names brought in by `use`; then merge in the `extend`s of
    /// the files its `fragment`s pull in. Last, report the `fragment` lines
    /// that repeat a line or add nothing.
    pub(super) fn build_scopes(
        &mut self,
        roots_by_file: &[Vec<DefId>],
        connector_types_by_file: &[Vec<ConnectorTypeId>],
    ) -> (
        HashMap<FileId, ComponentScope>,
        HashMap<FileId, ConnectorTypeScope>,
    ) {
        let mut scopes: HashMap<FileId, HashMap<String, DefId>> = HashMap::new();
        let mut connector_type_scopes: HashMap<FileId, HashMap<String, ConnectorTypeId>> =
            HashMap::new();
        let mut graph = FragmentGraph::new(self.project);
        let mut lines: Vec<(FileId, FragmentLine<'a>)> = Vec::new();
        for (fi, file) in self.project.files.iter().enumerate() {
            let fid = FileId(fi);
            let mut scope: HashMap<String, DefId> = HashMap::new();
            let mut connector_type_scope: HashMap<String, ConnectorTypeId> = HashMap::new();

            for &id in &roots_by_file[fi] {
                if let Some((into, extra)) = self.insert_type(
                    &mut scope,
                    self.defs[id].name,
                    id,
                    fid,
                    self.defs[id].ast.name.span,
                ) {
                    self.groups.link(into, extra);
                }
            }
            for &id in &connector_types_by_file[fi] {
                self.insert_connector_type(
                    &mut connector_type_scope,
                    self.connector_types[id].name,
                    id,
                    fid,
                    self.connector_types[id].ast.name.span,
                );
            }

            // The merges the file's `use` and `fragment` lines make, each at
            // its line's start: made once the scope is complete, in source
            // order across both spellings, so a group gathers its fragments
            // in the order the file names them.
            let mut merges: Vec<(usize, DefId, DefId)> = Vec::new();
            for use_decl in &file.ast.uses {
                // A path that names no loaded file was reported by the loader
                // (not found), or its file failed to load (its own errors).
                let Some(tfid) = self.project.import_target(fid, &use_decl.path.node) else {
                    continue;
                };
                let wanted = use_decl.name.node.as_str();
                let component = roots_by_file[tfid.0]
                    .iter()
                    .copied()
                    .find(|&id| self.defs[id].name == wanted);
                let connector_type = connector_types_by_file[tfid.0]
                    .iter()
                    .copied()
                    .find(|&id| self.connector_types[id].name == wanted);
                match (component, connector_type) {
                    (Some(id), _) => {
                        if let Some((into, extra)) =
                            self.insert_type(&mut scope, wanted, id, fid, use_decl.name.span)
                        {
                            merges.push((use_decl.span.start, into, extra));
                        }
                    }
                    (None, Some(id)) => self.insert_connector_type(
                        &mut connector_type_scope,
                        wanted,
                        id,
                        fid,
                        use_decl.name.span,
                    ),
                    (None, None) => self.problems.push(Problem::UnresolvedImport {
                        name: wanted.to_string(),
                        file: use_decl.path.node.clone(),
                        src: self.project.source(fid),
                        at: use_decl.name.span.into(),
                    }),
                }
            }

            // With the scope complete, whatever order the lines come in, each
            // `fragment` merges into the components it holds.
            let mut fragmented: HashMap<FileId, Span> = HashMap::new();
            for fragment in &file.ast.fragments {
                let Some(tfid) = self.project.import_target(fid, &fragment.path.node) else {
                    continue;
                };
                graph.targets[fi].push(tfid);
                let fate = match fragmented.get(&tfid) {
                    Some(&first) => FragmentFate::Repeats(first),
                    None => {
                        fragmented.insert(tfid, fragment.path.span);
                        FragmentFate::Merges(self.merge_fragment(
                            fragment,
                            &roots_by_file[tfid.0],
                            &scope,
                            &mut merges,
                        ))
                    }
                };
                lines.push((
                    fid,
                    FragmentLine {
                        fragment,
                        target: tfid,
                        fate,
                    },
                ));
            }
            graph.merges_in_scope[fi] = !merges.is_empty();
            merges.sort_by_key(|&(at, ..)| at);
            for (_, into, extra) in merges {
                self.groups.link(into, extra);
            }

            scopes.insert(fid, scope);
            connector_type_scopes.insert(fid, connector_type_scope);
        }
        for (file, line) in lines {
            self.report_fragment_line(file, line, &graph);
        }
        (scopes, connector_type_scopes)
    }

    /// Record in `merges` (at the line's start) a merge of each top-level
    /// `extend` among `target_roots` (the top-level definitions of the file
    /// `fragment` names) into the component of its name in `scope` (the
    /// scope of the file the line is written in), and return what merged.
    /// No name enters the scope.
    fn merge_fragment(
        &self,
        fragment: &ast::Fragment,
        target_roots: &[DefId],
        scope: &ComponentScope,
        merges: &mut Vec<(usize, DefId, DefId)>,
    ) -> FragmentMerge {
        let mut merge = FragmentMerge::default();
        for &id in target_roots {
            if self.defs[id].kind != ast::DefKind::Extend {
                continue;
            }
            let name = self.defs[id].name;
            let unmerged = match scope.get(name) {
                Some(&into) if into != id => {
                    merges.push((fragment.span.start, into, id));
                    merge.merged = true;
                    continue;
                }
                // The scope entry is this very `extend`: the line names its
                // own file, or a `use` of this file imported it.
                Some(_) => &mut merge.imported,
                None => &mut merge.not_in_scope,
            };
            if !unmerged.iter().any(|n| n == name) {
                unmerged.push(name.to_string());
            }
        }
        merge
    }

    /// Report `line`, written in `file`, if it repeats an earlier line of
    /// the file (`duplicate_fragment`) or adds nothing (`unused_fragment`,
    /// with the reason).
    fn report_fragment_line(
        &mut self,
        file: FileId,
        FragmentLine {
            fragment,
            target,
            fate,
        }: FragmentLine<'a>,
        graph: &FragmentGraph<'a>,
    ) {
        let src = self.project.source(file);
        let at = fragment.path.span.into();
        let merge = match fate {
            FragmentFate::Repeats(first) => {
                self.problems.push(Problem::DuplicateFragment {
                    target: fragment.path.node.clone(),
                    src,
                    at,
                    first: first.into(),
                });
                return;
            }
            FragmentFate::Merges(merge) => merge,
        };
        let target_ast = &self.project.file(target).ast;
        let reason = if target == file {
            UnusedFragmentReason::ItsOwnFile
        } else if merge.merged || graph.contributes(target, file) {
            return;
        } else if target == self.project.root {
            UnusedFragmentReason::TheEntryFile
        } else if !merge.not_in_scope.is_empty() {
            UnusedFragmentReason::NotInScope(merge.not_in_scope)
        } else if !merge.imported.is_empty() {
            UnusedFragmentReason::Imported(merge.imported)
        } else if !target_ast.fragments.is_empty() {
            UnusedFragmentReason::ItsFragmentsAddNothing
        } else {
            UnusedFragmentReason::Empty
        };
        self.problems.push(Problem::UnusedFragment {
            target: fragment.path.node.clone(),
            reason,
            src,
            at,
        });
    }

    /// Put `id` in `scope` under `name`. A same-named entry already there
    /// is a merge when either is an `extend`, returned for the caller to
    /// make (`(entry, id)`), and otherwise a duplicate, reported.
    #[must_use]
    fn insert_type(
        &mut self,
        scope: &mut HashMap<String, DefId>,
        name: &str,
        id: DefId,
        file: FileId,
        at: Span,
    ) -> Option<(DefId, DefId)> {
        if let Some(&first) = scope.get(name) {
            // A same-named collision is a merge — not an error — as soon as
            // either side is an `extend` fragment. Two plain `component`s stay
            // a genuine duplicate. The group's canonical (its lone root
            // `component`) is settled in `finalize_groups`; keep a root in the
            // scope slot when we can so the entry already points at it.
            let first_extend = self.defs[first].kind == ast::DefKind::Extend;
            let new_extend = self.defs[id].kind == ast::DefKind::Extend;
            if first_extend || new_extend {
                if first_extend && !new_extend {
                    scope.insert(name.to_string(), id);
                }
                return Some((first, id));
            }
            self.problems.push(Problem::DuplicateType {
                name: name.to_string(),
                src: self.project.source(file),
                at: at.into(),
                first: self.defs[first].ast.name.span.into(),
            });
        } else {
            scope.insert(name.to_string(), id);
        }
        None
    }

    fn insert_connector_type(
        &mut self,
        scope: &mut HashMap<String, ConnectorTypeId>,
        name: &str,
        id: ConnectorTypeId,
        file: FileId,
        at: Span,
    ) {
        if let Some(&first) = scope.get(name) {
            self.problems.push(Problem::DuplicateConnectorType {
                name: name.to_string(),
                src: self.project.source(file),
                at: at.into(),
                first: self.connector_types[first].ast.name.span.into(),
            });
        } else {
            scope.insert(name.to_string(), id);
        }
    }

    /// Types instantiable inside definition `d`: the file scope overlaid
    /// with the nested definitions of `d` and its ancestors (inner wins).
    pub(super) fn visible_types(
        &self,
        d: DefId,
        file_scope: &HashMap<FileId, HashMap<String, DefId>>,
    ) -> HashMap<String, DefId> {
        let mut env = file_scope[&self.defs[d].file].clone();
        let mut chain = Vec::new();
        let mut cur = Some(d);
        while let Some(c) = cur {
            chain.push(c);
            cur = self.defs[c].parent;
        }
        for c in chain.into_iter().rev() {
            for &n in &self.defs[c].nested {
                env.insert(self.defs[n].name.to_string(), n);
            }
        }
        env
    }
}

/// A `fragment` line, as resolved in the file it is written in.
struct FragmentLine<'a> {
    fragment: &'a ast::Fragment,
    /// The file the line names.
    target: FileId,
    fate: FragmentFate,
}

/// What became of a `fragment` line.
enum FragmentFate {
    /// It names the file an earlier line of the same file names, whose
    /// path is at this span.
    Repeats(Span),
    /// It is the first line of its file to name its target, and merged
    /// this.
    Merges(FragmentMerge),
}

/// What a `fragment` line made of its target's top-level `extend`s.
#[derive(Default)]
struct FragmentMerge {
    /// Whether it merged at least one.
    merged: bool,
    /// The components extended there that are not in the naming file's
    /// scope, in the target's order.
    not_in_scope: Vec<String>,
    /// The components extended there whose scope entry in the naming file
    /// is that same `extend`, in the target's order.
    imported: Vec<String>,
}

/// The files the `fragment` lines name, and the files that contribute to
/// the design on their own, for judging whether a line adds anything.
struct FragmentGraph<'a> {
    project: &'a Project,
    /// By file: the files its `fragment` lines name.
    targets: Vec<Vec<FileId>>,
    /// By file: whether its scope made a merge (from a `use` collision or
    /// a `fragment` line).
    merges_in_scope: Vec<bool>,
}

impl<'a> FragmentGraph<'a> {
    fn new(project: &'a Project) -> Self {
        Self {
            project,
            targets: vec![Vec::new(); project.files.len()],
            merges_in_scope: vec![false; project.files.len()],
        }
    }

    /// Whether `target`, named by a `fragment` line of `from`, contributes
    /// something: a view, a merge its own scope makes, or a `fragment` line
    /// naming a file that contributes. The files already loaded on the way
    /// (the entry file, `from`, and those followed to get here) contribute
    /// nothing again, so a cycle adds nothing of itself.
    fn contributes(&self, target: FileId, from: FileId) -> bool {
        let mut seen: HashSet<FileId> = HashSet::from([self.project.root, from]);
        let mut todo = vec![target];
        while let Some(file) = todo.pop() {
            if !seen.insert(file) {
                continue;
            }
            if self.merges_in_scope[file.0] || self.project.file(file).ast.has_views() {
                return true;
            }
            todo.extend(&self.targets[file.0]);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use miette::Diagnostic;

    use crate::dsl::ast;
    use crate::dsl::diagnostics::Problem;
    use crate::dsl::resolve::testkit::{LEAF, codes, has, problems, with_resolved};

    #[test]
    fn unresolved_import_is_reported() {
        let p = problems(&[
            (
                "main.wb",
                "use missing from \"leaf.wb\";\ncomponent m { x: missing; }\n",
            ),
            ("leaf.wb", "component other { pub port a \"A\"; }\n"),
        ]);
        assert!(has(&p, "wirebug::unresolved_import"), "{:?}", codes(&p));
    }

    #[test]
    fn fragment_merges_the_files_extend_into_the_component_in_scope() {
        with_resolved(
            &[
                (
                    "main.wb",
                    "fragment \"systems/traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                     component vehicle { pack: leaf \"Pack\"; }\n",
                ),
                (
                    "systems/traction.wb",
                    "use leaf from \"../leaf.wb\";\n\
                     extend vehicle { inv: leaf \"Inv\"; wire red 1 [pack.a, inv.a]; }\n",
                ),
                ("leaf.wb", LEAF),
            ],
            |r| {
                assert!(r.problems.is_empty(), "problems: {:?}", codes(&r.problems));
                let root = r.root.expect("vehicle is the root");
                assert_eq!(r.fragments(root).len(), 2, "the extend merged");
            },
        );
    }

    #[test]
    fn fragment_merges_into_a_component_a_later_use_brings_in() {
        // The scope is complete before any `fragment` merges: the line order
        // does not matter.
        with_resolved(
            &[
                (
                    "main.wb",
                    "fragment \"a.wb\";\nuse vehicle from \"vehicle.wb\";\n\
                     component shell { v: vehicle; }\n",
                ),
                ("vehicle.wb", "component vehicle { }\n"),
                ("a.wb", "extend vehicle { pub port x \"X\"; }\n"),
            ],
            |r| {
                assert!(r.problems.is_empty(), "problems: {:?}", codes(&r.problems));
                let vehicle = r
                    .defs
                    .iter()
                    .position(|d| d.name == "vehicle" && d.kind == ast::DefKind::Component)
                    .expect("vehicle");
                assert_eq!(r.fragments(vehicle).len(), 2, "the extend merged");
            },
        );
    }

    #[test]
    fn use_and_fragment_lines_merge_in_source_order() {
        // A file migrating line by line mixes the two spellings: the group
        // still gathers the fragments in the order the lines name them.
        with_resolved(
            &[
                (
                    "main.wb",
                    "fragment \"a.wb\";\nuse vehicle from \"b.wb\";\nfragment \"c.wb\";\n\
                     component vehicle { }\n",
                ),
                ("a.wb", "extend vehicle { pub port x \"X\"; }\n"),
                ("b.wb", "extend vehicle { pub port x \"X\"; }\n"),
                ("c.wb", "extend vehicle { pub port c \"C\"; }\n"),
            ],
            |r| {
                let files: Vec<String> = r
                    .fragments(r.root.expect("root"))
                    .into_iter()
                    .map(|d| {
                        let path = &r.project.file(r.defs[d].file).path;
                        path.file_name().unwrap().to_string_lossy().into_owned()
                    })
                    .collect();
                assert_eq!(files, ["main.wb", "a.wb", "b.wb", "c.wb"]);
                // So the port declared twice is reported where it is
                // declared second.
                let reused: Vec<String> = r
                    .problems
                    .iter()
                    .filter_map(|p| match p {
                        Problem::DuplicatePort { src, .. } => Some(src.name().to_string()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(reused.len(), 1, "{:?}", codes(&r.problems));
                assert!(reused[0].ends_with("b.wb"), "{reused:?}");
            },
        );
    }

    #[test]
    fn fragment_imports_no_name() {
        // `lamp.wb` is loaded, but its component is not in main.wb's scope.
        let p = problems(&[
            (
                "main.wb",
                "fragment \"lamp.wb\";\ncomponent m { l: lamp; }\n\
                 view schematic \"M\" { }\n",
            ),
            (
                "lamp.wb",
                "component lamp { }\nview schematic \"Lamp\" for lamp { }\n",
            ),
        ]);
        assert_eq!(codes(&p), ["wirebug::undefined_type"]);
    }

    #[test]
    fn fragment_of_a_missing_file_is_not_found_on_the_path() {
        let src = "fragment \"systems/nope.wb\";\ncomponent m { }\n";
        let p = problems(&[("main.wb", src)]);
        assert_eq!(codes(&p), ["wirebug::use_not_found"]);
        let at = p[0].labels().expect("a label").next().expect("one label");
        assert_eq!(
            &src[at.offset()..at.offset() + at.len()],
            "\"systems/nope.wb\""
        );
    }

    /// The help of the one `unused_fragment` among `problems`.
    fn unused_fragment_help(problems: &[Problem]) -> String {
        let [(_, help)] = &unused_fragment_helps(problems)[..] else {
            panic!("expected one unused_fragment: {:?}", codes(problems));
        };
        help.clone()
    }

    /// Each `unused_fragment` among `problems`: the name of the file its
    /// line is written in, and its help.
    fn unused_fragment_helps(problems: &[Problem]) -> Vec<(String, String)> {
        problems
            .iter()
            .filter_map(|p| match p {
                Problem::UnusedFragment { src, .. } => {
                    assert!(!p.is_error(), "a warning");
                    let file = Path::new(src.name()).file_name().expect("a file name");
                    Some((
                        file.to_string_lossy().into_owned(),
                        p.help().expect("a help").to_string(),
                    ))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fragment_of_a_file_that_adds_nothing_warns() {
        let p = problems(&[
            ("main.wb", "fragment \"lamp.wb\";\ncomponent m { }\n"),
            ("lamp.wb", "component lamp { }\n"),
        ]);
        assert_eq!(codes(&p), ["wirebug::unused_fragment"]);
        assert_eq!(
            unused_fragment_help(&p),
            "`lamp.wb` has no view and no `extend`, so pulling it in adds nothing; remove the line, or `use` a component it defines by name"
        );
    }

    #[test]
    fn fragment_whose_extends_have_no_component_in_scope_names_them() {
        // `vehicle` and `trailer` are extended, but neither is in the
        // aggregating file's scope: merging does not pass through main.wb.
        // So all.wb adds nothing, and main.wb's line is reported too.
        let p = problems(&[
            (
                "main.wb",
                "fragment \"all.wb\";\ncomponent vehicle { }\ncomponent trailer { }\n",
            ),
            ("all.wb", "fragment \"a.wb\";\n"),
            (
                "a.wb",
                "extend vehicle { }\nextend trailer { }\nextend vehicle { }\n",
            ),
        ]);
        assert_eq!(
            unused_fragment_helps(&p),
            [
                (
                    "main.wb".to_string(),
                    "`all.wb` has no view and no `extend`, and its own `fragment` lines add nothing either: each names a file already loaded on the way here, or one that adds nothing through it; remove the line".to_string()
                ),
                (
                    "all.wb".to_string(),
                    "`a.wb` extends `vehicle` and `trailer`, which are not in scope here, and a `fragment` merges an `extend` only into a component in scope where the line is written: `use` them in this file, or write the line in a file that has them in scope, such as the one that defines them".to_string()
                ),
            ]
        );
    }

    #[test]
    fn fragment_whose_extend_has_no_component_in_scope_names_it() {
        let p = problems(&[
            ("main.wb", "fragment \"a.wb\";\ncomponent m { }\n"),
            ("a.wb", "extend other { }\n"),
        ]);
        assert_eq!(
            unused_fragment_help(&p),
            "`a.wb` extends `other`, which is not in scope here, and a `fragment` merges an `extend` only into a component in scope where the line is written: `use` it in this file, or write the line in a file that has it in scope, such as the one that defines it"
        );
    }

    #[test]
    fn fragment_of_its_own_file_warns() {
        let p = problems(&[(
            "main.wb",
            "fragment \"main.wb\";\ncomponent m { }\nview schematic \"M\" { }\n",
        )]);
        assert_eq!(
            unused_fragment_help(&p),
            "`main.wb` is the file this line is written in, which is already loaded; remove the line"
        );
    }

    #[test]
    fn aggregating_file_that_uses_the_component_merges_its_fragments() {
        // all.wb has `vehicle` in scope through its `use`, so its fragment
        // merges; main.wb's line is not judged for all.wb's own lines.
        with_resolved(
            &[
                ("main.wb", "fragment \"all.wb\";\ncomponent vehicle { }\n"),
                (
                    "all.wb",
                    "use vehicle from \"main.wb\";\nfragment \"a.wb\";\n",
                ),
                ("a.wb", "extend vehicle { pub port x \"X\"; }\n"),
            ],
            |r| {
                assert!(r.problems.is_empty(), "problems: {:?}", codes(&r.problems));
                assert_eq!(r.fragments(r.root.expect("root")).len(), 2);
            },
        );
    }

    #[test]
    fn fragment_of_a_file_of_views_is_used() {
        let p = problems(&[
            ("main.wb", "fragment \"docs.wb\";\ncomponent m { }\n"),
            (
                "docs.wb",
                "use m from \"main.wb\";\nview schematic \"M\" for m { }\n",
            ),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn second_fragment_of_one_file_warns_once() {
        let src = "fragment \"a.wb\";\nfragment \"./a.wb\";\ncomponent m { }\n";
        let p = problems(&[("main.wb", src), ("a.wb", "extend m { }\n")]);
        assert_eq!(codes(&p), ["wirebug::duplicate_fragment"]);
        assert!(!p[0].is_error(), "a warning");
        let labels: Vec<&str> = p[0]
            .labels()
            .expect("labels")
            .map(|l| &src[l.offset()..l.offset() + l.len()])
            .collect();
        assert_eq!(labels, ["\"./a.wb\"", "\"a.wb\""]);
    }

    #[test]
    fn fragment_and_use_of_one_file_is_fine() {
        let p = problems(&[
            (
                "main.wb",
                "fragment \"a.wb\";\nuse lamp from \"a.wb\";\ncomponent m { l: lamp; }\n",
            ),
            ("a.wb", "component lamp { }\nextend m { }\n"),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn fragment_cycle_loads_each_file_once() {
        // main.wb's line merges a.wb's `extend`; a.wb's line back names the
        // entry file, loaded anyway, and merges nothing of it.
        with_resolved(
            &[
                (
                    "main.wb",
                    "fragment \"a.wb\";\ncomponent m { }\nview schematic \"M\" { }\n",
                ),
                (
                    "a.wb",
                    "fragment \"main.wb\";\nextend m { pub port x \"X\"; }\n",
                ),
            ],
            |r| {
                assert_eq!(r.fragments(r.root.expect("root")).len(), 2);
                assert_eq!(
                    unused_fragment_helps(&r.problems),
                    [(
                        "a.wb".to_string(),
                        "`main.wb` is the project's entry file, which is loaded whatever names it, and the line merges none of its `extend`s; remove the line".to_string()
                    )]
                );
            },
        );
    }

    #[test]
    fn fragments_that_only_name_each_other_are_both_reported() {
        let p = problems(&[
            (
                "main.wb",
                "fragment \"systems/all.wb\";\ncomponent m { }\nview schematic \"M\" { }\n",
            ),
            ("systems/all.wb", "fragment \"../main.wb\";\n"),
        ]);
        assert_eq!(
            unused_fragment_helps(&p),
            [
                (
                    "main.wb".to_string(),
                    "`systems/all.wb` has no view and no `extend`, and its own `fragment` lines add nothing either: each names a file already loaded on the way here, or one that adds nothing through it; remove the line".to_string()
                ),
                (
                    "all.wb".to_string(),
                    "`../main.wb` is the project's entry file, which is loaded whatever names it, and the line merges none of its `extend`s; remove the line".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_file_reached_again_through_a_cycle_contributes_nothing() {
        // b.wb's only line leads back to a.wb, which main.wb already loads:
        // a.wb's line to b.wb adds nothing. b.wb's line names a file of
        // views, which is enough.
        let p = problems(&[
            ("main.wb", "fragment \"a.wb\";\ncomponent m { }\n"),
            ("a.wb", "fragment \"b.wb\";\nview schematic \"A\" { }\n"),
            ("b.wb", "fragment \"a.wb\";\n"),
        ]);
        let helps = unused_fragment_helps(&p);
        let files: Vec<&str> = helps.iter().map(|(file, _)| file.as_str()).collect();
        assert_eq!(files, ["a.wb"], "{helps:?}");
    }

    #[test]
    fn fragment_of_an_extend_this_file_imports_says_so() {
        // all.wb's `use` imports the very `extend` its `fragment` names, and
        // no component of the name: there is nothing to merge it into.
        let p = problems(&[
            ("main.wb", "fragment \"systems/all.wb\";\ncomponent m { }\n"),
            (
                "systems/all.wb",
                "use Vehicle from \"a.wb\";\nfragment \"a.wb\";\n",
            ),
            ("systems/a.wb", "extend Vehicle { pub port x \"X\"; }\n"),
        ]);
        assert!(has(&p, "wirebug::orphan_fragment"), "{:?}", codes(&p));
        let helps = unused_fragment_helps(&p);
        let help = helps
            .iter()
            .find(|(file, _)| file == "all.wb")
            .map(|(_, help)| help.as_str());
        assert_eq!(
            help,
            Some(
                "`a.wb` extends `Vehicle`, but the `Vehicle` in scope here is that same `extend`, imported with `use`, so there is no component to merge it into; import the component itself, from the file that defines it"
            ),
            "{helps:?}"
        );
    }

    #[test]
    fn fragment_of_a_file_whose_use_merges_is_used() {
        // all.wb merges a.wb's `extend` through the older `use` spelling.
        with_resolved(
            &[
                ("main.wb", "fragment \"all.wb\";\ncomponent vehicle { }\n"),
                (
                    "all.wb",
                    "use vehicle from \"main.wb\";\nuse vehicle from \"a.wb\";\n",
                ),
                ("a.wb", "extend vehicle { pub port x \"X\"; }\n"),
            ],
            |r| {
                assert!(r.problems.is_empty(), "problems: {:?}", codes(&r.problems));
                assert_eq!(r.fragments(r.root.expect("root")).len(), 2);
            },
        );
    }

    #[test]
    fn use_as_a_merge_trigger_still_merges() {
        with_resolved(
            &[
                ("main.wb", "use m from \"a.wb\";\ncomponent m { }\n"),
                ("a.wb", "extend m { pub port x \"X\"; }\n"),
            ],
            |r| {
                assert!(r.problems.is_empty(), "problems: {:?}", codes(&r.problems));
                assert_eq!(r.fragments(r.root.expect("root")).len(), 2);
            },
        );
    }

    #[test]
    fn two_plain_components_with_one_name_still_duplicate() {
        let p = problems(&[(
            "main.wb",
            "component dup { }\ncomponent dup { }\ncomponent root { d: dup; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_type"), "{:?}", codes(&p));
    }
}
