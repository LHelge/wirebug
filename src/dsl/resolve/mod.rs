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

/// A port's connector grouping: the designator, the optional human
/// description of the physical part, and its order index among the
/// component's connectors.
#[derive(Clone, Copy)]
pub struct ConnectorRef<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
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
    pub kind: ast::DefKind,
    pub ast: &'a ast::Definition,
    pub parent: Option<DefId>,
    pub ports: IndexMap<PortName, PortFacts<'a>>,
    pub connectors: IndexMap<ConnectorName, ConnectorInstFacts<'a>>,
    pub instances: IndexMap<InstanceName, InstFacts<'a>>,
    pub nested: Vec<DefId>,
}

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
    fn link(&mut self, a: DefId, b: DefId) {
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
    pub groups: MergeGroups,
    pub root: Option<DefId>,
    pub views: Vec<ViewBinding<'a>>,
    pub problems: Vec<Problem>,
    pub project: &'a Project,
}

impl Resolved<'_> {
    /// Every fragment id of `d`'s merged component, canonical first.
    pub fn fragments(&self, d: DefId) -> Vec<DefId> {
        self.groups.fragments(d)
    }
}

/// Resolve every name in `project`.
pub fn resolve(project: &Project) -> Resolved<'_> {
    let mut r = Resolver {
        project,
        defs: Vec::new(),
        connector_types: Vec::new(),
        groups: MergeGroups::default(),
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

    // Pass 2: scopes (which discover merge groups), then references. The
    // merge groups are finalized before any reference pass so that endpoint
    // and view lookups see the flat merged namespace, and instance-type
    // scopes resolve to canonical ids.
    let (mut file_scope, connector_type_scope) =
        r.build_scopes(&roots_by_file, &connector_types_by_file);
    r.finalize_groups();
    r.report_orphan_fragments();
    r.validate_merge_groups();
    r.canonicalize_scopes(&mut file_scope);
    let envs: Vec<HashMap<String, DefId>> = (0..r.defs.len())
        .map(|d| r.visible_types(d, &file_scope))
        .collect();
    r.resolve_instances(&envs);
    r.resolve_connector_instances(&connector_type_scope);
    r.apply_connector_types();
    r.resolve_endpoints();
    let views = r.resolve_views(&roots_by_file);

    let root = match roots_by_file[project.root.0].as_slice() {
        [only] => Some(r.groups.canonical(*only)),
        _ => None,
    };

    Resolved {
        defs: r.defs,
        connector_types: r.connector_types,
        groups: r.groups,
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
    groups: MergeGroups,
    problems: Vec<Problem>,
}

/// Move `id` to the front of `group`, preserving the relative order of the
/// rest. Used to seat a merge group's canonical fragment first.
fn move_first(group: &mut Vec<DefId>, id: DefId) {
    group.retain(|&d| d != id);
    group.insert(0, id);
}

impl<'a> Resolver<'a> {
    /// Register a definition and its nested definitions, flattening ports
    /// and collecting instances. Returns the new [`DefId`].
    fn register(&mut self, def: &'a ast::Definition, file: FileId, parent: Option<DefId>) -> DefId {
        let id = self.defs.len();
        // `extend` only splits a top-level component; nested fragments make no
        // sense (a nested definition is private to its parent).
        if def.kind == ast::DefKind::Extend && parent.is_some() {
            self.problems.push(Problem::NestedExtend {
                src: self.project.source(file),
                at: def.name.span.into(),
            });
        }
        self.defs.push(DefInfo {
            name: def.name.node.as_str(),
            file,
            kind: def.kind,
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
                    let name = conn.name.node.as_str();
                    if let Some(&first) = connector_names.get(name) {
                        self.problems.push(Problem::DuplicateConnectorName {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: conn.name.span.into(),
                            first: first.into(),
                        });
                    } else {
                        connector_names.insert(name, conn.name.span);
                    }
                    let cref = ConnectorRef {
                        name,
                        description: conn.description.as_ref().map(|d| d.node.as_str()),
                        index: connector_index,
                    };
                    connector_index += 1;
                    for port in &conn.ports {
                        self.add_port(&mut ports, port, Some(cref), file);
                    }
                    self.check_duplicate_pins(name, &conn.ports, file);
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
                    }
                    // Same flat port declarations as an inline connector; the
                    // description comes from the connector type, resolved
                    // later — `apply_connector_types` fills it in.
                    let cref = ConnectorRef {
                        name: conn.name.node.as_str(),
                        description: None,
                        index: connector_index,
                    };
                    connector_index += 1;
                    for port in &conn.ports {
                        self.add_port(&mut ports, port, Some(cref), file);
                    }
                    self.check_duplicate_pins(conn.name.node.as_str(), &conn.ports, file);
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

    /// One cavity, one port: flag a pin number claimed twice within a single
    /// connector (inline or typed), including twice in one `pins [..]` list.
    fn check_duplicate_pins(&mut self, connector: &str, ports: &[ast::Port], file: FileId) {
        let mut pins: HashMap<u32, Span> = HashMap::new();
        for port in ports {
            for pin in &port.pins {
                if let Some(&first) = pins.get(&pin.node) {
                    self.problems.push(Problem::DuplicateConnectorPin {
                        pin: pin.node,
                        connector: connector.to_string(),
                        src: self.project.source(file),
                        at: pin.span.into(),
                        first: first.into(),
                    });
                } else {
                    pins.insert(pin.node, pin.span);
                }
            }
        }
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

    /// Settle every merge group's canonical (the lone root `component`) into
    /// first position, and report groups that lack a root (`OrphanFragment`)
    /// or carry two (`DuplicateType`).
    fn finalize_groups(&mut self) {
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
    fn report_orphan_fragments(&mut self) {
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
    fn validate_merge_groups(&mut self) {
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
                for (name, facts) in &self.defs[f].instances {
                    if let Some(first) = instances.get(name.as_str()).copied() {
                        problems.push(Problem::DuplicateInstance {
                            name: name.to_string(),
                            src: self.project.source(file),
                            at: facts.ast.name.span.into(),
                            first: first.into(),
                        });
                    } else {
                        instances.insert(name.to_string(), facts.ast.name.span);
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

    /// Rewrite every type-scope entry to its group's canonical id, so an
    /// instance of a merged type resolves to the one representative def.
    fn canonicalize_scopes(&self, scopes: &mut HashMap<FileId, ComponentScope>) {
        for scope in scopes.values_mut() {
            for id in scope.values_mut() {
                *id = self.groups.canonical(*id);
            }
        }
    }

    /// Find an instance by name across every fragment of `d`'s merged
    /// component (the flat namespace).
    fn lookup_instance(&self, d: DefId, name: &InstanceName) -> Option<&InstFacts<'a>> {
        self.groups
            .fragments(d)
            .into_iter()
            .find_map(|f| self.defs[f].instances.get(name))
    }

    /// Find a port by name across every fragment of `d`'s merged component.
    fn lookup_port(&self, d: DefId, name: &PortName) -> Option<&PortFacts<'a>> {
        self.groups
            .fragments(d)
            .into_iter()
            .find_map(|f| self.defs[f].ports.get(name))
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

    /// Fill each typed connector's port `ConnectorRef.description` from its
    /// resolved connector type. Pass 1 registered the ports without one —
    /// the type wasn't resolved yet.
    fn apply_connector_types(&mut self) {
        for d in 0..self.defs.len() {
            let typed: Vec<(&'a str, &'a str, Vec<PortName>)> = self.defs[d]
                .connectors
                .values()
                .filter_map(|facts| {
                    let type_id = facts.type_id?;
                    Some((
                        facts.ast.name.node.as_str(),
                        self.connector_types[type_id].ast.description.node.as_str(),
                        facts
                            .ast
                            .ports
                            .iter()
                            .map(|p| PortName::from(p.name.node.as_str()))
                            .collect(),
                    ))
                })
                .collect();
            for (connector_name, description, port_names) in typed {
                for name in port_names {
                    // A duplicate-named port kept the *first* declaration in
                    // the registry; the name guard skips it if that one
                    // belongs to a different connector.
                    if let Some(port) = self.defs[d].ports.get_mut(&name)
                        && let Some(cref) = &mut port.connector
                        && cref.name == connector_name
                    {
                        cref.description = Some(description);
                    }
                }
            }
        }
    }

    fn resolve_endpoints(&mut self) {
        let mut problems = Vec::new();
        for d in 0..self.defs.len() {
            let ast = self.defs[d].ast;
            for member in &ast.members {
                let wires: Vec<&ast::Wire> = match member {
                    Member::Wire(wire) => vec![wire],
                    Member::Cable(cable) => cable.wires().collect(),
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
        // Flat namespace: an endpoint resolves against the merged instances
        // and ports of `d`'s whole component, so one fragment's wire may reach
        // an instance or own-port declared in another fragment.
        match &ep.instance {
            None => {
                if self.lookup_port(d, &PortName::from(port)).is_some() {
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
                let Some(facts) = self.lookup_instance(d, &InstanceName::from(iname)) else {
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
        let p = problems(&[("main.wb", "component m { g: ghost; }\n")]);
        assert!(has(&p, "wirebug::undefined_type"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_instance_name_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; }\ncomponent m { x: leaf; x: leaf; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_instance"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_port_across_connectors_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { pub port a \"A\"; connector c \"C\" { pub port a \"A2\" pin 1; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_port"), "{:?}", codes(&p));
    }

    #[test]
    fn private_port_access_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { port secret \"S\"; }\ncomponent m { l: leaf; wire red 1 [l.secret, l.secret]; }\n",
        )]);
        assert!(has(&p, "wirebug::private_port"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_port_on_instance_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component leaf { pub port a \"A\"; }\ncomponent m { l: leaf; wire red 1 [l.nope, l.a]; }\n",
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
                "use missing from \"leaf.wb\";\ncomponent m { x: missing; }\n",
            ),
            ("leaf.wb", "component other { pub port a \"A\"; }\n"),
        ]);
        assert!(has(&p, "wirebug::unresolved_import"), "{:?}", codes(&p));
    }

    #[test]
    fn unknown_view_kind_is_reported() {
        let p = problems(&[("main.wb", "component m { } view schemtic \"V\" { }\n")]);
        assert!(has(&p, "wirebug::unknown_view_kind"), "{:?}", codes(&p));
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
                    "use leaf from \"leaf.wb\";\ncomponent m {{ l: leaf; }}\nview schematic \"V\" {{ include l at (0, 0) ports {{ {ports_body} }} }}\n"
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
                    "use leaf from \"leaf.wb\";\ncomponent m {{ l: leaf; pub port a \"A\"; port secret \"S\"; wire red 1 [a, l.p]; }}\nview schematic \"V\" {{ enclosure {{ {enclosure_body} }} include l at (0, 0) ports {{ west: p; }} }}\n"
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
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; pub port a \"A\"; wire red 1 [a, l.p]; }\nview schematic \"V\" { include l at (0,0) ports { west: p; } enclosure { a at (west, 2); } grid: 10; }\n",
            ),
            ("leaf.wb", "component leaf { pub port p \"P\"; }\n"),
        ]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn duplicate_grid_in_view_is_reported() {
        let p = problems(&[(
            "main.wb",
            "component m { } view schematic \"V\" { grid: 10; grid: 20; }\n",
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
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview schematic \"V\" { include l at (0, 0); include l at (2, 0); }\n",
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
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview harness \"H\" { include l.j1 at (0, 0); include l.j1 at (2, 0); }\n",
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
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview harness \"H\" { include l.j1 at (0, 0); include l.j2 at (2, 0); }\n",
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
            "connector_type ampseal \"AMPSEAL\" { part: \"TE\"; }\ncomponent m { connector x1: ampseal { pub port can_h \"CAN H\" pin 1; } }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn connector_instance_resolves_imported_connector_type() {
        let p = problems(&[
            (
                "main.wb",
                "use ampseal from \"connectors.wb\";\ncomponent m { connector x1: ampseal { pub port can_h \"CAN H\" pin 1; } }\n",
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
            "component m { connector x1: ghost { pub port can_h \"CAN H\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::undefined_connector_type"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn typed_connector_ports_are_component_ports() {
        // A port declared inside a typed connector is an ordinary flat
        // component port: wires reach it, and its name collides with a
        // bare port of the same name.
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port a \"A\"; connector x1: ampseal { pub port b \"B\" pin 1; } wire red 1 [a, b]; }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));

        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { pub port a \"A\"; connector x1: ampseal { pub port a \"A\" pin 1; } }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_port"), "{:?}", codes(&p));
    }

    #[test]
    fn duplicate_connector_pin_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { connector x1: ampseal { pub port can_h \"CAN H\" pin 1; pub port can_l \"CAN L\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_pin"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn duplicate_pin_errors_on_inline_connectors_too() {
        let p = problems(&[(
            "main.wb",
            "component m { connector x1 \"X 2p\" { pub port a \"A\" pin 1; pub port b \"B\" pin 1; } }\n",
        )]);
        assert!(
            has(&p, "wirebug::duplicate_connector_pin"),
            "{:?}",
            codes(&p)
        );
    }

    #[test]
    fn one_port_can_occupy_multiple_pins_on_one_connector() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }\ncomponent m { connector x1: ampseal { pub port gnd \"GND\" pins [1, 2]; } }\n",
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
                    "use leaf from \"leaf.wb\";\ncomponent m {{ l: leaf; }}\nview harness \"H\" {{ {include} }}\n"
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
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview harness \"H\" { include l.hv at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "connector_type hv_2p \"HV 2p\" { }\ncomponent leaf { connector hv: hv_2p { pub port hv_pos \"HV+\" pin 1; } }\n",
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
        let p = harness_view("include l.hv at (0, 0) ports { west: hv_pos; }");
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn connector_on_schematic_include_errors() {
        let p = problems(&[
            (
                "main.wb",
                "use leaf from \"leaf.wb\";\ncomponent m { l: leaf; }\nview schematic \"S\" { include l.hv at (0, 0); }\n",
            ),
            (
                "leaf.wb",
                "component leaf { connector hv \"HV 2p\" { pub port hv_pos \"HV+\" pin 1; } }\n",
            ),
        ]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn pinout_include_resolves_subject_connector_instance() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }
            component m {
                connector x1: ampseal { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include x1 at (0, 0); }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn pinout_include_resolves_inline_subject_connector() {
        let p = problems(&[(
            "main.wb",
            "component m {
                connector x1 \"Legacy 1p\" { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include x1 at (0, 0); }\n",
        )]);
        assert!(p.is_empty(), "unexpected problems: {:?}", codes(&p));
    }

    #[test]
    fn pinout_unknown_connector_errors() {
        let p = problems(&[(
            "main.wb",
            "component m { } view pinout \"X1\" { include x1 at (0, 0); }\n",
        )]);
        assert!(has(&p, "wirebug::unknown_connector"), "{:?}", codes(&p));
    }

    #[test]
    fn pinout_include_with_instance_connector_form_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }
            component m {
                connector x1: ampseal { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include child.x1 at (0, 0); }\n",
        )]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    #[test]
    fn ports_block_on_pinout_include_errors() {
        let p = problems(&[(
            "main.wb",
            "connector_type ampseal \"AMPSEAL\" { }
            component m {
                connector x1: ampseal { pub port can_h \"CAN H\" pin 1; }
            }
            view pinout \"X1\" { include x1 at (0, 0) ports { west: can_h; } }\n",
        )]);
        assert!(has(&p, "wirebug::wrong_include_form"), "{:?}", codes(&p));
    }

    // --- `extend` fragments (split a component across files) ---

    /// Load + resolve a multi-file project and hand the registry to `check`.
    /// (Mirrors `problems`, but exposes the merged `Resolved`.)
    fn with_resolved(files: &[(&str, &str)], check: impl FnOnce(&Resolved)) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
        )
        .expect("write wirebug.toml");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let (project, load_problems) = load(&dir.path().join("main.wb"));
        assert!(load_problems.is_empty(), "load: {load_problems:?}");
        let project = project.expect("loads");
        check(&resolve(&project));
    }

    const LEAF: &str = "component leaf { pub port a \"A\"; pub port b \"B\"; }\n";

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
    fn fragment_wire_reaches_an_instance_from_another_fragment() {
        // `traction` wires to `pack`, declared only in `main`'s fragment.
        let p = problems(&[
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
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }

    #[test]
    fn wire_to_an_instance_in_no_fragment_is_unknown() {
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { pack: leaf \"Pack\"; }\n",
            ),
            (
                "traction.wb",
                "use leaf from \"leaf.wb\";\n\
                 extend vehicle { inv: leaf \"Inv\"; wire red 1 [ghost.a, inv.a]; }\n",
            ),
            ("leaf.wb", LEAF),
        ]);
        assert!(has(&p, "wirebug::unknown_instance"), "{:?}", codes(&p));
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

    #[test]
    fn two_plain_components_with_one_name_still_duplicate() {
        let p = problems(&[(
            "main.wb",
            "component dup { }\ncomponent dup { }\ncomponent root { d: dup; }\n",
        )]);
        assert!(has(&p, "wirebug::duplicate_type"), "{:?}", codes(&p));
    }

    #[test]
    fn nested_extend_is_rejected() {
        let p = problems(&[("main.wb", "component outer { extend inner { } }\n")]);
        assert!(has(&p, "wirebug::nested_extend"), "{:?}", codes(&p));
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

    #[test]
    fn fragment_view_includes_an_instance_from_another_fragment() {
        // A schematic in `traction.wb` includes `pack`, which lives in `main`.
        let p = problems(&[
            (
                "main.wb",
                "use vehicle from \"traction.wb\";\nuse leaf from \"leaf.wb\";\n\
                 component vehicle { pack: leaf \"Pack\"; }\n",
            ),
            (
                "traction.wb",
                "use leaf from \"leaf.wb\";\nextend vehicle { inv: leaf \"Inv\"; }\n\
                 view schematic \"Traction\" {\n\
                   include pack at (2, 2) ports { east: a; }\n\
                   include inv  at (8, 2) ports { west: a; }\n\
                 }\n",
            ),
            ("leaf.wb", LEAF),
        ]);
        assert!(p.is_empty(), "{:?}", codes(&p));
    }
}
