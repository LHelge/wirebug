//! SVG → single-file PDF export for `render --pdf`.
//!
//! Each rendered view becomes one page of the chosen [`Paper`] (A4 unless
//! `--paper` says otherwise): the SVG is parsed with [`svg2pdf`]'s own
//! `usvg` (a separate crate version from the resvg pair the PNG path uses —
//! the two never interoperate), converted to a unit-sized Form XObject via
//! [`svg2pdf::to_chunk`], and placed on a page whose orientation follows the
//! view's aspect ratio. The multi-page document is assembled by hand with
//! [`pdf_writer`]. Fixed-size page furniture replaces the in-SVG title and
//! stamp a standalone render carries: a title header, plus a footer with
//! the identity stamp and a `<page> / <total>` number.
//!
//! A view is scaled to fit its page, so a wide one shrinks its text with
//! it: a page whose view's smallest text comes out below
//! [`MIN_LEGIBLE_PT`] is reported as a [`SmallText`], naming the smallest
//! paper that would print it legibly.
//!
//! Text is outlined to paths at conversion time (`embed_text: false`), so
//! labels survive any PDF viewer without font embedding — but only if the
//! font database is populated at *parse* time, exactly as in
//! [`super::png::PngRasterizer`]; an empty fontdb drops text silently.

use std::collections::HashMap;
use std::fmt;

use pdf_writer::{Chunk, Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

use super::stamp::stamp_text;
use super::{RenderedView, ViewOutcome};
use crate::dsl::manifest::Manifest;
use crate::error::{ViewError, ViewErrorKind, ViewResult};

/// Page margin (15 mm) in PDF points.
const MARGIN_PT: f32 = 42.52;
/// Cap on blowing up small views, so a lone tiny box doesn't fill a page.
const MAX_UPSCALE: f32 = 2.0;

/// The smallest a view's text may print, in points, before its page is
/// reported: 5 pt is a common floor for lettering on printed technical
/// drawings — below it pin numbers and wire codes stop being readable on
/// paper. One number, so the threshold is easy to move.
pub const MIN_LEGIBLE_PT: f32 = 5.0;

/// The paper sheet each view of a PDF is laid on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Paper {
    /// ISO A4, 210 × 297 mm.
    #[default]
    A4,
    /// ISO A3, 297 × 420 mm.
    A3,
    /// US Letter, 8.5 × 11 in.
    Letter,
    /// US Tabloid (ledger), 11 × 17 in.
    Tabloid,
}

impl Paper {
    /// Every paper, smallest sheet (by area) first: the order in which a
    /// [`SmallText`] looks for one that prints its view legibly.
    const BY_AREA: [Self; 4] = [Self::Letter, Self::A4, Self::Tabloid, Self::A3];

    /// The sheet's short and long edge, in PDF points (1/72 in).
    #[must_use]
    pub fn size(self) -> (f32, f32) {
        match self {
            Self::A4 => (595.276, 841.89),
            Self::A3 => (841.89, 1190.551),
            Self::Letter => (612.0, 792.0),
            Self::Tabloid => (792.0, 1224.0),
        }
    }
}

impl fmt::Display for Paper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::A4 => "A4",
            Self::A3 => "A3",
            Self::Letter => "Letter",
            Self::Tabloid => "Tabloid",
        })
    }
}

/// A view whose smallest text prints below [`MIN_LEGIBLE_PT`] on the paper
/// its page was laid on — a warning: the page is in the document all the
/// same. It prints `view "<title>": smallest text <n> pt on <paper>, …`.
#[derive(Debug, Clone, PartialEq)]
pub struct SmallText {
    /// The view's title.
    pub title: String,
    /// The paper its page was laid on.
    pub paper: Paper,
    /// Its smallest text's size on that paper, in points.
    pub points: f32,
    /// The paper that would serve the view best.
    pub best: BestPaper,
}

/// Which of the four papers would serve a [`SmallText`] view best.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BestPaper {
    /// The smallest paper (by area) on which its smallest text reaches
    /// [`MIN_LEGIBLE_PT`].
    Legible(Paper),
    /// No paper makes it legible: the one that prints it largest (the
    /// paper in use unless another prints it larger, else the smaller sheet
    /// on a tie), and the size it prints at there, in points — telling a
    /// near miss from a hopeless case.
    Closest { paper: Paper, points: f32 },
}

