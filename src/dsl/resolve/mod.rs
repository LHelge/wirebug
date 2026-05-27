//! Name resolution: build a registry of every component definition,
//! bind type/instance/port/include references, and report the resolution
//! errors (undefined type, unresolved import, duplicates, bad endpoints,
//! private-port access, unknown view includes).
//!
//! The result is a [`Resolved`] registry that elaboration consumes. Each
//! definition (top-level or nested) gets a [`DefId`]; ports are flattened
//! across connectors (which are grouping metadata, not namespaces, so
//! port names are unique per component).

use std::collections::HashMap;
use std::path::PathBuf;

use indexmap::IndexMap;

use crate::dsl::ast::{self, Item, Member};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{InstanceName, Pin, PortName, Side};
use crate::dsl::project::Project;
use crate::dsl::span::{FileId, Span};

/// Index of a definition in [`Resolved::defs`].
pub type DefId = usize;

/// A flattened port of a component, with its connector grouping (if any).
pub struct PortFacts<'a> {
    pub visibility: ast::Visibility,
    pub label: &'a str,
    pub pins: Vec<Pin>,
    pub connector: Option<ConnectorRef<'a>>,
    /// Span of the port's name, for diagnostics.
    pub span: Span,
}

/// A port's connector grouping: the part description and its order index.
pub struct ConnectorRef<'a> {
    pub part: &'a str,
    pub index: usize,
}

/// A direct child instance, with its resolved type (if it resolved).
pub struct InstFacts<'a> {
    pub ast: &'a ast::Instance,
    pub type_id: Option<DefId>,
}

/// A resolved component definition.
pub struct DefInfo<'a> {
    pub name: &'a str,
    pub file: FileId,
    pub ast: &'a ast::Definition,
    pub parent: Option<DefId>,
    pub ports: IndexMap<PortName, PortFacts<'a>>,
    pub instances: IndexMap<InstanceName, InstFacts<'a>>,
    pub nested: Vec<DefId>,
}

/// A view bound to the component it documents.
pub struct ViewBinding<'a> {
    pub ast: &'a ast::View,
    pub subject: Option<DefId>,
}

/// The output of resolution: the definition registry, the design root
/// (main.wb's sole top-level component), the views, and any problems.
pub struct Resolved<'a> {
    pub defs: Vec<DefInfo<'a>>,
    pub root: Option<DefId>,
    pub views: Vec<ViewBinding<'a>>,
    pub problems: Vec<Problem>,
    pub project: &'a Project,
}

/// Resolve every name in `project`.
pub fn resolve(project: &Project) -> Resolved<'_> {
    let mut r = Resolver {
        project,
        defs: Vec::new(),
        problems: Vec::new(),
    };

    // Pass 1: register every definition, recording top-level ids per file.
    let mut roots_by_file: Vec<Vec<DefId>> = vec![Vec::new(); project.files.len()];
    for (fi, file) in project.files.iter().enumerate() {
        for item in &file.ast.items {
            if let Item::Definition(def) = item {
                let id = r.register(def, FileId(fi), None);
                roots_by_file[fi].push(id);
            }
        }
    }

    // Pass 2: scopes, then references.
    let file_scope = r.build_scopes(&roots_by_file);
    let envs: Vec<HashMap<String, DefId>> = (0..r.defs.len())
        .map(|d| r.visible_types(d, &file_scope))
        .collect();
    r.resolve_instances(&envs);
    r.resolve_endpoints();
    let views = r.resolve_views(&roots_by_file);

    let root = match roots_by_file[project.root.0].as_slice() {
        [only] => Some(*only),
        _ => None,
    };

    Resolved {
        defs: r.defs,
        root,
        views,
        problems: r.problems,
        project,
    }
}

struct Resolver<'a> {
    project: &'a Project,
    defs: Vec<DefInfo<'a>>,
    problems: Vec<Problem>,
}

