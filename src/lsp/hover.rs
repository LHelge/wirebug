//! Hover: describe the declaration the reference under the cursor names —
//! its kind and name, its label, its type (an instance's component, a
//! connector's connector type), its pins (a port's) and the file and line
//! it is declared on — as Markdown.
//!
//! The lookup is go-to-definition's ([`definition::with_target`]): the
//! same fresh resolve and span walk, so hover answers on exactly the
//! references a jump does.

use std::path::Path;

use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use crate::dsl::ast::Visibility;
use crate::dsl::ir::Pin;
use crate::dsl::project::{Overlay, Project};

use super::definition::{self, Connector, Target};
use super::line_index::LineIndex;

/// Answer one hover request: `doc` at `position`, with `text` as its live
/// buffer.
pub(crate) fn hover(
    overlay: &Overlay,
    doc: &Path,
    text: &str,
    position: Position,
) -> Option<Hover> {
    definition::with_target(overlay, doc, text, position, |project, target| {
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: target.hover_markdown(project)?,
            }),
            range: None,
        })
    })
}

impl Target<'_> {
    /// The hover text; `None` for a file, which has nothing to describe
    /// beyond the path already under the cursor.
    fn hover_markdown(&self, project: &Project) -> Option<String> {
        let card = match self {
            Target::File(_) => return None,
            Target::Component(def) => Card::new("component", def.name),
            Target::ConnectorType(ct) => {
                Card::new("connector_type", ct.name).label(Some(&ct.ast.description.node))
            }
            Target::Instance(facts) => Card::new("instance", facts.ast.name.node.as_str())
                .label(facts.ast.label.as_ref().map(|l| l.node.as_str()))
                .fact("type", code(facts.ast.type_name.node.as_str())),
            Target::Inline(facts) => {
                let mut card = Card::new("inline connector", facts.ast.name.node.as_str())
                    .label(facts.ast.label.as_ref().map(|l| l.node.as_str()));
                for half in &facts.ast.halves {
                    card = card.fact(half.key.node.as_str(), code(half.type_name.node.as_str()));
                }
                card
            }
            Target::InlineHalf { inline, half } => {
                let card = Card::new("inline connector", inline.ast.name.node.as_str())
                    .label(inline.ast.label.as_ref().map(|l| l.node.as_str()));
                match half {
                    Some(half) => {
                        card.fact(half.key.node.as_str(), code(half.type_name.node.as_str()))
                    }
                    None => card,
                }
            }
            Target::Port { owner, name, facts } => {
                let kind = match facts.visibility {
                    Visibility::Public => "pub port",
                    Visibility::Private => "port",
                };
                let mut card = Card::new(kind, &format!("{owner}.{name}")).label(Some(facts.label));
                if let Some(pins) = Pin::display_list(&facts.pins) {
                    let key = if facts.pins.len() == 1 { "pin" } else { "pins" };
                    card = card.fact(key, pins);
                }
                if let Some(connector) = facts.connector {
                    card = card.fact("connector", code(connector.name));
                }
                card
            }
            Target::Splice(splice) => Card::new("splice", splice.name.node.as_str())
                .label(splice.label.as_ref().map(|l| l.node.as_str())),
            Target::Connector(Connector::Typed(facts)) => {
                Card::new("connector", facts.ast.name.node.as_str())
                    .fact("type", code(facts.ast.type_name.node.as_str()))
            }
            Target::Connector(Connector::Block(conn)) => {
                Card::new("connector", conn.name.node.as_str())
                    .label(conn.description.as_ref().map(|d| d.node.as_str()))
            }
        };
        Some(card.into_markdown(&self.location(project)))
    }

    /// `` `file.wb`, line N `` for the declaration, the path relative to the
    /// project's root directory.
    fn location(&self, project: &Project) -> String {
        let span = self.span();
        let file = project.file(span.file);
        let root = project.file(project.root).path.parent();
        let path = root
            .and_then(|root| file.path.strip_prefix(root).ok())
            .unwrap_or(&file.path);
        let line = LineIndex::new(&file.src)
            .position(&file.src, span.start)
            .line
            + 1;
        format!("{}, line {line}", code(&path.display().to_string()))
    }
}

