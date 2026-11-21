//! Writing a design's views to disk, in each of `render`'s output modes.
//!
//! `wirebug render` checks the project, then hands the design to
//! [`Output::write`]; what lands in the output directory, and what the
//! command reports, is decided here so it can be exercised on a hand-built
//! design whose views fail.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::pdf::PdfExporter;
use super::png::PngRasterizer;
use super::{
    EMBED_MANIFEST_FILENAME, EMBED_STYLESHEET_FILENAME, INDEX_FILENAME, RenderedView, SvgMode,
    ViewOutcome, embed_manifest, embed_stylesheet, index_html, pdf_filename, render_views,
};
use crate::dsl::ir::Design;
use crate::error::{Error, Result, ViewError};

/// The file format each view is written in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewFormat {
    Svg,
    /// Rasterised at `scale` × the SVG's intrinsic size.
    Png {
        scale: f32,
    },
}

/// What `render` writes for a design.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Output {
    /// One file per view plus [`INDEX_FILENAME`] — or, with `embed`,
    /// host-styled SVGs plus [`EMBED_MANIFEST_FILENAME`] and
    /// [`EMBED_STYLESHEET_FILENAME`] in place of the index.
    Views { format: ViewFormat, embed: bool },
    /// One A4 PDF with a page per view, named by [`pdf_filename`].
    Pdf,
}

impl Output {
    /// How the views' SVGs carry their presentation in this output: plain
    /// for the PDF (its page furniture carries the title and identity at a
    /// fixed size), naked for embedding, self-contained otherwise.
    fn svg_mode(self) -> SvgMode {
        match self {
            Self::Pdf => SvgMode::Plain,
            Self::Views { embed: true, .. } => SvgMode::Embed,
            Self::Views { embed: false, .. } => SvgMode::Standalone,
        }
    }

    /// Render every view of `design` and write the output into `out_dir`,
    /// creating it. A view that fails to render (or to rasterise or
    /// convert) is left out and named in [`Written::failures`]; the others
    /// are still written. Only a disk error, or the index or manifest
    /// failing to build, stops the write.
    pub fn write(self, design: &Design, out_dir: &Path) -> Result<Written> {
        let views = render_views(design, self.svg_mode());
        let total = views.len();
        fs::create_dir_all(out_dir).map_err(|source| Error::Write {
            path: out_dir.to_path_buf(),
            source,
        })?;
        let manifest = design.manifest.as_ref();

        let (label, entry, failures) = match self {
            // One artifact for the whole design, not per-view files plus a
            // sidecar. Its pages are the views that rendered; the export
            // decides whether that leaves a document to write.
            Self::Pdf => {
                let export = PdfExporter::new().to_pdf(views, manifest);
                let filename = pdf_filename(manifest);
                let entry = match export.document() {
                    Some(bytes) => Entry::Written(write_file(out_dir, &filename, bytes)?),
                    None => Entry::NoPdf(filename),
                };
                ("pdf", entry, export.failures)
            }
            Self::Views { format, embed } => {
                let views = format.write_views(views, out_dir)?;
                // The embed manifest lists only the views that were
                // written, so a host never looks for a missing file; the
                // HTML index shows a failed view's error in its place.
                let entry = if embed {
                    let manifest = embed_manifest(views.iter().flatten(), manifest);
                    let json = serde_json::to_string_pretty(&manifest)?;
                    let path = write_file(out_dir, EMBED_MANIFEST_FILENAME, json.as_bytes())?;
                    write_file(
                        out_dir,
                        EMBED_STYLESHEET_FILENAME,
                        embed_stylesheet().as_bytes(),
                    )?;
                    path
                } else {
                    let index = index_html(&views, manifest, false)?;
                    write_file(out_dir, INDEX_FILENAME, index.as_bytes())?
                };
                let failures = views
                    .into_iter()
                    .filter_map(std::result::Result::err)
                    .collect();
                (format.label(), Entry::Written(entry), failures)
            }
        };
        Ok(Written {
            failures,
            total,
            label,
            out_dir: out_dir.to_path_buf(),
            entry,
        })
    }
}