impl<'a> Resolver<'a> {
    /// Register a definition and its nested definitions, flattening ports
    /// and collecting instances. Returns the new [`DefId`].
    fn register(&mut self, def: &'a ast::Definition, file: FileId, parent: Option<DefId>) -> DefId {
        let id = self.defs.len();
        self.defs.push(DefInfo {
            name: def.name.node.as_str(),
            file,
            ast: def,
            parent,
            ports: IndexMap::new(),
            instances: IndexMap::new(),
            nested: Vec::new(),
        });

        let mut ports: IndexMap<PortName, PortFacts<'a>> = IndexMap::new();
        let mut instances: IndexMap<InstanceName, InstFacts<'a>> = IndexMap::new();
        let mut nested = Vec::new();
        let mut connector_index = 0;

        for member in &def.members {
            match member {
                Member::Port(port) => self.add_port(&mut ports, port, None, file),
                Member::Connector(conn) => {
                    let cref = ConnectorRef {
                        part: conn.part.node.as_str(),
                        index: connector_index,
                    };
                    connector_index += 1;
                    for port in &conn.ports {
                        self.add_port(
                            &mut ports,
                            port,
                            Some(ConnectorRef {
                                part: cref.part,
                                index: cref.index,
                            }),
                            file,
                        );
                    }
                }
                Member::Instance(inst) => {
                    let name = InstanceName::from(inst.name.node.as_str());
                    if let Some(first) = instances.get(&name) {
                        self.problems.push(Problem::DuplicateInstance {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: inst.name.span.into(),
                            first: first.ast.name.span.into(),
                        });
                    } else {
                        instances.insert(
                            name,
                            InstFacts {
                                ast: inst,
                                type_id: None,
                            },
                        );
                    }
                }
                Member::Wire(_) => {} // endpoints resolved in pass 2
                Member::Definition(child) => {
                    let child_id = self.register(child, file, Some(id));
                    nested.push(child_id);
                }
            }
        }

        self.defs[id].ports = ports;
        self.defs[id].instances = instances;
        self.defs[id].nested = nested;
        id
    }

    fn add_port(
        &mut self,
        ports: &mut IndexMap<PortName, PortFacts<'a>>,
        port: &'a ast::Port,
        connector: Option<ConnectorRef<'a>>,
        file: FileId,
    ) {
        let name = PortName::from(port.name.node.as_str());
        if let Some(first) = ports.get(&name) {
            self.problems.push(Problem::DuplicatePort {
                name: name.to_string(),
                src: self.project.source(file),
                at: port.name.span.into(),
                first: first.span.into(),
            });
            return;
        }
        ports.insert(
            name,
            PortFacts {
                visibility: port.visibility,
                label: port.label.node.as_str(),
                pins: port.pins.iter().map(|p| Pin(p.node)).collect(),
                connector,
                span: port.name.span,
            },
        );
    }

