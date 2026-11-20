//! SVG → single-file A4 PDF export for `render --pdf`.
//!
//! Each rendered view becomes one A4 page: the SVG is parsed with
//! [`svg2pdf`]'s own `usvg` (a separate crate version from the resvg pair
//! the PNG path uses — the two never interoperate), converted to a
//! unit-sized Form XObject via [`svg2pdf::to_chunk`], and placed on a page
//! whose orientation follows the view's aspect ratio. The multi-page
//! document is assembled by hand with [`pdf_writer`]. Fixed-size page
//! furniture replaces the in-SVG title and stamp a standalone render
//! carries: a title header, plus a footer with the identity stamp and a
//! `<page> / <total>` number.
//!
//! Text is outlined to paths at conversion time (`embed_text: false`), so
//! labels survive any PDF viewer without font embedding — but only if the
//! font database is populated at *parse* time, exactly as in
//! [`super::png::PngRasterizer`]; an empty fontdb drops text silently.

use std::collections::HashMap;

use pdf_writer::{Chunk, Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

use super::stamp::stamp_text;
use super::{RenderedView, ViewOutcome};
use crate::dsl::manifest::Manifest;
use crate::error::{ViewError, ViewErrorKind, ViewResult};

/// A4 short edge (210 mm) in PDF points.
const A4_SHORT_PT: f32 = 595.276;
/// A4 long edge (297 mm) in PDF points.
const A4_LONG_PT: f32 = 841.89;
/// Page margin (15 mm) in PDF points.
const MARGIN_PT: f32 = 42.52;
/// Cap on blowing up small views, so a lone tiny box doesn't fill a page.
const MAX_UPSCALE: f32 = 2.0;

/// Footer text size in points. The footer uses the built-in Courier so
/// its fixed metrics (0.6 em per glyph) make alignment exact without
/// embedding a font.
const FOOTER_SIZE: f32 = 7.0;
/// Courier glyph advance as a fraction of the font size.
const COURIER_ADVANCE: f32 = 0.6;
/// Footer text baseline, from the page's bottom edge — inside the margin,
/// clear of the view content.
const FOOTER_BASELINE: f32 = MARGIN_PT / 2.0;
/// Footer fill gray (0 black – 1 white), matching the SVG stamp's #666.
const FOOTER_GRAY: f32 = 0.4;

/// Header (view title) size in points, matching the SVG title's weight.
const HEADER_SIZE: f32 = 12.0;
/// Header baseline above the content area's top edge — left-aligned to
/// the margin, so no glyph metrics are needed (Helvetica-Bold, built-in).
const HEADER_GAP: f32 = 8.0;

/// Exports rendered views into one multi-page A4 PDF, holding a font
/// database loaded once up front.
///
/// The fontdb setup mirrors [`super::png::PngRasterizer::new`]; it cannot
/// be shared because svg2pdf's `usvg` is a different crate version than
/// resvg's, so the two `Options` types are distinct.
pub struct PdfExporter {
    options: svg2pdf::usvg::Options<'static>,
}

impl PdfExporter {
    /// Build an exporter with the system fonts loaded.
    #[must_use]
    pub fn new() -> Self {
        use svg2pdf::usvg::fontdb::{Family, Query};

        let mut options = svg2pdf::usvg::Options::default();
        let db = options.fontdb_mut();
        db.load_system_fonts();

        // Same generic-family fallback as the PNG path: point `sans-serif`
        // at a present face so labels survive on hosts without the default
        // family name.
        let resolves = db
            .query(&Query {
                families: &[Family::SansSerif],
                ..Query::default()
            })
            .is_some();
        if !resolves {
            let fallback = db
                .faces()
                .next()
                .and_then(|f| f.families.first())
                .map(|(name, _)| name.clone());
            if let Some(name) = fallback {
                db.set_sans_serif_family(name);
            }
        }

        Self { options }
    }

    /// Export `views` (as [`super::render_views`] returns them) into one
    /// PDF, one A4 page per rendered view in order.
    ///
    /// `manifest` supplies the document title (`<name> v<version>`) and
    /// the footer's project-identity stamp; pass `None` for synthetic
    /// designs without one. Views are expected plain
    /// ([`super::SvgMode::Plain`]) — the page header carries the view
    /// title and the footer the identity stamp plus a `<page> / <total>`
    /// number, so every page reads the same regardless of view scale.
    ///
    /// A view that failed to render, or fails to convert, is left out — the
    /// pages that remain number `1 / n` among themselves — and named in
    /// [`PdfExport::failures`], in view order.
    pub fn to_pdf(&self, views: Vec<ViewOutcome>, manifest: Option<&Manifest>) -> PdfExport {
        let mut alloc = Ref::new(1);
        let catalog_id = alloc.bump();
        let page_tree_id = alloc.bump();
        let footer_font_id = alloc.bump();
        let header_font_id = alloc.bump();
        let mut pdf = Pdf::new();

        // Page-furniture fonts, both from the PDF's built-in set and
        // WinAnsi-encoded (the stamp's `·` separator is Latin-1 0xB7):
        // Courier for the footer (fixed metrics make alignment exact),
        // Helvetica-Bold for the left-aligned title header.
        pdf.type1_font(footer_font_id)
            .base_font(Name(b"Courier"))
            .encoding_predefined(Name(b"WinAnsiEncoding"));
        pdf.type1_font(header_font_id)
            .base_font(Name(b"Helvetica-Bold"))
            .encoding_predefined(Name(b"WinAnsiEncoding"));

        // Convert every view before laying out any page, so the footer's
        // page total counts only the views that made it.
        let mut failures = Vec::new();
        let mut pages = Vec::new();
        for view in views {
            match view.and_then(|view| self.convert(view, &mut alloc)) {
                Ok(page) => pages.push(page),
                Err(failure) => failures.push(failure),
            }
        }

        let mut page_ids = Vec::with_capacity(pages.len());
        for (index, page) in pages.iter().enumerate() {
            pdf.extend(&page.chunk);

            // Resources are per-page, so one XObject name suffices.
            let content_id = alloc.bump();
            let mut content = Content::new();
            content.save_state();
            content.transform(page.layout.matrix);
            content.x_object(Name(b"V"));
            content.restore_state();
            header(&mut content, &page.layout, &page.title);
            footer(&mut content, &page.layout, index + 1, pages.len(), manifest);
            pdf.stream(content_id, &content.finish());

            let page_id = alloc.bump();
            let mut writer = pdf.page(page_id);
            writer.media_box(Rect::new(0.0, 0.0, page.layout.width, page.layout.height));
            writer.parent(page_tree_id);
            writer.contents(content_id);
            let mut resources = writer.resources();
            resources.x_objects().pair(Name(b"V"), page.svg_id);
            let mut fonts = resources.fonts();
            fonts.pair(Name(b"F"), footer_font_id);
            fonts.pair(Name(b"H"), header_font_id);
            fonts.finish();
            resources.finish();
            writer.finish();
            page_ids.push(page_id);
        }

        pdf.catalog(catalog_id).pages(page_tree_id);
        pdf.pages(page_tree_id)
            .kids(page_ids.iter().copied())
            .count(page_ids.len() as i32);

        let info_id = alloc.bump();
        let mut info = pdf.document_info(info_id);
        if let Some(m) = manifest {
            info.title(TextStr(&format!("{} v{}", m.name, m.version)));
        }
        info.producer(TextStr(&format!("wirebug {}", super::stamp::APP_VERSION)));
        info.finish();

        PdfExport {
            bytes: pdf.finish(),
            pages: pages.len(),
            failures,
        }
    }

    /// Convert one view's SVG into a Form XObject chunk numbered from
    /// `alloc`, with the page geometry that fits it; a failure names the
    /// view.
    fn convert(&self, view: RenderedView, alloc: &mut Ref) -> Result<Page, ViewError> {
        match self.convert_svg(&view.svg, alloc) {
            Ok((layout, chunk, svg_id)) => Ok(Page {
                title: view.title,
                layout,
                chunk,
                svg_id,
            }),
            Err(cause) => Err(view.failed(cause)),
        }
    }

    /// The conversion half of [`Self::convert`], before the view is named.
    fn convert_svg(&self, svg: &str, alloc: &mut Ref) -> ViewResult<(PageLayout, Chunk, Ref)> {
        let tree = svg2pdf::usvg::Tree::from_str(svg, &self.options)
            .map_err(|e| ViewErrorKind::SvgParse(e.to_string()))?;
        let conversion = svg2pdf::ConversionOptions {
            embed_text: false,
            ..Default::default()
        };
        let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, conversion)
            .map_err(|e| ViewErrorKind::PdfConvert(e.to_string()))?;
        let mut map = HashMap::new();
        let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
        let svg_id = map.get(&svg_ref).copied().ok_or_else(|| {
            ViewErrorKind::PdfConvert("svg root object missing after renumber".into())
        })?;
        Ok((PageLayout::fit(tree.size()), chunk, svg_id))
    }
}