impl ViewFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png { .. } => "png",
        }
    }

    /// Write every rendered view to `out_dir` in this format, returning the
    /// views as the index and manifest should list them (file names in this
    /// format). Failed views pass through, and a view that fails to
    /// rasterise joins them.
    fn write_views(self, views: Vec<ViewOutcome>, out_dir: &Path) -> Result<Vec<ViewOutcome>> {
        match self {
            Self::Svg => {
                for view in views.iter().flatten() {
                    write_file(out_dir, &view.filename, view.svg.as_bytes())?;
                }
                Ok(views)
            }
            Self::Png { scale } => {
                let rasterizer = PngRasterizer::new();
                views
                    .into_iter()
                    .map(|view| {
                        let view = match view {
                            Ok(view) => view,
                            Err(failure) => return Ok(Err(failure)),
                        };
                        let bytes = match rasterizer.to_png(&view.svg, scale) {
                            Ok(bytes) => bytes,
                            Err(cause) => return Ok(Err(view.failed(cause))),
                        };
                        let filename = Path::new(&view.filename)
                            .with_extension("png")
                            .to_string_lossy()
                            .into_owned();
                        write_file(out_dir, &filename, &bytes)?;
                        Ok(Ok(RenderedView {
                            title: view.title,
                            filename,
                            kind: view.kind,
                            svg: String::new(),
                        }))
                    })
                    .collect()
            }
        }
    }
}

/// Write `contents` to `filename` in `out_dir`, returning the file's path.
fn write_file(out_dir: &Path, filename: &str, contents: &[u8]) -> Result<PathBuf> {
    let path = out_dir.join(filename);
    match fs::write(&path, contents) {
        Ok(()) => Ok(path),
        Err(source) => Err(Error::Write { path, source }),
    }
}

/// What [`Output::write`] left in the output directory. Its `Display` is
/// the one-line summary `render` prints.
#[derive(Debug)]
#[must_use]
pub struct Written {
    /// Each view that was left out, in view order.
    pub failures: Vec<ViewError>,
    /// How many views the design has.
    total: usize,
    /// The output's format, as the summary names it.
    label: &'static str,
    out_dir: PathBuf,
    entry: Entry,
}

/// The file that leads to the written views.
#[derive(Debug)]
enum Entry {
    /// The HTML index, the embed manifest or the PDF, at this path.
    Written(PathBuf),
    /// Every view failed, so the PDF of this name was not written.
    NoPdf(String),
}

