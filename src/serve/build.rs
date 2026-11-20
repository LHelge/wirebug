//! Build the in-memory [`Site`] from the `.wb` project — the shared render
//! step the watcher re-runs on every change.

use std::path::Path;
use std::time::Duration;

use indexmap::IndexMap;
use miette::{GraphicalReportHandler, GraphicalTheme};

use super::livereload::ERROR_PAGE_SCRIPT;
use super::state::Site;
use crate::dsl::CheckReport;
use crate::error::ViewError;
use crate::render::{SvgMode, index_html, render_views};

/// The outcome of one build: the servable site plus what `check` reported
/// and which views failed to render, so the watcher can log a summary to
/// the console. The browser always gets the full detail; these keep the
/// terminal honest.
pub(crate) struct Build {
    pub(crate) site: Site,
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    /// Views that passed `check` but failed to render; the page shows each
    /// one's error in place of its drawing.
    pub(crate) failed_views: Vec<ViewError>,
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
        } else if !self.failed_views.is_empty() {
            tracing::error!(
                "built{timing}, but {} view(s) failed to render",
                self.failed_views.len(),
            );
            for failure in &self.failed_views {
                tracing::error!("{failure}");
            }
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
/// index; a view that fails to render shows its error in its place while
/// the others stay visible. If `check` reports errors — or the index fails
/// to render — the site instead serves a diagnostics page (with the same
/// live-reload script), so the browser shows what's wrong and recovers
/// automatically once it's fixed.
pub(crate) fn build_site(target: Option<&Path>) -> Build {
    let report = crate::check_project(target);
    let counts = report.counts();

    if counts.errors > 0 {
        return Build {
            site: error_site(&report),
            errors: counts.errors,
            warnings: counts.warnings,
            failed_views: Vec::new(),
        };
    }
    let Some(design) = &report.design else {
        // No errors yet a missing design shouldn't happen, but serve the
        // diagnostics rather than panic.
        return Build {
            site: error_site(&report),
            errors: counts.errors.max(1),
            warnings: counts.warnings,
            failed_views: Vec::new(),
        };
    };

    // `serve` always emits self-contained SVGs (built-in style + identity
    // stamp); embed-mode is for static export, not the live browser.
    let views = render_views(design, SvgMode::Standalone);
    let index_html = match index_html(&views, design.manifest.as_ref(), true) {
        Ok(html) => html,
        // An index failure is a build error in its own right; show it on
        // the page and count it so the console summary reflects it.
        Err(e) => {
            return Build {
                site: message_site(&format!("HTML index failed: {e}")),
                errors: 1,
                warnings: counts.warnings,
                failed_views: Vec::new(),
            };
        }
    };
    let mut svgs = IndexMap::new();
    let mut failed_views = Vec::new();
    for view in views {
        match view {
            Ok(view) => {
                svgs.insert(view.filename, view.svg);
            }
            Err(failure) => failed_views.push(failure),
        }
    }
    Build {
        site: Site { index_html, svgs },
        errors: counts.errors,
        warnings: counts.warnings,
        failed_views,
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
    fn a_view_whose_includes_collide_yields_a_diagnostics_page() {
        // `render` would fail to route this fixture's "Crowded" view; the
        // check finds it first, so the page shows the diagnostics instead.
        let main = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/crowded_views_project/main.wb");
        let build = build_site(Some(&main));
        assert!(build.errors > 0, "colliding includes are errors");
        assert!(build.site.svgs.is_empty());
        assert!(build.site.index_html.contains("include_too_close"));
        assert!(build.site.index_html.contains("overlapping_includes"));
    }

    #[test]
    fn a_view_that_fails_to_render_shows_its_error_and_the_others_stay() {
        // The fixture's "Fine Grid" view passes `check` but not `render`.
        let main = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/render_failure_project/main.wb");
        let build = build_site(Some(&main));
        assert_eq!(build.errors, 0, "the project checks clean");
        let [failure] = build.failed_views.as_slice() else {
            panic!("one failed view: {:?}", build.failed_views);
        };
        assert_eq!(failure.title, "Fine Grid");
        // The other views are served and shown; the failure takes its
        // view's place on the page, which still live-reloads.
        let served: Vec<&str> = build.site.svgs.keys().map(String::as_str).collect();
        assert_eq!(served, ["overview.svg", "harness.svg"]);
        let html = &build.site.index_html;
        assert!(html.contains("<img src=\"overview.svg\""));
        assert!(html.contains("<img src=\"harness.svg\""));
        assert!(html.contains("<h2>Fine Grid</h2>"));
        assert!(html.contains("grid step 5 is too small"));
        assert!(html.contains("new WebSocket"));
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