    /// Build each file's type-name scope: its own top-level definitions
    /// plus the names brought in by `use`.
    fn build_scopes(
        &mut self,
        roots_by_file: &[Vec<DefId>],
    ) -> HashMap<FileId, HashMap<String, DefId>> {
        let path_to_file: HashMap<PathBuf, FileId> = self
            .project
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), FileId(i)))
            .collect();

        let mut scopes: HashMap<FileId, HashMap<String, DefId>> = HashMap::new();
        for (fi, file) in self.project.files.iter().enumerate() {
            let fid = FileId(fi);
            let mut scope: HashMap<String, DefId> = HashMap::new();

            for &id in &roots_by_file[fi] {
                self.insert_type(
                    &mut scope,
                    self.defs[id].name,
                    id,
                    fid,
                    self.defs[id].ast.name.span,
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
                match roots_by_file[tfid.0]
                    .iter()
                    .copied()
                    .find(|&id| self.defs[id].name == wanted)
                {
                    Some(id) => {
                        self.insert_type(&mut scope, wanted, id, fid, use_decl.name.span);
                    }
                    None => self.problems.push(Problem::UnresolvedImport {
                        name: wanted.to_string(),
                        file: use_decl.path.node.clone(),
                        src: self.project.source(fid),
                        at: use_decl.name.span.into(),
                    }),
                }
            }

            scopes.insert(fid, scope);
        }
        scopes
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
            self.problems.push(Problem::DuplicateType {
                name: name.to_string(),
                src: self.project.source(file),
                at: at.into(),
                first: self.defs[first].ast.name.span.into(),
            });
        } else {
            scope.insert(name.to_string(), id);
        }
    }

    /// Types instantiable inside definition `d`: the file scope overlaid
    /// with the nested definitions of `d` and its ancestors (inner wins).
    fn visible_types(
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

    fn resolve_instances(&mut self, envs: &[HashMap<String, DefId>]) {
        for (d, env) in envs.iter().enumerate() {
            let pending: Vec<(InstanceName, String, Span)> = self.defs[d]
                .instances
                .iter()
                .map(|(name, facts)| {
                    (
                        name.clone(),
                        facts.ast.type_name.node.as_str().to_string(),
                        facts.ast.type_name.span,
                    )
                })
                .collect();
            let file = self.defs[d].file;
            for (name, type_name, span) in pending {
                match env.get(&type_name) {
                    Some(&tid) => {
                        self.defs[d].instances.get_mut(&name).unwrap().type_id = Some(tid);
                    }
                    None => self.problems.push(Problem::UndefinedType {
                        name: type_name,
                        src: self.project.source(file),
                        at: span.into(),
                    }),
                }
            }
        }
    }

    fn resolve_endpoints(&mut self) {
        let mut problems = Vec::new();
        for d in 0..self.defs.len() {
            let ast = self.defs[d].ast;
            for member in &ast.members {
                if let Member::Wire(wire) = member {
                    for ep in &wire.endpoints {
                        if let Some(problem) = self.check_endpoint(d, ep) {
                            problems.push(problem);
                        }
                    }
                }
            }
        }
        self.problems.extend(problems);
    }

    fn check_endpoint(&self, d: DefId, ep: &ast::Endpoint) -> Option<Problem> {
        let file = self.defs[d].file;
        let port = ep.port.node.as_str();
        match &ep.instance {
            None => {
                if self.defs[d].ports.contains_key(&PortName::from(port)) {
                    None
                } else {
                    Some(Problem::UnknownPort {
                        port: port.to_string(),
                        on: String::new(),
                        src: self.project.source(file),
                        at: ep.port.span.into(),
                    })
                }
            }
            Some(inst) => {
                let iname = inst.node.as_str();
                let Some(facts) = self.defs[d].instances.get(&InstanceName::from(iname)) else {
                    return Some(Problem::UnknownInstance {
                        name: iname.to_string(),
                        src: self.project.source(file),
                        at: inst.span.into(),
                    });
                };
                let tid = facts.type_id?; // undefined type already reported
                match self.defs[tid].ports.get(&PortName::from(port)) {
                    None => Some(Problem::UnknownPort {
                        port: port.to_string(),
                        on: format!(" on `{}`", self.defs[tid].name),
                        src: self.project.source(file),
                        at: ep.port.span.into(),
                    }),
                    Some(facts) if facts.visibility != ast::Visibility::Public => {
                        Some(Problem::PrivatePort {
                            port: port.to_string(),
                            ty: self.defs[tid].name.to_string(),
                            src: self.project.source(file),
                            at: ep.port.span.into(),
                        })
                    }
                    Some(_) => None,
                }
            }
        }
    }

    /// Validate one include's `ports { }` placements against the included
    /// instance's type: side names, duplicate ports, port existence, and
    /// `pub` visibility (mirroring the wire-endpoint rule).
    fn check_view_ports(
        &self,
        inc: &ast::Include,
        type_id: Option<DefId>,
        file: FileId,
        problems: &mut Vec<Problem>,
    ) {
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for placement in &inc.ports {
            let side = placement.side.node.as_str();
            if side.parse::<Side>().is_err() {
                problems.push(Problem::UnknownPortSide {
                    found: side.to_string(),
                    src: self.project.source(file),
                    at: placement.side.span.into(),
                });
            }

            let port = placement.port.node.as_str();
            if let Some(&first) = seen.get(port) {
                problems.push(Problem::DuplicateViewPort {
                    port: port.to_string(),
                    src: self.project.source(file),
                    at: placement.port.span.into(),
                    first: first.into(),
                });
            } else {
                seen.insert(port, placement.port.span);
            }

            let Some(tid) = type_id else { continue }; // undefined type already reported
            match self.defs[tid].ports.get(&PortName::from(port)) {
                None => problems.push(Problem::UnknownPort {
                    port: port.to_string(),
                    on: format!(" on `{}`", self.defs[tid].name),
                    src: self.project.source(file),
                    at: placement.port.span.into(),
                }),
                Some(facts) if facts.visibility != ast::Visibility::Public => {
                    problems.push(Problem::PrivatePort {
                        port: port.to_string(),
                        ty: self.defs[tid].name.to_string(),
                        src: self.project.source(file),
                        at: placement.port.span.into(),
                    });
                }
                Some(_) => {}
            }
        }
    }

    fn resolve_views(&mut self, roots_by_file: &[Vec<DefId>]) -> Vec<ViewBinding<'a>> {
        let mut bindings = Vec::new();
        let mut problems = Vec::new();
        for (fi, file) in self.project.files.iter().enumerate() {
            let roots = &roots_by_file[fi];
            for item in &file.ast.items {
                let Item::View(view) = item else { continue };
                let subject = if roots.len() == 1 {
                    Some(roots[0])
                } else {
                    problems.push(Problem::ViewSubject {
                        src: self.project.source(FileId(fi)),
                        at: view.span.into(),
                    });
                    None
                };
                if let Some(s) = subject {
                    for inc in &view.includes {
                        let name = inc.instance.node.as_str();
                        match self.defs[s].instances.get(&InstanceName::from(name)) {
                            None => problems.push(Problem::UnknownInclude {
                                name: name.to_string(),
                                src: self.project.source(FileId(fi)),
                                at: inc.instance.span.into(),
                            }),
                            Some(facts) => {
                                self.check_view_ports(inc, facts.type_id, FileId(fi), &mut problems)
                            }
                        }
                    }
                }
                bindings.push(ViewBinding { ast: view, subject });
            }
        }
        self.problems.extend(problems);
        bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::project::load;

    /// Write `files` into a temp dir and resolve, returning every problem
    /// (loading + resolution).
    fn problems(files: &[(&str, &str)]) -> Vec<Problem> {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let (project, mut problems) = load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        problems.extend(resolve(&project).problems);
        problems
    }

    fn codes(problems: &[Problem]) -> Vec<String> {
        use miette::Diagnostic;
        problems
            .iter()
            .filter_map(|p| p.code().map(|c| c.to_string()))
            .collect()
    }

    fn has(problems: &[Problem], code: &str) -> bool {
        codes(problems).iter().any(|c| c == code)
    }

    #[test]
    fn seed_project_resolves_cleanly() {
        let main =
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/main.wb"));
        let (project, load_problems) = load(&main);
        assert!(load_problems.is_empty(), "load: {load_problems:?}");
        let project = project.expect("loads");
        let resolved = resolve(&project);
        assert!(
            resolved.problems.is_empty(),
            "resolution problems: {:?}",
            codes(&resolved.problems)
        );
        assert!(resolved.root.is_some(), "vehicle is the root");
    }

    #[test]
    fn undefined_type_is_reported() {
        let p = problems(&[("main.wb", "component m { ghost g; }\n")]);
        assert!(has(&p, "wirebug::undefined_type"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_instance_name_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; }\ncomponent m { leaf x; leaf x; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_port_across_connectors_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; connector \"C\" { pub port a \"A2\" pin 1; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_port"), "{:?}", codes(&p));
    }

    #[test]
    fn private_port_access_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { port secret \"S\"; }\ncomponent m { leaf l; wire red 1 [l.secret, l.secret]; }\n",
        )]);
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_port_on_instance_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; }\ncomponent m { leaf l; wire red 1 [l.nope, l.a]; }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_instance_in_wire_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { wire red 1 [ghost.a, ghost.b]; }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn unresolved_import_is_reported() {
        let p = problems(&[
            (
                "main.wb",
                "use missing from \"leaf.wb\"\ncomponent m { missing x; }\n",
            ),
            ("leaf.wb", "component other { pub port a \"A\"; }\n"),
        ]);
        assert!(has(&p, "wirebug::unresolved_import"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_view_include_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { include ghost at (0, 0); }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_include"), "{:?}", codes(&p));
    }

    /// A two-file project whose `main.wb` is a single top-level component
    /// `m` holding one `leaf` instance `l`, plus a view placing `l` with the
    /// given `ports { }` body. `leaf` lives in `leaf.wb`.
    fn view_with_ports(leaf_body: &str, ports_body: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\"\ncomponent m {{ leaf l; }}\nview schematic \"V\" {{ include l at (0, 0) ports {{ {ports_body} }}; }}\n"
                ),
            ),
            ("leaf.wb", &format!("component leaf {{ {leaf_body} }}\n")),
        ])
    }

    #[test]
    fn unknown_port_side_in_view_is_reported() {
        let p = view_with_ports("pub port a \"A\";", "up: a;");
        assert!(has(&p, "wirebug::unknown_port_side"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_port_in_view_is_reported() {
        let p = view_with_ports("pub port a \"A\";", "west: nope;");
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn private_port_in_view_is_reported() {
        let p = view_with_ports("port secret \"S\";", "west: secret;");
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_port_in_view_is_reported() {
        let p = view_with_ports("pub port a \"A\";", "west: a; east: a;");
        assert!(has(&p, "wirebug::duplicate_view_port"), "{:?}", codes(&p));
    }
}
