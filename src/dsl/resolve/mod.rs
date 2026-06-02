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
use crate::dsl::ir::{ConnectorName, InstanceName, Pin, PortName};
use crate::dsl::project::Project;
use crate::dsl::span::{FileId, Span};

mod views;

/// Index of a definition in [`Resolved::defs`].
pub type DefId = usize;

/// Index of a connector type in [`Resolved::connector_types`].
pub type ConnectorTypeId = usize;

type ComponentScope = HashMap<String, DefId>;
type ConnectorTypeScope = HashMap<String, ConnectorTypeId>;

/// A flattened port of a component, with its connector grouping (if any).
pub struct PortFacts<'a> {
    pub visibility: ast::Visibility,
    pub label: &'a str,
    pub pins: Vec<Pin>,
    pub connector: Option<ConnectorRef<'a>>,
    /// Span of the port's name, for diagnostics.
    pub span: Span,
}

/// A port's connector grouping: the optional designator, part description,
/// and its order index among the component's connectors.
#[derive(Clone, Copy)]
pub struct ConnectorRef<'a> {
    pub name: Option<&'a str>,
    pub part: &'a str,
    pub index: usize,
}

/// A resolved top-level reusable connector type.
pub struct ConnectorTypeInfo<'a> {
    pub name: &'a str,
    pub file: FileId,
    pub ast: &'a ast::ConnectorType,
}

/// A direct child instance, with its resolved type (if it resolved).
pub struct InstFacts<'a> {
    pub ast: &'a ast::Instance,
    pub type_id: Option<DefId>,
}

/// A component-owned connector instance, with its connector type resolved
/// once the per-file connector-type scopes are known.
pub struct ConnectorInstFacts<'a> {
    pub ast: &'a ast::ConnectorInstance,
    pub type_id: Option<ConnectorTypeId>,
    pub index: usize,
}

/// A resolved component definition.
pub struct DefInfo<'a> {
    pub name: &'a str,
    pub file: FileId,
    pub ast: &'a ast::Definition,
    pub parent: Option<DefId>,
    pub ports: IndexMap<PortName, PortFacts<'a>>,
    pub connectors: IndexMap<ConnectorName, ConnectorInstFacts<'a>>,
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
    pub connector_types: Vec<ConnectorTypeInfo<'a>>,
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
        connector_types: Vec::new(),
        problems: Vec::new(),
    };

    // Pass 1: register every definition, recording top-level ids per file.
    let mut roots_by_file: Vec<Vec<DefId>> = vec![Vec::new(); project.files.len()];
    let mut connector_types_by_file: Vec<Vec<ConnectorTypeId>> =
        vec![Vec::new(); project.files.len()];
    for (fi, file) in project.files.iter().enumerate() {
        for item in &file.ast.items {
            match item {
                Item::Definition(def) => {
                    let id = r.register(def, FileId(fi), None);
                    roots_by_file[fi].push(id);
                }
                Item::ConnectorType(connector_type) => {
                    let id = r.register_connector_type(connector_type, FileId(fi));
                    connector_types_by_file[fi].push(id);
                }
                Item::View(_) => {}
            }
        }
    }

    // Pass 2: scopes, then references.
    let (file_scope, connector_type_scope) =
        r.build_scopes(&roots_by_file, &connector_types_by_file);
    let envs: Vec<HashMap<String, DefId>> = (0..r.defs.len())
        .map(|d| r.visible_types(d, &file_scope))
        .collect();
    r.resolve_instances(&envs);
    r.resolve_connector_instances(&connector_type_scope);
    r.apply_connector_bindings();
    r.resolve_endpoints();
    let views = r.resolve_views(&roots_by_file);

    let root = match roots_by_file[project.root.0].as_slice() {
        [only] => Some(*only),
        _ => None,
    };

    Resolved {
        defs: r.defs,
        connector_types: r.connector_types,
        root,
        views,
        problems: r.problems,
        project,
    }
}