/// A hover card being built: a bold kind and code name heading, then a
/// list of `key: value` facts.
#[must_use]
struct Card {
    heading: String,
    facts: Vec<String>,
}

impl Card {
    fn new(kind: &str, name: &str) -> Self {
        Self {
            heading: format!("**{kind}** {}", code(name)),
            facts: Vec::new(),
        }
    }

    /// Add the quoted, escaped user label, when there is one.
    fn label(self, label: Option<&str>) -> Self {
        match label {
            Some(label) => self.fact("label", format!("\"{}\"", escape(label))),
            None => self,
        }
    }

    fn fact(mut self, key: &str, value: String) -> Self {
        self.facts.push(format!("{key}: {value}"));
        self
    }

    /// The Markdown: the heading, then the facts and `location` as a list.
    fn into_markdown(self, location: &str) -> String {
        let items: String = self
            .facts
            .iter()
            .map(|fact| format!("\n- {fact}"))
            .collect();
        format!("{}\n{items}\n- defined in {location}", self.heading)
    }
}

/// `text` as inline code. Names are identifiers and paths, which carry no
/// backticks, so a single fence always closes.
fn code(text: &str) -> String {
    format!("`{text}`")
}

/// Backslash-escape every ASCII punctuation character, so a user label
/// renders literally instead of as Markdown.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `files` into a temp project and hover at the `‸` marker in
    /// `doc`, returning the Markdown shown.
    fn hover_at(files: &[(&str, &str)], doc: &str) -> Option<String> {
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
        let text = marked.replace('‸', "");
        let offset = marked.find('‸').expect("cursor marker");
        let position = LineIndex::new(&text).position(&text, offset);

        let hover = hover(&Overlay::default(), &dir.path().join(doc), &text, position)?;
        match hover.contents {
            HoverContents::Markup(markup) => {
                assert_eq!(markup.kind, MarkupKind::Markdown);
                Some(markup.value)
            }
            other => panic!("expected Markdown, got {other:?}"),
        }
    }

    const LEAF: (&str, &str) = (
        "parts/leaf.wb",
        "component Leaf {\n    pub port a \"A\";\n    connector j \"J 1p\" {\n        pub port b \"B\" pins [1, 2];\n    }\n}\n",
    );

    /// `main.wb` instancing `Leaf`, with `body` appended to its members.
    fn main_with(body: &str) -> String {
        format!(
            "use Leaf from \"parts/leaf.wb\";\ncomponent Root {{\n    l: Leaf \"Left leaf\";\n    port own \"Own\" pin 3;\n{body}\n}}\n"
        )
    }

    #[test]
    fn an_instance_shows_its_type_label_and_declaration() {
        let main = main_with("    wire red 1 [own, l‸.a];");
        assert_eq!(
            hover_at(&[LEAF, ("main.wb", &main)], "main.wb").as_deref(),
            Some(
                "**instance** `l`\n\n- label: \"Left leaf\"\n- type: `Leaf`\n- defined in `main.wb`, line 3"
            )
        );
    }

    #[test]
    fn a_child_port_shows_its_label_pins_and_connector() {
        let main = main_with("    wire red 1 [own, l.b‸];");
        assert_eq!(
            hover_at(&[LEAF, ("main.wb", &main)], "main.wb").as_deref(),
            Some(
                "**pub port** `Leaf.b`\n\n- label: \"B\"\n- pins: 1,2\n- connector: `j`\n- defined in `parts/leaf.wb`, line 4"
            )
        );
    }

    #[test]
    fn an_own_private_port_shows_its_pin() {
        let main = main_with("    wire red 1 [o‸wn, l.a];");
        assert_eq!(
            hover_at(&[LEAF, ("main.wb", &main)], "main.wb").as_deref(),
            Some(
                "**port** `Root.own`\n\n- label: \"Own\"\n- pin: 3\n- defined in `main.wb`, line 4"
            )
        );
    }

    #[test]
    fn a_type_reference_shows_the_component() {
        let main = main_with("").replace("l: Leaf", "l: Le‸af");
        assert_eq!(
            hover_at(&[LEAF, ("main.wb", &main)], "main.wb").as_deref(),
            Some("**component** `Leaf`\n\n- defined in `parts/leaf.wb`, line 1")
        );
    }

    #[test]
    fn a_pinout_include_shows_the_connector() {
        let main = "connector_type M2 \"Molex 2p\" { }\ncomponent Root {\n    connector j \"J 1p\" {\n        pub port b \"B\" pin 1;\n    }\n    connector k: M2 {\n        pub port c \"C\" pin 1;\n    }\n}\nview pinout \"P\" {\n    include MARK at (0, 0);\n}\n";
        assert_eq!(
            hover_at(&[("main.wb", &main.replace("MARK", "j‸"))], "main.wb").as_deref(),
            Some("**connector** `j`\n\n- label: \"J 1p\"\n- defined in `main.wb`, line 3")
        );
        assert_eq!(
            hover_at(&[("main.wb", &main.replace("MARK", "k‸"))], "main.wb").as_deref(),
            Some("**connector** `k`\n\n- type: `M2`\n- defined in `main.wb`, line 6")
        );
        let main = main.replace("MARK", "j").replace("k: M2", "k: M‸2");
        assert_eq!(
            hover_at(&[("main.wb", &main)], "main.wb").as_deref(),
            Some(
                "**connector_type** `M2`\n\n- label: \"Molex 2p\"\n- defined in `main.wb`, line 1"
            )
        );
    }

    const INLINE_MAIN_SRC: &str = "connector_type M2 \"M 2p\" { }\ncomponent Root {\n    pub port feed \"F\";\n    inline ic \"Branch\" {\n        male: M2;\n        port sig \"SIG\" pin 1;\n    }\n    wire red 1 [feed, ic.sig];\n}\nview harness \"H\" {\n    include ic.male at (0, 0);\n}\n";

    #[test]
    fn inline_references_show_the_inline_and_its_halves() {
        let with_cursor = |needle: &str, marked: &str| {
            let src = INLINE_MAIN_SRC.replacen(needle, marked, 1);
            assert_ne!(src, INLINE_MAIN_SRC, "cursor target `{needle}` exists");
            hover_at(&[("main.wb", &src)], "main.wb")
        };
        assert_eq!(
            with_cursor("[feed, ic.sig]", "[feed, i‸c.sig]").as_deref(),
            Some(
                "**inline connector** `ic`\n\n- label: \"Branch\"\n- male: `M2`\n- defined in `main.wb`, line 4"
            )
        );
        assert_eq!(
            with_cursor("[feed, ic.sig]", "[feed, ic.s‸ig]").as_deref(),
            Some("**port** `ic.sig`\n\n- label: \"SIG\"\n- pin: 1\n- defined in `main.wb`, line 6")
        );
        assert_eq!(
            with_cursor("include ic.male", "include ic.ma‸le").as_deref(),
            Some(
                "**inline connector** `ic`\n\n- label: \"Branch\"\n- male: `M2`\n- defined in `main.wb`, line 5"
            )
        );
    }

    #[test]
    fn a_splice_end_shows_the_splice() {
        let main = main_with(
            "    splice sp \"Joint\";\n    wire red 1 [l.a, s‸p];\n    wire red 1 [sp, own];",
        );
        assert_eq!(
            hover_at(&[LEAF, ("main.wb", &main)], "main.wb").as_deref(),
            Some("**splice** `sp`\n\n- label: \"Joint\"\n- defined in `main.wb`, line 5")
        );
    }

    #[test]
    fn a_label_renders_literally() {
        let main = main_with("    wire red 1 [own, l‸.a];").replace("Left leaf", "*not* `code`");
        let hover = hover_at(&[LEAF, ("main.wb", &main)], "main.wb").expect("hover");
        assert!(hover.contains(r#"- label: "\*not\* \`code\`""#), "{hover}");
    }

    #[test]
    fn a_path_or_plain_text_shows_nothing() {
        let main = main_with("").replace("parts/leaf.wb", "parts/le‸af.wb");
        assert_eq!(hover_at(&[LEAF, ("main.wb", &main)], "main.wb"), None);
        let main = main_with("").replace("component Root", "component Ro‸ot");
        assert_eq!(hover_at(&[LEAF, ("main.wb", &main)], "main.wb"), None);
    }
}