impl SmallText {
    /// Judge a view titled `title`, `size` user units large, whose smallest
    /// text is `smallest` units, laid on `paper`: `None` when that text is
    /// legible there.
    fn check(title: &str, smallest: f32, size: svg2pdf::usvg::Size, paper: Paper) -> Option<Self> {
        let points = |paper| smallest * PageLayout::fit(size, paper).scale;
        let on_paper = points(paper);
        if on_paper >= MIN_LEGIBLE_PT {
            return None;
        }
        let sizes = Paper::BY_AREA.map(|paper| (paper, points(paper)));
        let best = match sizes.iter().find(|(_, points)| *points >= MIN_LEGIBLE_PT) {
            Some(&(paper, _)) => BestPaper::Legible(paper),
            None => {
                // From the paper in use, so another only wins by printing
                // larger; among the others the smaller sheet wins a tie.
                let (paper, points) = sizes.into_iter().fold((paper, on_paper), |best, next| {
                    if next.1 > best.1 { next } else { best }
                });
                BestPaper::Closest { paper, points }
            }
        };
        Some(Self {
            title: title.to_string(),
            paper,
            points: on_paper,
            best,
        })
    }
}

/// `points` to one decimal, rounded down, so a size just under the minimum
/// never reads as the minimum.
fn round_down(points: f32) -> f32 {
    (points * 10.0).floor() / 10.0
}

impl fmt::Display for SmallText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "view \"{}\": smallest text {:.1} pt on {}, below the {MIN_LEGIBLE_PT} pt \
             legibility minimum; ",
            self.title,
            round_down(self.points),
            self.paper
        )?;
        match self.best {
            BestPaper::Legible(paper) => write!(f, "legible on {paper}"),
            BestPaper::Closest { paper, .. } if paper == self.paper => {
                f.write_str("no paper size prints it larger")
            }
            BestPaper::Closest { paper, points } => {
                write!(f, "closest is {paper} at {:.1} pt", round_down(points))
            }
        }
    }
}

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

/// Exports rendered views into one multi-page PDF, holding a font database
/// loaded once up front.
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
    /// PDF, one `paper` page per rendered view in order.
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
    /// [`PdfExport::failures`], in view order. A page whose view's text
    /// prints too small to read is kept, and named in
    /// [`PdfExport::small_text`].
    pub fn to_pdf(
        &self,
        views: Vec<ViewOutcome>,
        manifest: Option<&Manifest>,
        paper: Paper,
    ) -> PdfExport {
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
        let mut small_text = Vec::new();
        let mut pages = Vec::new();
        for view in views {
            match view.and_then(|view| self.convert(view, paper, &mut alloc)) {
                Ok((page, small)) => {
                    pages.push(page);
                    small_text.extend(small);
                }
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
            small_text,
        }
    }

    /// Convert one view's SVG into a Form XObject chunk numbered from
    /// `alloc`, with the `paper` page geometry that fits it, and judge
    /// whether its text is legible there; a failure names the view.
    fn convert(
        &self,
        view: RenderedView,
        paper: Paper,
        alloc: &mut Ref,
    ) -> Result<(Page, Option<SmallText>), ViewError> {
        match self.convert_svg(&view.svg, alloc) {
            Ok((size, chunk, svg_id)) => {
                let small = view.smallest_text().and_then(|smallest| {
                    SmallText::check(&view.title, smallest as f32, size, paper)
                });
                let page = Page {
                    title: view.title,
                    layout: PageLayout::fit(size, paper),
                    chunk,
                    svg_id,
                };
                Ok((page, small))
            }
            Err(cause) => Err(view.failed(cause)),
        }
    }

    /// The conversion half of [`Self::convert`], before the view is named:
    /// the view's size in user units, its chunk and the chunk's root.
    fn convert_svg(
        &self,
        svg: &str,
        alloc: &mut Ref,
    ) -> ViewResult<(svg2pdf::usvg::Size, Chunk, Ref)> {
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
        Ok((tree.size(), chunk, svg_id))
    }
}

