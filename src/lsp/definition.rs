//! Go-to-definition: map the reference under the cursor to the span of
//! the declaration it resolves to.
//!
//! Like diagnostics, each request re-runs the load/resolve pipeline with
//! the overlay shadowing the disk — projects are small and the pipeline
//! already runs per keystroke, so a fresh resolve per jump is cheap and
//! never stale. The resolution itself is a span walk over the [`Resolved`]
//! registry: find the reference whose span contains the offset, then
//! return its target's name span (spans are file-tagged, so the target
//! carries its own file).

use std::path::Path;

use lsp_types::{Location, Position, Range};

use crate::dsl::ast::{self, Member};
use crate::dsl::ir::{ConnectorName, InstanceName, PortName};
use crate::dsl::project::{self, Overlay};
use crate::dsl::resolve::{self, DefId, Resolved};
use crate::dsl::span::{FileId, Span};

use super::line_index::LineIndex;
use super::uri;

/// Answer one go-to-definition request: `doc` at `position`, with `text`
/// as its live buffer.
pub(crate) fn goto_definition(
    overlay: &Overlay,
    doc: &Path,
    text: &str,
    position: Position,
) -> Option<Location> {
    let offset = LineIndex::new(text).offset(text, position);
    let parent = doc.parent().unwrap_or(Path::new("."));
    let entry = project::discover(Some(parent)).unwrap_or_else(|_| doc.to_path_buf());
    let (project, _) = project::load_with(&entry, overlay);
    let project = project?;
    let canonical = doc.canonicalize().unwrap_or_else(|_| doc.to_path_buf());
    let file = FileId(project.files.iter().position(|f| f.path == canonical)?);
    let resolved = resolve::resolve(&project);

    let span = find(&resolved, file, offset)?;
    let target = &project.files[span.file.0];
    let index = LineIndex::new(&target.src);
    Some(Location {
        uri: uri::to_uri(&target.path)?,
        range: Range {
            start: index.position(&target.src, span.start),
            end: index.position(&target.src, span.end),
        },
    })
}

/// The definition-site span for the reference at `offset` in `file`, if
/// the offset sits on something that resolves.
fn find(resolved: &Resolved, file: FileId, offset: usize) -> Option<Span> {
    let hit = |span: Span| span.file == file && span.start <= offset && offset <= span.end;

    // `use` names and paths.
    for use_decl in &resolved.project.files[file.0].ast.uses {
        if !hit(use_decl.name.span) && !hit(use_decl.path.span) {
            continue;
        }
        let dir = resolved.project.files[file.0].path.parent()?;
        let target = dir.join(use_decl.path.node.as_str()).canonicalize().ok()?;
        let tf = FileId(
            resolved
                .project
                .files
                .iter()
                .position(|f| f.path == target)?,
        );
        if hit(use_decl.path.span) {
            // The path string itself: jump to the top of the file.
            return Some(Span {
                file: tf,
                start: 0,
                end: 0,
            });
        }
        let name = use_decl.name.node.as_str();
        if let Some(d) = resolved
            .defs
            .iter()
            .find(|d| d.file == tf && d.parent.is_none() && d.name == name)
        {
            return Some(d.ast.name.span);
        }
        return resolved
            .connector_types
            .iter()
            .find(|ct| ct.file == tf && ct.name == name)
            .map(|ct| ct.ast.name.span);
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
                return Some(resolved.defs[canonical].ast.name.span);
            }
            continue;
        }

        for facts in def.instances.values() {
            if hit(facts.ast.type_name.span) {
                return facts.type_id.map(|t| resolved.defs[t].ast.name.span);
            }
        }
        for facts in def.connectors.values() {
            if hit(facts.ast.type_name.span) {
                return facts
                    .type_id
                    .map(|t| resolved.connector_types[t].ast.name.span);
            }
        }

        for member in &def.ast.members {
            let wires: Vec<&ast::Wire> = match member {
                Member::Wire(wire) => vec![wire],
                Member::Cable(cable) => cable.wires().collect(),
                _ => continue,
            };
            for wire in wires {
                for ep in &wire.endpoints {
                    if let Some(inst) = &ep.instance {
                        if hit(inst.span) {
                            return find_instance(resolved, d, inst.node.as_str())
                                .map(|f| f.ast.name.span);
                        }
                        if hit(ep.port.span) {
                            let t = find_instance(resolved, d, inst.node.as_str())?.type_id?;
                            return find_port(resolved, t, ep.port.node.as_str());
                        }
                    } else if hit(ep.port.span) {
                        return find_port(resolved, d, ep.port.node.as_str());
                    }
                }
            }
        }
    }

    // References inside views (bound to their subject component).
    for binding in &resolved.views {
        let view = binding.ast;
        let subject = binding.subject;
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
                    return find_connector(resolved, subject, inc.instance.node.as_str());
                }
                return find_instance(resolved, subject, inc.instance.node.as_str())
                    .map(|f| f.ast.name.span);
            }
            let inst_type = || {
                find_instance(resolved, subject?, inc.instance.node.as_str())
                    .and_then(|f| f.type_id)
            };
            if let Some(conn) = &inc.connector
                && hit(conn.span)
            {
                return find_connector(resolved, inst_type()?, conn.node.as_str());
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
fn find_instance<'a>(
    resolved: &'a Resolved,
    d: DefId,
    name: &str,
) -> Option<&'a resolve::InstFacts<'a>> {
    let name = InstanceName::from(name);
    resolved
        .fragments(d)
        .into_iter()
        .find_map(|frag| resolved.defs[frag].instances.get(&name))
}

/// The port declaration span for `name` in `d`'s merged component.
fn find_port(resolved: &Resolved, d: DefId, name: &str) -> Option<Span> {
    let name = PortName::from(name);
    resolved
        .fragments(d)
        .into_iter()
        .find_map(|frag| resolved.defs[frag].ports.get(&name).map(|p| p.span))
}

/// The connector declaration span for designator `name` in `d`'s merged
/// component: a typed connector instance or an inline `connector` block.
fn find_connector(resolved: &Resolved, d: DefId, name: &str) -> Option<Span> {
    let typed = ConnectorName::from(name);
    for frag in resolved.fragments(d) {
        let def = &resolved.defs[frag];
        if let Some(facts) = def.connectors.get(&typed) {
            return Some(facts.ast.name.span);
        }
        for member in &def.ast.members {
            if let Member::Connector(conn) = member
                && conn.name.node.as_str() == name
            {
                return Some(conn.name.span);
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

        let span = find(&resolved, file, offset)?;
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

    #[test]
    fn plain_text_resolves_to_nothing() {
        let main = ("main.wb", "component Ro‸ot { }\n");
        assert_eq!(target_of(&[main], "main.wb"), None);
    }
}
