//! Per-file name scopes: each file sees its own top-level definitions and
//! connector types plus whatever its `use` declarations import. A same-name
//! component collision is a merge (when either side is an `extend`) — that
//! discovery happens here, feeding the merge machinery in [`super::merge`].

use std::collections::HashMap;
use std::path::PathBuf;

use crate::dsl::ast;
use crate::dsl::diagnostics::Problem;
use crate::dsl::span::{FileId, Span};

use super::{ComponentScope, ConnectorTypeId, ConnectorTypeScope, DefId, Resolver};

impl<'a> Resolver<'a> {
    /// Build each file's type-name scope: its own top-level definitions
    /// plus the names brought in by `use`.
    pub(super) fn build_scopes(
        &mut self,
        roots_by_file: &[Vec<DefId>],
        connector_types_by_file: &[Vec<ConnectorTypeId>],
    ) -> (
        HashMap<FileId, ComponentScope>,
        HashMap<FileId, ConnectorTypeScope>,
    ) {
        let path_to_file: HashMap<PathBuf, FileId> = self
            .project
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), FileId(i)))
            .collect();

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
                let dir = file
                    .path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                let Ok(target) = dir.join(&use_decl.path.node).canonicalize() else {
                    continue; // loader already reported the missing file
                };
                let Some(&tfid) = path_to_file.get(&target) else {
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

            scopes.insert(fid, scope);
            connector_type_scopes.insert(fid, connector_type_scope);
        }
        (scopes, connector_type_scopes)
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
    use crate::dsl::resolve::testkit::{codes, has, problems};

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
    fn two_plain_components_with_one_name_still_duplicate() {
        let p = problems(&[(
            "main.wb",
            "component dup { }\ncomponent dup { }\ncomponent root { d: dup; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_type"), "{:?}", codes(&p));
    }
}