/// What [`PdfExporter::to_pdf`] produced: the document, how many pages it
/// holds, and the views left out because they failed to render or
/// convert. The bytes are reached only through [`Self::document`], which
/// decides whether there is a document worth writing.
#[derive(Debug)]
#[must_use]
pub struct PdfExport {
    bytes: Vec<u8>,
    /// Pages in the document: one per view that rendered and converted.
    pub pages: usize,
    /// The views left out, each naming why.
    pub failures: Vec<ViewError>,
}

impl PdfExport {
    /// The PDF to write, or `None` when every view failed: a document of
    /// no pages would stand in for drawings that were never made. A design
    /// with no views at all still gets its (empty) document.
    #[must_use]
    pub fn document(&self) -> Option<&[u8]> {
        (self.pages > 0 || self.failures.is_empty()).then_some(self.bytes.as_slice())
    }
}

/// One converted view, ready to place on its page.
struct Page {
    title: String,
    layout: PageLayout,
    chunk: Chunk,
    svg_id: Ref,
}

impl Default for PdfExporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Draw the page header: the view's title, left-aligned to the margin
/// just above the content area — replacing the in-SVG title a
/// [`super::SvgMode::Standalone`] render carries, at a fixed size.
fn header(content: &mut Content, layout: &PageLayout, title: &str) {
    if title.is_empty() {
        return;
    }
    content.set_fill_gray(0.0);
    content.begin_text();
    content.set_font(Name(b"H"), HEADER_SIZE);
    content.next_line(MARGIN_PT, layout.height - MARGIN_PT + HEADER_GAP);
    content.show(Str(&win_ansi(title)));
    content.end_text();
}

