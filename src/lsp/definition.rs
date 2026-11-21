//! Go-to-definition: map the reference under the cursor to the span of
//! the declaration it resolves to.
//!
//! Like diagnostics, each request re-runs the load/resolve pipeline with
//! the overlay shadowing the disk — projects are small and the pipeline
//! already runs per keystroke, so a fresh resolve per jump is cheap and
//! never stale. The resolution itself is a span walk over the [`Resolved`]
//! registry: find the reference whose span contains the offset, then
//! return the [`Target`] it names, whose name span is the jump target
//! (spans are file-tagged, so the target carries its own file). Hover
//! (`lsp/hover.rs`) describes the same [`Target`].

use std::path::Path;

use lsp_types::{Location, Position, Range};

use crate::dsl::ast::{self, Member};
use crate::dsl::ir::{ConnectorName, InstanceName, PortName, SpliceName};
use crate::dsl::project::{self, Overlay, Project};
use crate::dsl::resolve::{self, DefId, Resolved};
use crate::dsl::span::{FileId, Span};

use super::line_index::LineIndex;
use super::uri;

/// The declaration a reference resolves to.
pub(super) enum Target<'r> {
    /// A `use`/`fragment` path: the top of the file it names.
    File(FileId),
    /// A component type: its root `component` when split with `extend`.
    Component(&'r resolve::DefInfo<'r>),
    ConnectorType(&'r resolve::ConnectorTypeInfo<'r>),
    Instance(&'r resolve::InstFacts<'r>),
    Inline(&'r resolve::InlineFacts<'r>),
    /// One housing half of an inline (`include ic.male`); `half` is `None`
    /// when the inline doesn't declare it.
    InlineHalf {
        inline: &'r resolve::InlineFacts<'r>,
        half: Option<&'r ast::InlineHalfDecl>,
    },
    /// A port, with the name of what owns it: a component, or an inline.
    Port {
        owner: &'r str,
        name: &'r PortName,
        facts: &'r resolve::PortFacts<'r>,
    },
    Splice(&'r ast::Splice),
    /// A connector designator: a typed connector instance, or an inline
    /// `connector` block.
    Connector(Connector<'r>),
}

/// The two declarations a connector designator can name.
pub(super) enum Connector<'r> {
    Typed(&'r resolve::ConnectorInstFacts<'r>),
    Block(&'r ast::Connector),
}

impl Target<'_> {
    /// The span of the declaration's name: where go-to-definition lands.
    pub(super) fn span(&self) -> Span {
        match self {
            Target::File(file) => Span {
                file: *file,
                start: 0,
                end: 0,
            },
            Target::Component(def) => def.ast.name.span,
            Target::ConnectorType(ct) => ct.ast.name.span,
            Target::Instance(facts) => facts.ast.name.span,
            Target::Inline(facts) => facts.ast.name.span,
            Target::InlineHalf { inline, half } => {
                half.map_or(inline.ast.name.span, |h| h.key.span)
            }
            Target::Port { facts, .. } => facts.span,
            Target::Splice(splice) => splice.name.span,
            Target::Connector(Connector::Typed(facts)) => facts.ast.name.span,
            Target::Connector(Connector::Block(conn)) => conn.name.span,
        }
    }
}

/// Answer one go-to-definition request: `doc` at `position`, with `text`
/// as its live buffer.
pub(crate) fn goto_definition(
    overlay: &Overlay,
    doc: &Path,
    text: &str,
    position: Position,
) -> Option<Location> {
    with_target(overlay, doc, text, position, |project, target| {
        let span = target.span();
        let target = project.file(span.file);
        let index = LineIndex::new(&target.src);
        Some(Location {
            uri: uri::to_uri(&target.path)?,
            range: Range {
                start: index.position(&target.src, span.start),
                end: index.position(&target.src, span.end),
            },
        })
    })
}