/// What [`PdfExporter::to_pdf`] produced: the document, how many pages it
/// holds, the views left out because they failed to render or convert, and
/// the pages whose text prints too small. The bytes are reached only
/// through [`Self::document`], which decides whether there is a document
/// worth writing.
#[derive(Debug)]
#[must_use]
pub struct PdfExport {
    bytes: Vec<u8>,
    /// Pages in the document: one per view that rendered and converted.
    pub pages: usize,
    /// The views left out, each naming why.
    pub failures: Vec<ViewError>,
    /// The pages kept whose view's smallest text prints below
    /// [`MIN_LEGIBLE_PT`], in view order.
    pub small_text: Vec<SmallText>,
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

/// Page geometry for one view: the page size (orientation chosen by the
/// view's aspect ratio) plus the content-stream matrix that scales and
/// centers svg2pdf's unit-sized XObject inside the margins.
struct PageLayout {
    width: f32,
    height: f32,
    /// Points per view user unit: the view is scaled down to fit inside
    /// the margins, or up to fill them, but at most [`MAX_UPSCALE`].
    scale: f32,
    /// `[dw, 0, 0, dh, tx, ty]` — the XObject is a unit square with image
    /// orientation (already y-flipped), so one `cm` both sizes and places it.
    matrix: [f32; 6],
}

impl PageLayout {
    /// Lay a view `view` user units large on `paper`: landscape for a view
    /// wider than tall, portrait otherwise — with equal margins all round
    /// that is also the orientation that gives the view the larger scale.
    fn fit(view: svg2pdf::usvg::Size, paper: Paper) -> Self {
        let (short, long) = paper.size();
        let (vw, vh) = (view.width(), view.height());
        let (width, height) = if vw > vh {
            (long, short)
        } else {
            (short, long)
        };
        let scale = ((width - 2.0 * MARGIN_PT) / vw)
            .min((height - 2.0 * MARGIN_PT) / vh)
            .min(MAX_UPSCALE);
        let (dw, dh) = (scale * vw, scale * vh);
        Self {
            width,
            height,
            scale,
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
            // The export reads the size from the SVG, never from here.
            canvas: crate::render::Canvas::new(0.0, 0.0),
        }
    }