/// Draw the page footer: the page number (`3 / 9`) centered, and — when a
/// manifest is present — the project-identity stamp right-aligned to the
/// margin. Both sit on [`FOOTER_BASELINE`], below the view's content area,
/// at a fixed size so every page reads the same regardless of view scale.
fn footer(
    content: &mut Content,
    layout: &PageLayout,
    page: usize,
    total: usize,
    manifest: Option<&Manifest>,
) {
    content.set_fill_gray(FOOTER_GRAY);
    content.begin_text();
    content.set_font(Name(b"F"), FOOTER_SIZE);

    let number = format!("{page} / {total}");
    content.next_line(
        (layout.width - footer_width(&number)) / 2.0,
        FOOTER_BASELINE,
    );
    content.show(Str(&win_ansi(&number)));

    if let Some(manifest) = manifest {
        let stamp = stamp_text(manifest);
        // Absolute positioning: `next_line` (Td) moves relative to the
        // previous line start, so subtract the number's start.
        content.next_line(
            layout.width
                - MARGIN_PT
                - footer_width(&stamp)
                - (layout.width - footer_width(&number)) / 2.0,
            0.0,
        );
        content.show(Str(&win_ansi(&stamp)));
    }

    content.end_text();
}

/// Width of `text` set in Courier at [`FOOTER_SIZE`] — exact, since every
/// Courier glyph advances the same 0.6 em.
fn footer_width(text: &str) -> f32 {
    text.chars().count() as f32 * COURIER_ADVANCE * FOOTER_SIZE
}