/// Load and resolve `doc`'s project (the overlay shadowing the disk, `text`
/// as `doc`'s live buffer) and hand the target of the reference at
/// `position` to `answer`.
pub(super) fn with_target<R>(
    overlay: &Overlay,
    doc: &Path,
    text: &str,
    position: Position,
    answer: impl FnOnce(&Project, Target<'_>) -> Option<R>,
) -> Option<R> {
    let offset = LineIndex::new(text).offset(text, position);
    let parent = doc.parent().unwrap_or(Path::new("."));
    let entry = project::discover(Some(parent)).unwrap_or_else(|_| doc.to_path_buf());
    let (project, _) = project::load_with(&entry, overlay);
    let project = project?;
    let canonical = doc.canonicalize().unwrap_or_else(|_| doc.to_path_buf());
    let file = FileId(project.files.iter().position(|f| f.path == canonical)?);
    let resolved = resolve::resolve(&project);
    answer(&project, find(&resolved, file, offset)?)
}

/// The declaration the reference at `offset` in `file` names, if the
/// offset sits on something that resolves.
pub(super) fn find<'r>(
    resolved: &'r Resolved<'r>,
    file: FileId,
    offset: usize,
) -> Option<Target<'r>> {
    let hit = |span: Span| span.file == file && span.start <= offset && offset <= span.end;

    // `use` and `fragment` paths: the path string jumps to the top of the
    // file it names.
    let project = resolved.project;
    for import in project.files[file.0].ast.imports() {
        if hit(import.path().span) {
            return project
                .import_target(file, &import.path().node)
                .map(Target::File);
        }
    }

    // `use` names: the definition they import.
    for use_decl in &project.files[file.0].ast.uses {
        if !hit(use_decl.name.span) {
            continue;
        }
        let tf = project.import_target(file, &use_decl.path.node)?;
        let name = use_decl.name.node.as_str();
        if let Some(d) = resolved
            .defs
            .iter()
            .find(|d| d.file == tf && d.parent.is_none() && d.name == name)
        {
            return Some(Target::Component(d));
        }
        return resolved
            .connector_types
            .iter()
            .find(|ct| ct.file == tf && ct.name == name)
            .map(Target::ConnectorType);
    }

    // References inside this file's definitions.
    for d in 0..resolved.defs.len() {
        let def = &resolved.defs[d];
        if def.file != file {
            continue;
        }

        // An `extend` fragment's name points at its root `component`.
        if hit(def.ast.name.span) {
            let canonical = resolved.groups.canonical(d);
            if canonical != d {
                return Some(Target::Component(&resolved.defs[canonical]));
            }
            continue;
        }

        for facts in def.instances.values() {
            if hit(facts.ast.type_name.span) {
                return facts.type_id.map(|t| Target::Component(&resolved.defs[t]));
            }
        }
        for facts in def.connectors.values() {
            if hit(facts.ast.type_name.span) {
                return facts
                    .type_id
                    .map(|t| Target::ConnectorType(&resolved.connector_types[t]));
            }
        }
        // An inline's `male:`/`female:` type refs point at the connector type.
        for facts in def.inlines.values() {
            for half in &facts.ast.halves {
                if hit(half.type_name.span) {
                    let type_id = match half.key.node.as_str() {
                        "male" => facts.male,
                        "female" => facts.female,
                        _ => None,
                    };
                    return type_id.map(|t| Target::ConnectorType(&resolved.connector_types[t]));
                }
            }
        }

        for member in &def.ast.members {
            let endpoints: Vec<&ast::Endpoint> = match member {
                Member::Wire(wire) => wire.endpoints.iter().collect(),
                Member::Cable(cable) => cable.endpoints().collect(),
                _ => continue,
            };
            for ep in endpoints {
                if let Some(inst) = &ep.instance {
                    if hit(inst.span) {
                        if let Some(inline) = find_inline(resolved, d, inst.node.as_str()) {
                            return Some(Target::Inline(inline));
                        }
                        return find_instance(resolved, d, inst.node.as_str())
                            .map(Target::Instance);
                    }
                    if hit(ep.port.span) {
                        if let Some(inline) = find_inline(resolved, d, inst.node.as_str()) {
                            let port = PortName::from(ep.port.node.as_str());
                            return inline.ports.get_key_value(&port).map(|(name, facts)| {
                                Target::Port {
                                    owner: inline.ast.name.node.as_str(),
                                    name,
                                    facts,
                                }
                            });
                        }
                        let t = find_instance(resolved, d, inst.node.as_str())?.type_id?;
                        return find_port(resolved, t, ep.port.node.as_str());
                    }
                } else if hit(ep.port.span) {
                    // A bare name is a splice when the component declares
                    // one of that name, else an own port.
                    let name = ep.port.node.as_str();
                    if let Some(splice) = resolved.splice(d, &SpliceName::from(name)) {
                        return Some(Target::Splice(splice));
                    }
                    return find_port(resolved, d, name);
                }
            }
        }
    }

    // References inside views (bound to their subject component).
    for binding in &resolved.views {
        let view = binding.ast;
        let subject = binding.subject;
        // `for <Component>` jumps to the component (its root `component`
        // when it is split with `extend`).
        if let Some(named) = &view.subject
            && hit(named.span)
        {
            return subject.map(|s| Target::Component(&resolved.defs[s]));
        }
        for ep in &view.enclosure {
            if hit(ep.port.span) {
                return find_port(resolved, subject?, ep.port.node.as_str());
            }
        }
        for inc in &view.includes {
            if hit(inc.instance.span) {
                let subject = subject?;
                // A pinout include names a connector on the subject itself.
                if view.kind.node.as_str() == "pinout" {
                    return find_connector(resolved, subject, inc.instance.node.as_str())
                        .map(Target::Connector);
                }
                // A harness include may place one of the subject's splices.
                let splice = SpliceName::from(inc.instance.node.as_str());
                if let Some(splice) = resolved.splice(subject, &splice) {
                    return Some(Target::Splice(splice));
                }
                if let Some(inline) = find_inline(resolved, subject, inc.instance.node.as_str()) {
                    return Some(Target::Inline(inline));
                }
                return find_instance(resolved, subject, inc.instance.node.as_str())
                    .map(Target::Instance);
            }
            let inst_type = || {
                find_instance(resolved, subject?, inc.instance.node.as_str())
                    .and_then(|f| f.type_id)
            };
            if let Some(conn) = &inc.connector
                && hit(conn.span)
            {
                // An inline include's dot segment is a housing half: jump to
                // the matching `male:`/`female:` line (the inline's name
                // when that half isn't declared).
                if let Some(inline) = find_inline(resolved, subject?, inc.instance.node.as_str()) {
                    let half = inline
                        .ast
                        .halves
                        .iter()
                        .find(|h| h.key.node.as_str() == conn.node.as_str());
                    return Some(Target::InlineHalf { inline, half });
                }
                return find_connector(resolved, inst_type()?, conn.node.as_str())
                    .map(Target::Connector);
            }
            for placement in &inc.ports {
                if hit(placement.port.span) {
                    return find_port(resolved, inst_type()?, placement.port.node.as_str());
                }
            }
        }
    }

    None
}