struct Resolver<'a> {
    project: &'a Project,
    defs: Vec<DefInfo<'a>>,
    connector_types: Vec<ConnectorTypeInfo<'a>>,
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
            connectors: IndexMap::new(),
            instances: IndexMap::new(),
            nested: Vec::new(),
        });

        let mut ports: IndexMap<PortName, PortFacts<'a>> = IndexMap::new();
        let mut connectors: IndexMap<ConnectorName, ConnectorInstFacts<'a>> = IndexMap::new();
        let mut instances: IndexMap<InstanceName, InstFacts<'a>> = IndexMap::new();
        let mut nested = Vec::new();
        let mut connector_index = 0;
        let mut connector_names: HashMap<&str, Span> = HashMap::new();
        let mut cable_names: HashMap<&str, Span> = HashMap::new();

        for member in &def.members {
            match member {
                Member::Port(port) => self.add_port(&mut ports, port, None, file),
                Member::Connector(conn) => {
                    let name = conn.name.as_ref().map(|n| n.node.as_str());
                    if let (Some(n), Some(named)) = (name, conn.name.as_ref()) {
                        if let Some(&first) = connector_names.get(n) {
                            self.problems.push(Problem::DuplicateConnectorName {
                                name: n.to_string(),
                                src: self.project.source(file),
                                at: named.span.into(),
                                first: first.into(),
                            });
                        } else {
                            connector_names.insert(n, named.span);
                        }
                    }
                    let cref = ConnectorRef {
                        name,
                        part: conn.part.node.as_str(),
                        index: connector_index,
                    };
                    connector_index += 1;
                    for port in &conn.ports {
                        self.add_port(&mut ports, port, Some(cref), file);
                    }
                }
                Member::ConnectorInstance(conn) => {
                    let name = ConnectorName::from(conn.name.node.as_str());
                    if let Some(first) = connector_names.get(conn.name.node.as_str()) {
                        self.problems.push(Problem::DuplicateConnectorName {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: conn.name.span.into(),
                            first: (*first).into(),
                        });
                    } else {
                        connector_names.insert(conn.name.node.as_str(), conn.name.span);
                        connectors.insert(
                            name,
                            ConnectorInstFacts {
                                ast: conn,
                                type_id: None,
                                index: connector_index,
                            },
                        );
                        connector_index += 1;
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
                Member::Cable(cable) => {
                    // endpoints resolved in pass 2; here, guard the designator.
                    let n = cable.name.node.as_str();
                    if let Some(&first) = cable_names.get(n) {
                        self.problems.push(Problem::DuplicateCableName {
                            name: n.to_string(),
                            src: self.project.source(file),
                            at: cable.name.span.into(),
                            first: first.into(),
                        });
                    } else {
                        cable_names.insert(n, cable.name.span);
                    }
                }
                Member::Definition(child) => {
                    let child_id = self.register(child, file, Some(id));
                    nested.push(child_id);
                }
            }
        }

        self.defs[id].ports = ports;
        self.defs[id].connectors = connectors;
        self.defs[id].instances = instances;
        self.defs[id].nested = nested;
        id
    }

    fn register_connector_type(
        &mut self,
        connector_type: &'a ast::ConnectorType,
        file: FileId,
    ) -> ConnectorTypeId {
        let id = self.connector_types.len();
        self.connector_types.push(ConnectorTypeInfo {
            name: connector_type.name.node.as_str(),
            file,
            ast: connector_type,
        });
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
                        if let Some(facts) = self.defs[d].instances.get_mut(&name) {
                            facts.type_id = Some(tid);
                        }
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

    fn resolve_connector_instances(
        &mut self,
        connector_type_scope: &HashMap<FileId, HashMap<String, ConnectorTypeId>>,
    ) {
        for d in 0..self.defs.len() {
            let pending: Vec<(ConnectorName, String, Span)> = self.defs[d]
                .connectors
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
            let env = &connector_type_scope[&file];
            for (name, type_name, span) in pending {
                match env.get(&type_name) {
                    Some(&tid) => {
                        if let Some(facts) = self.defs[d].connectors.get_mut(&name) {
                            facts.type_id = Some(tid);
                        }
                    }
                    None => self.problems.push(Problem::UndefinedConnectorType {
                        name: type_name,
                        src: self.project.source(file),
                        at: span.into(),
                    }),
                }
            }
        }
    }

    fn apply_connector_bindings(&mut self) {
        let mut problems = Vec::new();
        for d in 0..self.defs.len() {
            let file = self.defs[d].file;
            let connectors: Vec<(
                ConnectorName,
                Option<ConnectorTypeId>,
                usize,
                &'a ast::ConnectorInstance,
            )> = self.defs[d]
                .connectors
                .iter()
                .map(|(name, facts)| (name.clone(), facts.type_id, facts.index, facts.ast))
                .collect();
            for (name, type_id, index, ast) in connectors {
                let Some(type_id) = type_id else { continue };
                let part = self.connector_types[type_id].ast.description.node.as_str();
                let connector_name = ast.name.node.as_str();
                let mut pins: HashMap<u32, Span> = HashMap::new();
                for binding in &ast.pins {
                    if let Some(&first) = pins.get(&binding.pin.node) {
                        problems.push(Problem::DuplicateConnectorPin {
                            pin: binding.pin.node,
                            connector: name.to_string(),
                            src: self.project.source(file),
                            at: binding.pin.span.into(),
                            first: first.into(),
                        });
                    } else {
                        pins.insert(binding.pin.node, binding.pin.span);
                    }

                    let port_name = PortName::from(binding.port.node.as_str());
                    let Some(port) = self.defs[d].ports.get_mut(&port_name) else {
                        problems.push(Problem::UnknownPort {
                            port: binding.port.node.as_str().to_string(),
                            on: String::new(),
                            src: self.project.source(file),
                            at: binding.port.span.into(),
                        });
                        continue;
                    };
                    match &port.connector {
                        Some(existing) if existing.name != Some(connector_name) => {
                            problems.push(Problem::PortConnectorConflict {
                                port: binding.port.node.as_str().to_string(),
                                src: self.project.source(file),
                                at: binding.port.span.into(),
                            });
                        }
                        Some(_) => {}
                        None => {
                            port.connector = Some(ConnectorRef {
                                name: Some(connector_name),
                                part,
                                index,
                            });
                        }
                    }
                    port.pins.push(Pin(binding.pin.node));
                }
            }
        }
        self.problems.extend(problems);
    }

    fn resolve_endpoints(&mut self) {
        let mut problems = Vec::new();
        for d in 0..self.defs.len() {
            let ast = self.defs[d].ast;
            for member in &ast.members {
                let wires: &[ast::Wire] = match member {
                    Member::Wire(wire) => std::slice::from_ref(wire),
                    Member::Cable(cable) => &cable.wires,
                    _ => continue,
                };
                for wire in wires {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::project::load;

    /// Write `files` into a temp dir and resolve, returning every problem
    /// (loading + resolution). Auto-writes a minimal `wirebug.toml` so a
    /// missing-manifest problem doesn't leak into resolve-focused tests.
    fn problems(files: &[(&str, &str)]) -> Vec<Problem> {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
        )
        .expect("write wirebug.toml");
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
    fn fixture_project_resolves_cleanly() {
        let main = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/main.wb"
        ));
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

    /// Single top-level `m` (a `leaf l` child, a `pub` port `a`, a private
    /// `secret`, wired to the child) with a schematic view carrying the given
    /// `enclosure { }` body. `leaf` lives in `leaf.wb` so `m` is the lone root.
    fn view_with_enclosure(enclosure_body: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\"\ncomponent m {{ leaf l; pub port a \"A\"; port secret \"S\"; wire red 1 [a, l.p]; }}\nview schematic \"V\" {{ enclosure {{ {enclosure_body} }} include l at (0, 0) ports {{ west: p; }}; }}\n"
                ),
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ])
    }

    #[test]
    fn enclosure_anchor_without_a_side_is_reported() {
        let p = view_with_enclosure("a at (1, 2);");
        assert!(has(&p, "wirebug::enclosure_anchor"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_side_in_the_wrong_slot_is_reported() {
        // north names a horizontal edge, so it belongs in the second slot.
        let p = view_with_enclosure("a at (north, 2);");
        assert!(has(&p, "wirebug::enclosure_anchor"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_unknown_side_is_reported() {
        let p = view_with_enclosure("a at (up, 2);");
        assert!(has(&p, "wirebug::unknown_port_side"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_unknown_subject_port_is_reported() {
        let p = view_with_enclosure("nope at (west, 2);");
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn enclosure_private_subject_port_is_reported() {
        let p = view_with_enclosure("secret at (west, 2);");
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn well_formed_enclosure_resolves_cleanly() {
        let p = view_with_enclosure("a at (west, 2);");
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn view_body_items_may_appear_in_any_order() {
        // include first, then enclosure, then grid — the reverse of the
        // canonical order — still parses and resolves cleanly.
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\"\ncomponent m { leaf l; pub port a \"A\"; wire red 1 [a, l.p]; }\nview schematic \"V\" { include l at (0,0) ports { west: p; }; enclosure { a at (west, 2); } grid 10; }\n",
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn duplicate_grid_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { grid 10; grid 20; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_view_item"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_enclosure_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; } view schematic \"V\" { enclosure { a at (west, 0); } enclosure { } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_view_item"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_text_box_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { text note at (0, 0) \"A\"; text note at (1, 1) \"B\"; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_view_text"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_schematic_include_is_reported() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\"\ncomponent m { leaf l; }\nview schematic \"V\" { include l at (0, 0); include l at (2, 0); }\n",
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ]);
        assert!(
            has(&p, "wirebug::duplicate_view_include"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_harness_connector_include_is_reported() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\"\ncomponent m { leaf l; }\nview harness \"H\" { include l.j1 at (0, 0); include l.j1 at (2, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector j1 \"J1\" { pub port p \"P\" pin 1; } }\n",
            ),
        ]);
        assert!(
            has(&p, "wirebug::duplicate_view_include"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn harness_can_include_two_connectors_from_one_instance() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\"\ncomponent m { leaf l; }\nview harness \"H\" { include l.j1 at (0, 0); include l.j2 at (2, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector j1 \"J1\" { pub port p \"P\" pin 1; } connector j2 \"J2\" { pub port q \"Q\" pin 1; } }\n",
            ),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn text_box_in_harness_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view harness \"H\" { text note at (0, 0) \"A\"; }\n",
        )]);
        assert!(has(&p, "wirebug::unsupported_view_text"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_connector_name_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { connector x \"A\" { pub port a \"A\" pin 1; } connector x \"B\" { pub port b \"B\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_name"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_cable_name_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; pub port b \"B\"; cable c { wire red 1 [a, b]; } cable c { wire red 1 [a, b]; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_cable_name"), "{:?}", codes(&p));
    }

    #[test]
    fn connector_instance_resolves_local_connector_type() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\ncomponent m { pub port can_h \"CAN H\"; connector x1: ampseal { pin 1 = can_h; } }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn connector_instance_resolves_imported_connector_type() {
        let p = problems(&[
            (
                "main.wb",
                "use ampseal from \"connectors.wb\"\ncomponent m { pub port can_h \"CAN H\"; connector x1: ampseal { pin 1 = can_h; } }\n",
            ),
            (
                "connectors.wb",
                "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\n",
            ),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn unknown_connector_type_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port can_h \"CAN H\"; connector x1: ghost { pin 1 = can_h; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::undefined_connector_type"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn unknown_connector_bound_port_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { connector x1: ampseal { pin 1 = nope; } }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_connector_pin_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port can_h \"CAN H\"; pub port can_l \"CAN L\"; connector x1: ampseal { pin 1 = can_h; pin 1 = can_l; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_pin"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn one_port_can_bind_to_multiple_pins_on_one_connector() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port gnd \"GND\"; connector x1: ampseal { pin 1 = gnd; pin 2 = gnd; } }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn cable_endpoints_resolve_like_loose_wires() {
        // An unknown port inside a cable wire is caught just like a loose wire.
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; cable c { wire red 1 [a, ghost]; } }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_port"), "{:?}", codes(&p));
    }

    /// A two-file project whose `main.wb` holds one `leaf` instance `l` and a
    /// harness view with the given include body. `leaf` (in `leaf.wb`) has a
    /// connector designated `hv`.
    fn harness_view(include: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\"\ncomponent m {{ leaf l; }}\nview harness \"H\" {{ {include} }}\n"
                ),
            ),
            (
                "leaf.wb",
                "component leaf { connector hv \"HV 2p\" { pub port hv_pos \"HV+\" pin 1; pub port hv_neg \"HV-\" pin 2; } }\n",
            ),
        ])
    }

    #[test]
    fn harness_include_resolves_cleanly() {
        let p = harness_view("include l.hv at (0, 0);");
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn harness_include_resolves_connector_instance() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\"\ncomponent m { leaf l; }\nview harness \"H\" { include l.hv at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "connector_type hv_2p \"HV 2p\" { }\ncomponent leaf { pub port hv_pos \"HV+\"; connector hv: hv_2p { pin 1 = hv_pos; } }\n",
            ),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn unknown_connector_errors() {
        let p = harness_view("include l.nope at (0, 0);");
        assert!(has(&p, "wirebug::unknown_connector"), "{:?}", codes(&p));
    }

    #[test]
    fn harness_include_without_connector_errors() {
        let p = harness_view("include l at (0, 0);");
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn ports_block_on_harness_include_errors() {
        let p = harness_view("include l.hv at (0, 0) ports { west: hv_pos; };");
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn connector_on_schematic_include_errors() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\"\ncomponent m { leaf l; }\nview schematic \"S\" { include l.hv at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector hv \"HV 2p\" { pub port hv_pos \"HV+\" pin 1; } }\n",
            ),
        ]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }
}