/// Encode to WinAnsi (Latin-1) bytes for a PDF string with
/// `WinAnsiEncoding`; characters outside it degrade to `?`.
fn win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}

/// A4 page geometry for one view: the page size (orientation chosen by the
/// view's aspect ratio) plus the content-stream matrix that scales and
/// centers svg2pdf's unit-sized XObject inside the margins.
struct PageLayout {
    width: f32,
    height: f32,
    /// `[dw, 0, 0, dh, tx, ty]` — the XObject is a unit square with image
    /// orientation (already y-flipped), so one `cm` both sizes and places it.
    matrix: [f32; 6],
}

impl PageLayout {
    fn fit(view: svg2pdf::usvg::Size) -> Self {
        let (vw, vh) = (view.width(), view.height());
        let (width, height) = if vw > vh {
            (A4_LONG_PT, A4_SHORT_PT)
        } else {
            (A4_SHORT_PT, A4_LONG_PT)
        };
        let scale = ((width - 2.0 * MARGIN_PT) / vw)
            .min((height - 2.0 * MARGIN_PT) / vh)
            .min(MAX_UPSCALE);
        let (dw, dh) = (scale * vw, scale * vh);
        Self {
            width,
            height,
            matrix: [dw, 0.0, 0.0, dh, (width - dw) / 2.0, (height - dh) / 2.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::ViewKind;

    fn size(w: f32, h: f32) -> svg2pdf::usvg::Size {
        svg2pdf::usvg::Size::from_wh(w, h).expect("positive size")
    }

    fn view(svg: &str) -> RenderedView {
        RenderedView {
            title: "Test".to_string(),
            filename: "test.svg".to_string(),
            kind: ViewKind::Schematic,
            svg: svg.to_string(),
        }
    }

    /// `views` as `render_views` hands them over: every one rendered.
    fn rendered(views: impl IntoIterator<Item = RenderedView>) -> Vec<ViewOutcome> {
        views.into_iter().map(Ok).collect()
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn page_layout_picks_orientation_from_aspect_ratio() {
        let wide = PageLayout::fit(size(200.0, 100.0));
        assert_eq!((wide.width, wide.height), (A4_LONG_PT, A4_SHORT_PT));

        let tall = PageLayout::fit(size(100.0, 200.0));
        assert_eq!((tall.width, tall.height), (A4_SHORT_PT, A4_LONG_PT));

        // A square view stays portrait, the conventional default.
        let square = PageLayout::fit(size(100.0, 100.0));
        assert_eq!((square.width, square.height), (A4_SHORT_PT, A4_LONG_PT));
    }

    #[test]
    fn page_layout_caps_upscaling_at_two_x() {
        // Raw fit would be ~7.5×; the cap keeps it at 2×, centered.
        let layout = PageLayout::fit(size(100.0, 50.0));
        assert_eq!(layout.matrix[0], 200.0);
        assert_eq!(layout.matrix[3], 100.0);
        assert_eq!(layout.matrix[4], (A4_LONG_PT - 200.0) / 2.0);
        assert_eq!(layout.matrix[5], (A4_SHORT_PT - 100.0) / 2.0);
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn page_layout_centers_a_large_view_inside_the_margins() {
        let layout = PageLayout::fit(size(2000.0, 1000.0));
        let scale = (A4_LONG_PT - 2.0 * MARGIN_PT) / 2000.0;
        assert_close(layout.matrix[0], A4_LONG_PT - 2.0 * MARGIN_PT);
        assert_close(layout.matrix[3], scale * 1000.0);
        assert_close(layout.matrix[4], MARGIN_PT);
        assert_close(layout.matrix[5], (A4_SHORT_PT - scale * 1000.0) / 2.0);
    }

    #[test]
    fn to_pdf_emits_one_page_per_view() {
        let views = [
            view(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><rect x="10" y="10" width="180" height="80" fill="red"/></svg>"#,
            ),
            view(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 200"><rect x="10" y="10" width="80" height="180" fill="blue"/></svg>"#,
            ),
        ];
        let pdf = PdfExporter::new().to_pdf(rendered(views), None).bytes;
        assert!(pdf.starts_with(b"%PDF-"), "missing PDF magic header");
        assert!(contains(&pdf, b"/Count 2"), "expected a two-page tree");
    }

    #[test]
    fn to_pdf_renders_text_labels() {
        let blank = view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"></svg>"#);
        let labelled = view(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"><text x="20" y="50" font-family="sans-serif" font-size="16">Battery Pack</text></svg>"#,
        );
        let exporter = PdfExporter::new();
        let blank_pdf = exporter.to_pdf(rendered([blank]), None).bytes;
        let labelled_pdf = exporter.to_pdf(rendered([labelled]), None).bytes;
        // Outlined glyph paths add real bytes; if text were silently
        // dropped the two documents would be nearly identical.
        assert!(
            labelled_pdf.len() > blank_pdf.len() + 200,
            "labelled PDF ({}) not materially larger than blank ({}) — text lost?",
            labelled_pdf.len(),
            blank_pdf.len()
        );
    }

    #[test]
    fn to_pdf_writes_the_view_title_in_the_header() {
        let views = [view(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"></svg>"#,
        )];
        let pdf = PdfExporter::new().to_pdf(rendered(views), None).bytes;
        // The `view` helper titles its view "Test"; the header shows it
        // literally in the uncompressed page content stream.
        assert!(contains(&pdf, b"(Test)"), "view title header missing");
    }

    #[test]
    fn to_pdf_numbers_pages_in_the_footer() {
        let views = [
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"></svg>"#),
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 200"></svg>"#),
        ];
        let pdf = PdfExporter::new().to_pdf(rendered(views), None).bytes;
        // Page content streams are uncompressed, so the footer's shown
        // strings appear literally.
        assert!(contains(&pdf, b"(1 / 2)"), "page 1 number missing");
        assert!(contains(&pdf, b"(2 / 2)"), "page 2 number missing");
    }

    #[test]
    fn to_pdf_stamps_every_page_from_the_manifest() {
        let manifest = Manifest {
            name: "demo".to_string(),
            version: "1.2.3".to_string(),
            description: None,
            authors: Vec::new(),
            license: None,
            revision: Some("abc1234".to_string()),
            date: None,
        };
        let views = [
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"></svg>"#),
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 200"></svg>"#),
        ];
        let pdf = PdfExporter::new()
            .to_pdf(rendered(views), Some(&manifest))
            .bytes;
        // The stamp text sits in each page's content stream; pdf-writer
        // hex-encodes strings with non-ASCII bytes (the `·` is WinAnsi
        // 0xB7), so match the hex form. The stamp always ends with the
        // wirebug-version suffix.
        let stamp_text = format!(
            "demo v1.2.3 · rev abc1234 · wirebug v{}",
            env!("CARGO_PKG_VERSION")
        );
        let hex: String = win_ansi(&stamp_text)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        let stamp = format!("<{hex}>");
        let hits = pdf
            .windows(stamp.len())
            .filter(|w| *w == stamp.as_bytes())
            .count();
        assert_eq!(hits, 2, "expected the stamp once per page");
    }

    #[test]
    fn to_pdf_leaves_out_a_view_that_fails_to_convert() {
        let good = view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"></svg>"#);
        let mut bad = view("this is not svg");
        bad.title = "Broken".to_string();
        let export = PdfExporter::new().to_pdf(rendered([bad, good]), None);

        assert_eq!(export.pages, 1);
        assert!(contains(&export.bytes, b"/Count 1"), "one page left");
        // The remaining page numbers itself among the pages written.
        assert!(contains(&export.bytes, b"(1 / 1)"), "page numbered 1 / 1");
        let [failure] = export.failures.as_slice() else {
            panic!("one failure expected: {:?}", export.failures);
        };
        assert_eq!(failure.title, "Broken");
        assert!(matches!(*failure.cause, ViewErrorKind::SvgParse(_)));
    }
}