/// The instance declaration for `name` in `d`'s merged component.
fn find_instance<'r>(
    resolved: &'r Resolved<'r>,
    d: DefId,
    name: &str,
) -> Option<&'r resolve::InstFacts<'r>> {
    let name = InstanceName::from(name);
    resolved
        .fragments(d)
        .into_iter()
        .find_map(|frag| resolved.defs[frag].instances.get(&name))
}

/// The inline connector declaration for `name` in `d`'s merged component.
fn find_inline<'r>(
    resolved: &'r Resolved<'r>,
    d: DefId,
    name: &str,
) -> Option<&'r resolve::InlineFacts<'r>> {
    let name = InstanceName::from(name);
    resolved
        .fragments(d)
        .into_iter()
        .find_map(|frag| resolved.defs[frag].inlines.get(&name))
}

/// The port declaration for `name` in `d`'s merged component.
fn find_port<'r>(resolved: &'r Resolved<'r>, d: DefId, name: &str) -> Option<Target<'r>> {
    let name = PortName::from(name);
    let owner = resolved.defs[d].name;
    resolved.fragments(d).into_iter().find_map(|frag| {
        resolved.defs[frag]
            .ports
            .get_key_value(&name)
            .map(|(name, facts)| Target::Port { owner, name, facts })
    })
}

