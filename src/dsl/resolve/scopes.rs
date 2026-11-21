//! Per-file name scopes: each file sees its own top-level definitions and
//! connector types plus whatever its `use` declarations import. A same-name
//! component collision is a merge (when either side is an `extend`) — that
//! discovery happens here, feeding the merge machinery in [`super::merge`].
//! A `fragment` imports no name: it merges the target file's `extend`s with
//! the components of their names already in scope.

use std::collections::HashMap;

use crate::dsl::ast;
use crate::dsl::diagnostics::{Problem, UnusedFragmentReason};
use crate::dsl::span::{FileId, Span};

use super::{ComponentScope, ConnectorTypeId, ConnectorTypeScope, DefId, Resolver};

impl<'a> Resolver<'a> {
    /// Build each file's type-name scope: its own top-level definitions
    /// plus the names brought in by `use`; then merge in the `extend`s of
    /// the files its `fragment`s pull in.
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
        for (fi, file) in self.project.files.iter().enumerate() {
            let fid = FileId(fi);
            let mut scope: HashMap<String, DefId> = HashMap::new();
            let mut connector_type_scope: HashMap<String, ConnectorTypeId> = HashMap::new();

            for &id in &roots_by_file[fi] {
                self.insert_type(
                    &mut scope,
                    self.defs[id].name,
                    id,
                    fid,
                    self.defs[id].ast.name.span,
                );
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
                        self.insert_type(&mut scope, wanted, id, fid, use_decl.name.span)
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
                if let Some(&first) = fragmented.get(&tfid) {
                    self.problems.push(Problem::DuplicateFragment {
                        target: fragment.path.node.clone(),
                        src: self.project.source(fid),
                        at: fragment.path.span.into(),
                        first: first.into(),
                    });
                    continue;
                }
                fragmented.insert(tfid, fragment.path.span);
                self.merge_fragment(fragment, tfid, fid, &scope, &roots_by_file[tfid.0]);
            }

            scopes.insert(fid, scope);
            connector_type_scopes.insert(fid, connector_type_scope);
        }
        (scopes, connector_type_scopes)
    }

    /// Merge each top-level `extend` of file `target` (whose top-level
    /// definitions are `target_roots`) into the component of its name in
    /// `scope`: the scope of `file`, whose `fragment` names `target`. No
    /// name enters the scope. A line naming its own file, or one that
    /// merges nothing, naming a file with no view and no `fragment` of its
    /// own (whose lines are judged in that file), is reported unused, with
    /// the reason.
    fn merge_fragment(
        &mut self,
        fragment: &ast::Fragment,
        target: FileId,
        file: FileId,
        scope: &ComponentScope,
        target_roots: &[DefId],
    ) {
        let mut merged = false;
        let mut not_in_scope: Vec<String> = Vec::new();
        for &id in target_roots {
            if self.defs[id].kind != ast::DefKind::Extend {
                continue;
            }
            let name = self.defs[id].name;
            match scope.get(name) {
                // A file that names itself finds its own `extend` here.
                Some(&into) if into != id => {
                    self.groups.link(into, id);
                    merged = true;
                }
                Some(_) => {}
                None if !not_in_scope.iter().any(|n| n == name) => {
                    not_in_scope.push(name.to_string());
                }
                None => {}
            }
        }
        let target_ast = &self.project.file(target).ast;
        let reason = if target == file {
            UnusedFragmentReason::ItsOwnFile
        } else if merged || target_ast.has_views() || !target_ast.fragments.is_empty() {
            return;
        } else if not_in_scope.is_empty() {
            UnusedFragmentReason::Empty
        } else {
            UnusedFragmentReason::NotInScope(not_in_scope)
        };
        self.problems.push(Problem::UnusedFragment {
            target: fragment.path.node.clone(),
            reason,
            src: self.project.source(file),
            at: fragment.path.span.into(),
        });
    }

    fn insert_type(
        &mut self,
        scope: &mut HashMap<String, DefId>,
        name: &str,
        id: DefId,
        file: FileId,
        at: Span,
    ) {
        if let Some(&first) = scope.get(name) {
            // A same-named collision is a merge — not an error — as soon as
            // either side is an `extend` fragment. Two plain `component`s stay
            // a genuine duplicate. The group's canonical (its lone root
            // `component`) is settled in `finalize_groups`; keep a root in the
            // scope slot when we can so the entry already points at it.
            let first_extend = self.defs[first].kind == ast::DefKind::Extend;
            let new_extend = self.defs[id].kind == ast::DefKind::Extend;
            if first_extend || new_extend {
                self.groups.link(first, id);
                if first_extend && !new_extend {
                    scope.insert(name.to_string(), id);
                }
            } else {
                self.problems.push(Problem::DuplicateType {
                    name: name.to_string(),
                    src: self.project.source(file),
                    at: at.into(),
                    first: self.defs[first].ast.name.span.into(),
                });
            }
        } else {
            scope.insert(name.to_string(), id);
        }
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

#[cfg(test)]
mod tests {
    use miette::Diagnostic;

    use crate::dsl::ast;
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
    fn unused_fragment_help(problems: &[crate::dsl::diagnostics::Problem]) -> String {
        let [unused] = problems
            .iter()
            .filter(|p| {
                p.code()
                    .is_some_and(|c| c.to_string() == "wirebug::unused_fragment")
            })
            .collect::<Vec<_>>()[..]
        else {
            panic!("expected one unused_fragment: {:?}", codes(problems));
        };
        assert!(!unused.is_error(), "a warning");
        unused.help().expect("a help").to_string()
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
            unused_fragment_help(&p),
            "`a.wb` extends `vehicle` and `trailer`, which are not in scope here, and a `fragment` merges an `extend` only into a component in scope where the line is written: `use` them in this file, or write the line in a file that has them in scope, such as the one that defines them"
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
        let p = problems(&[
            (
                "main.wb",
                "fragment \"a.wb\";\ncomponent m { }\nview schematic \"M\" { }\n",
            ),
            (
                "a.wb",
                "fragment \"main.wb\";\nextend m { pub port x \"X\"; }\n",
            ),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
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
