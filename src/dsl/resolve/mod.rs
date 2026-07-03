//! Name resolution: build a registry of every component definition,
//! bind type/instance/port/include references, and report the resolution
//! errors (undefined type, unresolved import, duplicates, bad endpoints,
//! private-port access, unknown view includes).
//!
//! The result is a [`Resolved`] registry that elaboration consumes. Each
//! definition (top-level or nested) gets a [`DefId`]; ports are flattened
//! across connectors (which are grouping metadata, not namespaces, so
//! port names are unique per component).
//!
//! This file holds the data model ([`Resolved`], the `*Facts` structs) and
//! the [`resolve`] driver; the passes live one-per-file, in the order the
//! driver runs them:
//!
//! - [`register`] — walk every definition's AST into [`DefInfo`] facts,
//!   reporting intra-definition duplicates;
//! - [`scopes`] — per-file type scopes from own definitions + `use` imports
//!   (where same-name `extend` collisions become merges);
//! - [`merge`] — the [`MergeGroups`] machinery: canonical fragments,
//!   orphans, cross-fragment duplicates;
//! - [`refs`] — the flat-namespace lookups and the reference-binding
//!   passes (instance types, connector types, inline halves, endpoints);
//! - [`views`] — bind each view to its subject and validate its includes.

use std::collections::HashMap;

use indexmap::IndexMap;

use crate::dsl::ast::{self, Item};
use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{ConnectorName, InstanceName, Pin, PortName};
use crate::dsl::project::Project;
use crate::dsl::span::{FileId, Span};

mod merge;
mod refs;
mod register;
mod scopes;
mod views;

pub use merge::MergeGroups;

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

/// An inline (mid-harness) connector member: its own port set (not
/// flattened into the component's ports — wire endpoints address it like
/// an instance) and the resolved connector type of each declared half.
pub struct InlineFacts<'a> {
    pub ast: &'a ast::Inline,
    pub ports: IndexMap<PortName, PortFacts<'a>>,
    pub male: Option<ConnectorTypeId>,
    pub female: Option<ConnectorTypeId>,
}

impl InlineFacts<'_> {
    /// Whether a half was *declared* (even if its type failed to resolve —
    /// an unresolved type already reported and shouldn't double-report as
    /// an undeclared half).
    pub fn declares(&self, half: &str) -> bool {
        self.ast.halves.iter().any(|h| h.key.node.as_str() == half)
    }
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
    /// Inline (mid-harness) connectors, sharing the instance namespace.
    pub inlines: IndexMap<InstanceName, InlineFacts<'a>>,
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
    r.resolve_inline_halves(&connector_type_scope);
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

/// Shared harness for the resolve submodules' test suites: tiny
/// project-on-disk builders and problem-code helpers.
#[cfg(test)]
pub(super) mod testkit {
    use super::{Resolved, resolve};
    use crate::dsl::diagnostics::Problem;
    use crate::dsl::project::load;

    pub(crate) const LEAF: &str = "component leaf { pub port a \"A\"; pub port b \"B\"; }\n";

    /// Write `files` into a temp dir and resolve, returning every problem
    /// (loading + resolution). Auto-writes a minimal `wirebug.toml` so a
    /// missing-manifest problem doesn't leak into resolve-focused tests.
    pub(super) fn problems(files: &[(&str, &str)]) -> Vec<Problem> {
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

    pub(super) fn codes(problems: &[Problem]) -> Vec<String> {
        use miette::Diagnostic;
        problems
            .iter()
            .filter_map(|p| p.code().map(|c| c.to_string()))
            .collect()
    }

    pub(super) fn has(problems: &[Problem], code: &str) -> bool {
        codes(problems).iter().any(|c| c == code)
    }

    /// A one-component project with an inline connector: `m` holds a `leaf`
    /// instance `l`, the given inline body as `inline ic { ... }`, and the
    /// given extra members/views appended after it.
    pub(super) fn inline_project(inline_body: &str, rest: &str) -> Vec<Problem> {
        problems(&[
            (
                "main.wb",
                &format!(
                    "use leaf from \"leaf.wb\";\nconnector_type M2 \"M 2p\" {{ }}\nconnector_type F2 \"F 2p\" {{ }}\ncomponent m {{ l: leaf; inline ic {{ {inline_body} }} {rest} }}\n"
                ),
            ),
            ("leaf.wb", "component leaf { pub port a \"A\"; }\n"),
        ])
    }

    /// Load + resolve a multi-file project and hand the registry to `check`.
    /// (Mirrors `problems`, but exposes the merged `Resolved`.)
    pub(super) fn with_resolved(files: &[(&str, &str)], check: impl FnOnce(&Resolved)) {
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
}

#[cfg(test)]
mod tests {
    use super::testkit::codes;
    use crate::dsl::project::load;
    use crate::dsl::resolve::resolve;

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
}
