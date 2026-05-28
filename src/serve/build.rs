//! Build the in-memory [`Site`] from the `.wb` project — the shared render
//! step the watcher re-runs on every change.

use std::path::Path;
use std::time::Duration;

use indexmap::IndexMap;
use miette::{GraphicalReportHandler, GraphicalTheme};

use super::livereload::ERROR_PAGE_SCRIPT;
use super::state::Site;
use crate::dsl::{self, CheckReport};
use crate::render::{index_html, render_views};

/// The outcome of one build: the servable site plus a count of what `check`
/// reported, so the watcher can log a summary to the console. The browser
/// always gets the full detail; these counts keep the terminal honest.
pub(crate) struct Build {
    pub(crate) site: Site,
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
}

impl Build {
    /// Log a one-line summary at a severity matching the worst problem.
    /// `elapsed` is the rebuild duration (absent for the initial build).
    pub(crate) fn log(&self, elapsed: Option<Duration>) {
        let timing = match elapsed {
            Some(d) => format!(" in {}ms", d.as_millis()),
            None => String::new(),
        };
        if self.errors > 0 {
            tracing::error!(
                "build{timing} found {} error(s), {} warning(s) — serving diagnostics",
                self.errors,
                self.warnings,
            );
        } else if self.warnings > 0 {
            tracing::warn!("built{timing} with {} warning(s)", self.warnings);
        } else {
            tracing::info!("built{timing}");
        }
    }
}

/// Run the full pipeline and produce a servable site.
///
/// On a clean project this is the rendered views plus the live-reloading
/// index. If `check` reports errors — or rendering fails — the site instead
/// serves a diagnostics page (with the same live-reload script), so the
/// browser shows what's wrong and recovers automatically once it's fixed.
pub(crate) fn build_site(target: Option<&Path>) -> Build {
    let report = dsl::check_project(target);
    let counts = report.counts();

    if counts.errors > 0 {
        return Build {
            site: error_site(&report),
            errors: counts.errors,
            warnings: counts.warnings,
        };
    }
    let Some(design) = &report.design else {
        // No errors yet a missing design shouldn't happen, but serve the
        // diagnostics rather than panic.
        return Build {
            site: error_site(&report),
            errors: counts.errors.max(1),
            warnings: counts.warnings,
        };
    };

    // A render/index failure is a build error in its own right; show it on the
    // page and count it so the console summary reflects the failure.
    // `serve` always emits self-contained SVGs (built-in style + identity
    // stamp); embed-mode is for static export, not the live browser.
    let render = render_views(design, false)
        .map_err(|e| format!("render failed: {e}"))
        .and_then(|views| {
            index_html(&views, design.manifest.as_ref(), true)
                .map_err(|e| format!("HTML index failed: {e}"))
                .map(|html| (views, html))
        });
    match render {
        Ok((views, html)) => {
            let svgs = views
                .into_iter()
                .map(|view| (view.filename, view.svg))
                .collect();
            Build {
                site: Site {
                    index_html: html,
                    svgs,
                },
                errors: counts.errors,
                warnings: counts.warnings,
            }
        }
        Err(message) => Build {
            site: message_site(&message),
            errors: 1,
            warnings: counts.warnings,
        },
    }
}

/// A site whose index lists the project's diagnostics as text.
fn error_site(report: &CheckReport) -> Site {
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor());
    let mut text = String::new();
    for problem in &report.problems {
        let _ = handler.render_report(&mut text, problem);
    }
    message_site(&text)
}

/// A site whose only content is a preformatted diagnostic message, with the
/// live-reload script so it recovers once the project builds.
fn message_site(message: &str) -> Site {
    let body = escape_html(message);
    let index_html = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n  <meta charset=\"utf-8\">\n  \
         <title>wirebug — problems</title>\n  <style>body {{ font-family: \
         ui-monospace, monospace; margin: 2rem; }} pre {{ white-space: pre-wrap; }}\
         </style>\n</head>\n<body>\n  <h1>wirebug found problems</h1>\n  \
         <pre>{body}</pre>\n  {ERROR_PAGE_SCRIPT}\n</body>\n</html>\n",
    );
    Site {
        index_html,
        svgs: IndexMap::new(),
    }
}

/// Escape the characters unsafe in HTML text so diagnostic spans render
/// verbatim inside the `<pre>` block.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_main() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic_project/main.wb")
    }

    #[test]
    fn clean_project_yields_views_and_an_index() {
        let build = build_site(Some(&fixture_main()));
        assert_eq!(build.errors, 0);
        assert!(!build.site.svgs.is_empty(), "expected rendered SVGs");
        assert!(build.site.index_html.contains("wirebug views"));
        assert!(build.site.index_html.contains("new WebSocket"));
    }

    #[test]
    fn missing_project_yields_a_diagnostics_page() {
        let build = build_site(Some(Path::new("/nonexistent/main.wb")));
        assert!(build.errors > 0, "a missing project is an error");
        assert!(build.site.svgs.is_empty());
        assert!(build.site.index_html.contains("found problems"));
        // The error page still live-reloads so it recovers on fix.
        assert!(build.site.index_html.contains("new WebSocket"));
    }
}