/// The connector declaration for designator `name` in `d`'s merged
/// component: a typed connector instance or an inline `connector` block.
fn find_connector<'r>(resolved: &'r Resolved<'r>, d: DefId, name: &str) -> Option<Connector<'r>> {
    let typed = ConnectorName::from(name);
    for frag in resolved.fragments(d) {
        let def = &resolved.defs[frag];
        if let Some(facts) = def.connectors.get(&typed) {
            return Some(Connector::Typed(facts));
        }
        for member in &def.ast.members {
            if let Member::Connector(conn) = member
                && conn.name.node.as_str() == name
            {
                return Some(Connector::Block(conn));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `files` into a temp project and resolve the definition at the
    /// `‸` marker in `doc` (whose marked content replaces its disk file).
    /// Returns `(target file name, target text)` — the text the target
    /// span covers.
    fn target_of(files: &[(&str, &str)], doc: &str) -> Option<(String, String)> {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
        )
        .expect("write manifest");
        for (name, body) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create dirs");
            }
            std::fs::write(path, body.replace('‸', "")).expect("write file");
        }
        let (_, marked) = files.iter().find(|(name, _)| *name == doc).expect("doc");
        let offset = marked.find('‸').expect("cursor marker");

        let entry = dir.path().join("main.wb");
        let (project, _) = project::load(&entry);
        let project = project.expect("loads");
        let doc_path = dir.path().join(doc).canonicalize().expect("canonical");
        let file = FileId(
            project
                .files
                .iter()
                .position(|f| f.path == doc_path)
                .expect("doc in project"),
        );
        let resolved = resolve::resolve(&project);

        let span = find(&resolved, file, offset)?.span();
        let target = &project.files[span.file.0];
        let name = target
            .path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        Some((name, target.src[span.start..span.end].to_string()))
    }

    const LEAF: (&str, &str) = (
        "leaf.wb",
        "component Leaf {\n    pub port a \"A\";\n    connector j \"J 1p\" {\n        pub port b \"B\" pin 1;\n    }\n}\n",
    );

    #[test]
    fn use_name_jumps_to_the_definition() {
        let main = (
            "main.wb",
            "use Le‸af from \"leaf.wb\";\ncomponent Root { l: Leaf; }\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "Leaf".into()))
        );
    }

    #[test]
    fn use_path_jumps_to_the_file() {
        let main = (
            "main.wb",
            "use Leaf from \"le‸af.wb\";\ncomponent Root { l: Leaf; }\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "".into()))
        );
    }

    #[test]
    fn fragment_path_jumps_to_the_file() {
        let frag = (
            "systems/frag.wb",
            "extend Root {\n    pub port x \"X\";\n}\n",
        );
        let main = (
            "main.wb",
            "fragment \"systems/fr‸ag.wb\";\ncomponent Root { }\n",
        );
        assert_eq!(
            target_of(&[frag, main], "main.wb"),
            Some(("frag.wb".into(), "".into()))
        );
    }

    #[test]
    fn fragment_keyword_resolves_to_nothing() {
        let frag = ("frag.wb", "extend Root { }\n");
        let main = ("main.wb", "frag‸ment \"frag.wb\";\ncomponent Root { }\n");
        assert_eq!(target_of(&[frag, main], "main.wb"), None);
    }

    #[test]
    fn instance_type_jumps_to_the_component() {
        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root { l: Le‸af; }\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "Leaf".into()))
        );
    }

    #[test]
    fn endpoint_instance_and_port_jump_to_their_declarations() {
        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root {\n    l: Leaf;\n    pub port own \"O\";\n    wire red 1 [own, l‸.a];\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("main.wb".into(), "l".into()))
        );

        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root {\n    l: Leaf;\n    pub port own \"O\";\n    wire red 1 [own, l.a‸];\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "a".into()))
        );

        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root {\n    l: Leaf;\n    pub port own \"O\";\n    wire red 1 [ow‸n, l.a];\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("main.wb".into(), "own".into()))
        );
    }

    #[test]
    fn shield_ends_jump_like_wire_endpoints() {
        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root {\n    l: Leaf;\n    pub port own \"O\";\n    cable c {\n        wire red 1 [own, l.a];\n        twisted { wire red 1 [own, l.a]; wire red 1 [l.a, own]; shield [l.a‸]; }\n    }\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "a".into()))
        );

        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root {\n    l: Leaf;\n    pub port own \"O\";\n    cable c {\n        wire red 1 [own, l.a];\n        shield \"S\" [ow‸n];\n    }\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("main.wb".into(), "own".into()))
        );
    }

    #[test]
    fn a_splice_end_and_include_jump_to_the_splice() {
        let main = |marked: &str| {
            format!(
                "use Leaf from \"leaf.wb\";\ncomponent Root {{\n    l: Leaf;\n    pub port own \"O\";\n    splice sp \"Joint\";\n    wire red 1 [l.b, {}];\n    wire red 1 [sp, own];\n}}\nview harness \"H\" {{\n    include l.j at (0, 0);\n    include {} at (10, 0);\n}}\n",
                if marked == "end" { "s‸p" } else { "sp" },
                if marked == "include" { "s‸p" } else { "sp" },
            )
        };
        for marked in ["end", "include"] {
            let main = main(marked);
            assert_eq!(
                target_of(&[LEAF, ("main.wb", &main)], "main.wb"),
                Some(("main.wb".into(), "sp".into())),
                "{marked}"
            );
        }
    }

    #[test]
    fn extend_name_jumps_to_the_root_component() {
        let frag = ("frag.wb", "extend Ro‸ot {\n    pub port x \"X\";\n}\n");
        let main = (
            "main.wb",
            "use Root from \"frag.wb\";\ncomponent Root { }\n",
        );
        assert_eq!(
            target_of(&[frag, main], "frag.wb"),
            Some(("main.wb".into(), "Root".into()))
        );
    }

    #[test]
    fn cross_fragment_endpoint_resolves_through_the_merge() {
        // A wire in the fragment reaches an instance declared in main.wb.
        let frag = (
            "frag.wb",
            "use Leaf from \"leaf.wb\";\nextend Root {\n    wire red 1 [l‸.a, l.a];\n}\n",
        );
        let main = (
            "main.wb",
            "use Root from \"frag.wb\";\nuse Leaf from \"leaf.wb\";\ncomponent Root { l: Leaf; }\n",
        );
        assert_eq!(
            target_of(&[LEAF, frag, main], "frag.wb"),
            Some(("main.wb".into(), "l".into()))
        );
    }

    #[test]
    fn include_instance_and_connector_jump_to_their_declarations() {
        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root { l: Leaf; }\nview harness \"H\" {\n    include l.j‸ at (0, 0);\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "j".into()))
        );

        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root { l: Leaf; }\nview schematic \"S\" {\n    include l‸ at (0, 0) ports {\n        west: a;\n    }\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("main.wb".into(), "l".into()))
        );
    }

    #[test]
    fn include_port_placement_jumps_to_the_port() {
        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root { l: Leaf; }\nview schematic \"S\" {\n    include l at (0, 0) ports {\n        west: a‸;\n    }\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "a".into()))
        );
    }

    #[test]
    fn pinout_include_jumps_to_the_subject_connector() {
        let main = (
            "main.wb",
            "component Root {\n    connector j \"J 1p\" {\n        pub port b \"B\" pin 1;\n    }\n}\nview pinout \"P\" {\n    include j‸ at (0, 0);\n}\n",
        );
        assert_eq!(
            target_of(&[main], "main.wb"),
            Some(("main.wb".into(), "j".into()))
        );
    }

    const INLINE_MAIN_SRC: &str = "connector_type M2 \"M 2p\" { }\ncomponent Root {\n    pub port feed \"F\";\n    inline ic {\n        male: M2;\n        port sig \"SIG\" pin 1;\n    }\n    wire red 1 [feed, ic.sig];\n}\nview harness \"H\" {\n    include ic.male at (0, 0);\n}\n";

    #[test]
    fn inline_references_jump_to_their_declarations() {
        let with_cursor = |needle: &str, marked: &str| {
            let src = INLINE_MAIN_SRC.replacen(needle, marked, 1);
            assert_ne!(src, INLINE_MAIN_SRC, "cursor target `{needle}` exists");
            target_of(&[("main.wb", &src)], "main.wb")
        };

        // The half's type ref jumps to the connector_type.
        assert_eq!(
            with_cursor("male: M2;", "male: M‸2;"),
            Some(("main.wb".into(), "M2".into()))
        );
        // An endpoint's instance segment jumps to the inline declaration.
        assert_eq!(
            with_cursor("wire red 1 [feed, ic.sig];", "wire red 1 [feed, i‸c.sig];"),
            Some(("main.wb".into(), "ic".into()))
        );
        // Its port segment jumps to the inline's port.
        assert_eq!(
            with_cursor("wire red 1 [feed, ic.sig];", "wire red 1 [feed, ic.s‸ig];"),
            Some(("main.wb".into(), "sig".into()))
        );
        // The include's base jumps to the inline; the half segment to the
        // matching `male:` line.
        assert_eq!(
            with_cursor("include ic.male", "include i‸c.male"),
            Some(("main.wb".into(), "ic".into()))
        );
        assert_eq!(
            with_cursor("include ic.male", "include ic.ma‸le"),
            Some(("main.wb".into(), "male".into()))
        );
    }

    #[test]
    fn view_subject_jumps_to_the_component() {
        let main = (
            "main.wb",
            "use Leaf from \"leaf.wb\";\ncomponent Root { l: Leaf; }\nview pinout \"P\" for Le‸af {\n    include j at (0, 0);\n}\n",
        );
        assert_eq!(
            target_of(&[LEAF, main], "main.wb"),
            Some(("leaf.wb".into(), "Leaf".into()))
        );
    }

    #[test]
    fn view_subject_in_a_fragment_jumps_to_the_root_component() {
        let frag = (
            "frag.wb",
            "extend Root {\n    pub port x \"X\";\n}\nview schematic \"S\" for Ro‸ot { }\n",
        );
        let main = (
            "main.wb",
            "use Root from \"frag.wb\";\ncomponent Root { }\n",
        );
        assert_eq!(
            target_of(&[frag, main], "frag.wb"),
            Some(("main.wb".into(), "Root".into()))
        );
    }

    #[test]
    fn plain_text_resolves_to_nothing() {
        let main = ("main.wb", "component Ro‸ot { }\n");
        assert_eq!(target_of(&[main], "main.wb"), None);
    }
}
