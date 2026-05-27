//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`.

use crate::dsl::ir::{Design, Instance, View};
use crate::error::{Error, Result};

pub mod geometry;
pub mod schematic;

/// One rendered view: the SVG document, the file name it should be
/// written to (a slug of the view title, without a directory), and the
/// human title it was rendered from (for the HTML index).
#[derive(Debug)]
#[must_use]
pub struct RenderedView {
    pub title: String,
    pub filename: String,
    pub svg: String,
}

/// Render every view in `design` to SVG, in declaration order.
///
/// Each view documents a component *type*; it is rendered against the
/// first instance of that type in the design (the root for a top-level
/// view). A view naming a `kind` this build can't render, or a subject
/// type with no instance, is an error.
pub fn render_views(design: &Design) -> Result<Vec<RenderedView>> {
    design
        .views
        .iter()
        .map(|view| {
            let subject = subject_instance(design, view)?;
            let svg = match view.kind.as_str() {
                "schematic" => schematic::SchematicRenderer.render(design, subject, view)?,
                other => return Err(Error::UnknownViewKind(other.to_string())),
            };
            Ok(RenderedView {
                title: view.title.clone(),
                filename: format!("{}.svg", slug(&view.title)),
                svg,
            })
        })
        .collect()
}

/// File name for the HTML index page written alongside the per-view SVGs.
pub const INDEX_FILENAME: &str = "index.html";

const INDEX_STYLE: &str = "\
    body { font-family: system-ui, sans-serif; margin: 2rem auto; max-width: 70rem; \
color: #1a1a1a; background: #fafafa; }\n\
    h1 { font-weight: 600; }\n\
    section { background: #fff; border: 1px solid #ddd; border-radius: 6px; \
margin: 1.5rem 0; padding: 1rem 1.5rem; }\n\
    section h2 { margin: 0 0 .75rem; font-size: 1.1rem; }\n\
    img { max-width: 100%; height: auto; }\n\
    p.empty { color: #777; }\n";

/// Build a self-contained HTML index that embeds every rendered view, in
/// render order, each under its title. The SVGs are referenced by their
/// sibling file names, so the page expects to live in the same directory.
#[must_use]
pub fn index_html(views: &[RenderedView]) -> String {
    let mut body = String::new();
    if views.is_empty() {
        body.push_str("    <p class=\"empty\">This design has no views.</p>\n");
    }
    for view in views {
        let title = escape_html(&view.title);
        let src = escape_html(&view.filename);
        body.push_str(&format!(
            "    <section>\n      <h2>{title}</h2>\n      \
             <img src=\"{src}\" alt=\"{title}\">\n    </section>\n",
        ));
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  \
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  \
         <title>wirebug views</title>\n  <style>\n{INDEX_STYLE}  </style>\n</head>\n\
         <body>\n  <h1>wirebug views</h1>\n{body}</body>\n</html>\n",
    )
}

/// Escape the five characters that are unsafe in HTML text and double-quoted
/// attribute values. View titles and slugged file names both flow into the
/// index, and titles are author-supplied.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The instance a view renders against: the first one whose type matches
/// the view's subject.
fn subject_instance<'d>(design: &'d Design, view: &View) -> Result<&'d Instance> {
    design
        .instances
        .values()
        .find(|inst| inst.type_name == view.subject)
        .ok_or_else(|| Error::UnknownSubject {
            subject: view.subject.to_string(),
        })
}

/// Turn a human title into a safe file-name stem: lowercase, runs of
/// non-alphanumeric characters collapsed to a single underscore, ends
/// trimmed. An empty result falls back to `view`.
fn slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut pending_sep = false;
    for ch in title.chars() {
        if ch.is_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.extend(ch.to_lowercase());
        } else {
            pending_sep = true;
        }
    }
    if out.is_empty() {
        "view".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collapses_and_lowercases() {
        assert_eq!(slug("HV System Overview"), "hv_system_overview");
        assert_eq!(slug("  Pack / Detail  "), "pack_detail");
        assert_eq!(slug("!!!"), "view");
    }

    #[test]
    fn index_references_each_view_by_title_and_file() {
        let views = vec![
            RenderedView {
                title: "HV Overview".to_string(),
                filename: "hv_overview.svg".to_string(),
                svg: String::new(),
            },
            RenderedView {
                title: "Pack Detail".to_string(),
                filename: "pack_detail.svg".to_string(),
                svg: String::new(),
            },
        ];
        let html = index_html(&views);
        assert!(html.contains("<h2>HV Overview</h2>"));
        assert!(html.contains("<img src=\"hv_overview.svg\" alt=\"HV Overview\">"));
        assert!(html.contains("<img src=\"pack_detail.svg\" alt=\"Pack Detail\">"));
    }

    #[test]
    fn index_escapes_author_supplied_titles() {
        let views = vec![RenderedView {
            title: "A & B <hv>".to_string(),
            filename: "a_b_hv.svg".to_string(),
            svg: String::new(),
        }];
        let html = index_html(&views);
        assert!(html.contains("<h2>A &amp; B &lt;hv&gt;</h2>"));
        assert!(!html.contains("<hv>"));
    }

    #[test]
    fn index_notes_a_design_with_no_views() {
        assert!(index_html(&[]).contains("no views"));
    }
}