impl fmt::Display for Written {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = match &self.entry {
            Entry::Written(path) => path,
            Entry::NoPdf(filename) => {
                return write!(f, "no view rendered — {filename} not written");
            }
        };
        let rendered = self.total - self.failures.len();
        write!(f, "rendered {rendered} ")?;
        if rendered != self.total {
            write!(f, "of {} ", self.total)?;
        }
        write!(
            f,
            "view(s) as {} to {} ({})",
            self.label,
            self.out_dir.display(),
            path.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::tests::design_with_views;

    /// Write `titles` (a view titled `Lost …` fails) as `output` into a
    /// fresh directory; return it, what was written and the directory's
    /// sorted file names.
    fn write(output: Output, titles: &[&str]) -> (tempfile::TempDir, Written, Vec<String>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("out");
        let written = output
            .write(&design_with_views(titles), &out)
            .expect("writes");
        let mut entries: Vec<String> = fs::read_dir(&out)
            .expect("out dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        (tmp, written, entries)
    }

    const SVG: Output = Output::Views {
        format: ViewFormat::Svg,
        embed: false,
    };

    fn failed(written: &Written) -> Vec<String> {
        written.failures.iter().map(ToString::to_string).collect()
    }

    const LOST: &str = "view \"Lost\": view subject \"gone\" has no instance in the design";

    #[test]
    fn every_view_written_is_summarised_without_a_count_of_failures() {
        let (tmp, written, entries) = write(SVG, &["Overview", "Detail"]);
        assert_eq!(entries, ["detail.svg", "index.html", "overview.svg"]);
        assert!(written.failures.is_empty());
        let out = tmp.path().join("out");
        assert_eq!(
            written.to_string(),
            format!(
                "rendered 2 view(s) as svg to {} ({})",
                out.display(),
                out.join("index.html").display()
            )
        );
    }

    #[test]
    fn svg_output_writes_the_views_that_render_and_shows_the_failure_in_the_index() {
        let (tmp, written, entries) = write(SVG, &["Overview", "Lost", "Detail"]);
        assert_eq!(entries, ["detail.svg", "index.html", "overview.svg"]);
        assert_eq!(failed(&written), [LOST]);
        let out = tmp.path().join("out");
        assert_eq!(
            written.to_string(),
            format!(
                "rendered 2 of 3 view(s) as svg to {} ({})",
                out.display(),
                out.join("index.html").display()
            )
        );

        // The index shows the failure where the drawing would be.
        let index = fs::read_to_string(out.join("index.html")).expect("index");
        assert!(index.contains("<img src=\"overview.svg\""));
        assert!(index.contains("<img src=\"detail.svg\""));
        assert!(index.contains("<section class=\"failed\">"));
        assert!(index.contains("<h2>Lost</h2>"));
        assert!(index.contains("has no instance in the design"));
    }

    #[test]
    fn png_output_rasterises_the_views_that_render() {
        let png = Output::Views {
            format: ViewFormat::Png { scale: 2.0 },
            embed: false,
        };
        let (tmp, written, entries) = write(png, &["Overview", "Lost", "Detail"]);
        assert_eq!(entries, ["detail.png", "index.html", "overview.png"]);
        assert_eq!(failed(&written), [LOST]);
        assert!(
            written
                .to_string()
                .starts_with("rendered 2 of 3 view(s) as png")
        );
        let index = fs::read_to_string(tmp.path().join("out/index.html")).expect("index");
        assert!(index.contains("<img src=\"overview.png\""));
        assert!(index.contains("<h2>Lost</h2>"));
    }

    #[test]
    fn png_output_refuses_a_view_too_large_to_rasterise_and_writes_the_others() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let out = tmp.path().join("out");
        let mut design = design_with_views(&["Overview", "Huge", "Detail"]);
        // A million grid steps east: a canvas of millions of units, whose
        // pixmap would be terabytes.
        design.views[1].includes[1].x = 1_000_000.0;
        let png = Output::Views {
            format: ViewFormat::Png { scale: 2.0 },
            embed: false,
        };
        let written = png.write(&design, &out).expect("writes");
        let mut entries: Vec<String> = fs::read_dir(&out)
            .expect("out dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, ["detail.png", "index.html", "overview.png"]);
        let [failure] = written.failures.as_slice() else {
            panic!("one failure: {:?}", written.failures);
        };
        assert_eq!(failure.title, "Huge");
        assert!(matches!(
            *failure.cause,
            crate::error::ViewErrorKind::CanvasTooLarge { .. }
        ));
        assert!(
            failure
                .to_string()
                .ends_with("more than the 8192 units a view may span on a side"),
            "{failure}"
        );

        // SVG output costs nothing per unit, and draws it.
        let svg = SVG.write(&design, &tmp.path().join("svg")).expect("writes");
        assert!(svg.failures.is_empty(), "{:?}", svg.failures);
    }

    #[test]
    fn embed_output_lists_only_the_views_it_wrote() {
        let embed = Output::Views {
            format: ViewFormat::Svg,
            embed: true,
        };
        let (tmp, written, entries) = write(embed, &["Overview", "Lost", "Detail"]);
        assert_eq!(
            entries,
            ["detail.svg", "manifest.json", "overview.svg", "wirebug.css"]
        );
        assert_eq!(failed(&written), [LOST]);
        let out = tmp.path().join("out");
        assert!(
            written
                .to_string()
                .ends_with(&format!("({})", out.join("manifest.json").display()))
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out.join("manifest.json")).expect("manifest"))
                .expect("json");
        let titles: Vec<&str> = manifest["views"]
            .as_array()
            .expect("views")
            .iter()
            .map(|v| v["title"].as_str().expect("title"))
            .collect();
        assert_eq!(titles, ["Overview", "Detail"]);
    }

    #[test]
    fn pdf_output_pages_only_the_views_that_render() {
        let (tmp, written, entries) = write(Output::Pdf, &["Overview", "Lost", "Detail"]);
        assert_eq!(entries, ["views.pdf"]);
        assert_eq!(failed(&written), [LOST]);
        let out = tmp.path().join("out");
        assert_eq!(
            written.to_string(),
            format!(
                "rendered 2 of 3 view(s) as pdf to {} ({})",
                out.display(),
                out.join("views.pdf").display()
            )
        );
        let pdf = fs::read(out.join("views.pdf")).expect("pdf");
        assert!(pdf.windows(8).any(|w| w == b"/Count 2"), "two pages");
        assert!(pdf.windows(7).any(|w| w == b"(2 / 2)"), "numbered of two");
    }

    #[test]
    fn pdf_output_writes_no_pdf_when_every_view_fails() {
        let (_tmp, written, entries) = write(Output::Pdf, &["Lost"]);
        assert!(entries.is_empty(), "nothing is written: {entries:?}");
        assert_eq!(failed(&written), [LOST]);
        assert_eq!(
            written.to_string(),
            "no view rendered — views.pdf not written"
        );
    }
}