    /// `views` as `render_views` hands them over: every one rendered.
    fn rendered(views: impl IntoIterator<Item = RenderedView>) -> Vec<ViewOutcome> {
        views.into_iter().map(Ok).collect()
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    const A4_SHORT_PT: f32 = 595.276;
    const A4_LONG_PT: f32 = 841.89;

    #[test]
    fn paper_sizes_are_the_standard_sheets_in_points() {
        let mm = |mm: f32| mm * 72.0 / 25.4;
        let close = |(short, long): (f32, f32), (w, h): (f32, f32)| {
            (short - w).abs() < 0.01 && (long - h).abs() < 0.01
        };
        assert!(close(Paper::A4.size(), (mm(210.0), mm(297.0))));
        assert!(close(Paper::A3.size(), (mm(297.0), mm(420.0))));
        assert_eq!(Paper::Letter.size(), (8.5 * 72.0, 11.0 * 72.0));
        assert_eq!(Paper::Tabloid.size(), (11.0 * 72.0, 17.0 * 72.0));
        // The A4 sheet is exactly what every PDF before `--paper` used, so
        // a default render writes the same bytes as before.
        assert_eq!(Paper::default().size(), (A4_SHORT_PT, A4_LONG_PT));
    }

    #[test]
    fn papers_by_area_run_smallest_first() {
        let area = |paper: Paper| paper.size().0 * paper.size().1;
        assert!(Paper::BY_AREA.windows(2).all(|w| area(w[0]) < area(w[1])));
    }

    #[test]
    fn page_layout_picks_orientation_from_aspect_ratio_on_every_paper() {
        for paper in Paper::BY_AREA {
            let (short, long) = paper.size();
            let wide = PageLayout::fit(size(200.0, 100.0), paper);
            assert_eq!((wide.width, wide.height), (long, short), "{paper}");

            let tall = PageLayout::fit(size(100.0, 200.0), paper);
            assert_eq!((tall.width, tall.height), (short, long), "{paper}");

            // A square view stays portrait, the conventional default.
            let square = PageLayout::fit(size(100.0, 100.0), paper);
            assert_eq!((square.width, square.height), (short, long), "{paper}");
        }
    }

    #[test]
    fn page_layout_caps_upscaling_at_two_x() {
        // Raw fit would be ~7.5×; the cap keeps it at 2×, centered.
        let layout = PageLayout::fit(size(100.0, 50.0), Paper::A4);
        assert_eq!(layout.scale, 2.0);
        assert_eq!(layout.matrix[0], 200.0);
        assert_eq!(layout.matrix[3], 100.0);
        assert_eq!(layout.matrix[4], (A4_LONG_PT - 200.0) / 2.0);
        assert_eq!(layout.matrix[5], (A4_SHORT_PT - 100.0) / 2.0);
    }

    #[test]
    fn a_small_view_is_not_blown_up_further_on_larger_paper() {
        let a4 = PageLayout::fit(size(100.0, 50.0), Paper::A4);
        let a3 = PageLayout::fit(size(100.0, 50.0), Paper::A3);
        assert_eq!(a3.scale, a4.scale);
        assert_eq!((a3.matrix[0], a3.matrix[3]), (200.0, 100.0));
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn page_layout_centers_a_large_view_inside_the_margins() {
        let layout = PageLayout::fit(size(2000.0, 1000.0), Paper::A4);
        let scale = (A4_LONG_PT - 2.0 * MARGIN_PT) / 2000.0;
        assert_close(layout.scale, scale);
        assert_close(layout.matrix[0], A4_LONG_PT - 2.0 * MARGIN_PT);
        assert_close(layout.matrix[3], scale * 1000.0);
        assert_close(layout.matrix[4], MARGIN_PT);
        assert_close(layout.matrix[5], (A4_SHORT_PT - scale * 1000.0) / 2.0);
    }

    #[test]
    fn page_layout_fits_a_tall_view_by_its_height() {
        let layout = PageLayout::fit(size(500.0, 3000.0), Paper::A3);
        assert_close(layout.scale, (1190.551 - 2.0 * MARGIN_PT) / 3000.0);
    }

    #[test]
    fn small_text_is_the_smallest_font_times_the_page_scale() {
        // 2000 units wide on A4 landscape: 756.85 pt of content width, so
        // 9-unit text prints at 9 × 756.85 / 2000 ≈ 3.41 pt.
        let small =
            SmallText::check("Wide", 9.0, size(2000.0, 600.0), Paper::A4).expect("too small on A4");
        assert_eq!(small.title, "Wide");
        assert_eq!(small.paper, Paper::A4);
        assert_close(small.points, 9.0 * (A4_LONG_PT - 2.0 * MARGIN_PT) / 2000.0);
    }

    #[test]
    fn text_at_or_above_the_minimum_is_legible() {
        // 1000 units on A4 landscape: 9-unit text prints at ≈ 6.8 pt.
        assert_eq!(
            SmallText::check("Fits", 9.0, size(1000.0, 500.0), Paper::A4),
            None
        );
        // A view whose text prints at exactly the minimum passes: 5 pt
        // text on a view scaled 1:1 (scale capped at 2 here, so 2.5 units).
        assert_eq!(
            SmallText::check("Edge", 2.5, size(100.0, 50.0), Paper::A4),
            None
        );
    }

    #[test]
    fn small_text_names_the_smallest_paper_that_prints_it_legibly() {
        // 1800 units wide: ≈ 3.8 pt on A4 and 3.5 on Letter, but 5.7 on
        // Tabloid and 5.5 on A3 — Tabloid is the smaller sheet.
        let small =
            SmallText::check("Wide", 9.0, size(1800.0, 400.0), Paper::A4).expect("too small on A4");

        assert_eq!(small.best, BestPaper::Legible(Paper::Tabloid));

        // ≈ 4.9 pt on A4, but Letter's taller short edge fits a nearly
        // square view larger: Letter, smaller than A4, is enough.
        let squarish = SmallText::check("Square", 9.0, size(1000.0, 940.0), Paper::A4)
            .expect("too small on A4");
        assert_eq!(squarish.best, BestPaper::Legible(Paper::Letter));
    }

    #[test]
    fn small_text_legible_on_no_paper_names_the_closest_and_its_size() {
        // 6000 units wide: Tabloid's long edge, the longest of the four,
        // prints 9-unit text at 9 × 1138.96 / 6000 ≈ 1.7 pt.
        let huge =
            SmallText::check("Huge", 9.0, size(6000.0, 400.0), Paper::A4).expect("too small on A4");
        let BestPaper::Closest { paper, points } = huge.best else {
            panic!("no paper is legible: {:?}", huge.best);
        };
        assert_eq!(paper, Paper::Tabloid);
        assert_close(points, 9.0 * (1224.0 - 2.0 * MARGIN_PT) / 6000.0);

        // A view small enough to be capped at 2× on every sheet prints the
        // same everywhere: no other paper beats the one in use.
        let tiny =
            SmallText::check("Tiny", 1.0, size(100.0, 50.0), Paper::A3).expect("too small on A3");
        assert_eq!(
            tiny.best,
            BestPaper::Closest {
                paper: Paper::A3,
                points: 2.0
            }
        );
    }

    #[test]
    fn small_text_warning_names_the_view_the_size_and_the_best_paper() {
        let small = SmallText {
            title: "HV System Overview".to_string(),
            paper: Paper::A4,
            points: 3.06,
            best: BestPaper::Legible(Paper::A3),
        };
        assert_eq!(
            small.to_string(),
            "view \"HV System Overview\": smallest text 3.0 pt on A4, below the 5 pt \
             legibility minimum; legible on A3"
        );
        let near_miss = SmallText {
            best: BestPaper::Closest {
                paper: Paper::Tabloid,
                points: 4.99,
            },
            ..small
        };
        assert_eq!(
            near_miss.to_string(),
            "view \"HV System Overview\": smallest text 3.0 pt on A4, below the 5 pt \
             legibility minimum; closest is Tabloid at 4.9 pt"
        );
        // Already on the closest paper: there is nothing better to name.
        let on_closest = SmallText {
            paper: Paper::Tabloid,
            points: 4.99,
            ..near_miss
        };
        assert_eq!(
            on_closest.to_string(),
            "view \"HV System Overview\": smallest text 4.9 pt on Tabloid, below the 5 pt \
             legibility minimum; no paper size prints it larger"
        );
    }

    #[test]
    fn to_pdf_lays_every_page_on_the_chosen_paper() {
        let views = [
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100"></svg>"#),
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 200"></svg>"#),
        ];
        let pdf = PdfExporter::new()
            .to_pdf(rendered(views), None, Paper::A3)
            .bytes;
        assert!(contains(&pdf, b"/MediaBox [0 0 1190.551 841.89]"));
        assert!(contains(&pdf, b"/MediaBox [0 0 841.89 1190.551]"));
    }

    #[test]
    fn to_pdf_warns_of_small_text_on_a_kept_page_only() {
        let wide = view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 3000 400"></svg>"#);
        let mut narrow =
            view(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300"></svg>"#);
        narrow.title = "Narrow".to_string();
        let mut broken = view("this is not svg");
        broken.title = "Broken".to_string();
        let export = PdfExporter::new().to_pdf(rendered([wide, narrow, broken]), None, Paper::A4);

        assert_eq!(export.pages, 2, "a small-text page is kept");
        let titles: Vec<&str> = export.small_text.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Test"], "only the wide view, not the failed one");
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
        let pdf = PdfExporter::new()
            .to_pdf(rendered(views), None, Paper::A4)
            .bytes;
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
        let blank_pdf = exporter.to_pdf(rendered([blank]), None, Paper::A4).bytes;
        let labelled_pdf = exporter.to_pdf(rendered([labelled]), None, Paper::A4).bytes;
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
        let pdf = PdfExporter::new()
            .to_pdf(rendered(views), None, Paper::A4)
            .bytes;
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
        let pdf = PdfExporter::new()
            .to_pdf(rendered(views), None, Paper::A4)
            .bytes;
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
            .to_pdf(rendered(views), Some(&manifest), Paper::A4)
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
        let export = PdfExporter::new().to_pdf(rendered([bad, good]), None, Paper::A4);

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
