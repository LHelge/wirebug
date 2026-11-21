//! SVG → PNG rasterisation for `render --png`.
//!
//! A thin wrapper over [`resvg`]: parse the SVG with `usvg`, allocate a
//! `tiny_skia::Pixmap` sized to `scale` × the SVG's intrinsic dimensions,
//! render, and encode to PNG bytes. The renderer is unchanged — this only
//! runs in the CLI's `--png` mode, after `render_views` has produced SVGs.
//!
//! `usvg`'s default options ship an *empty* font database, so text simply
//! vanishes from the raster. [`PngRasterizer`] loads the system fonts once
//! (the load is slow, so it's amortised across every view) and resolves our
//! `sans-serif` labels through fontdb's generic-family fallback.

use resvg::tiny_skia::{IntSize, Pixmap};
use resvg::usvg::Tree;

use super::Canvas;
use crate::error::{ViewErrorKind, ViewResult};

/// How many pixels `render --png` draws per world unit.
pub const PNG_SCALE: f32 = 2.0;

/// Rasterises SVGs to PNG, holding a font database loaded once up front.
pub struct PngRasterizer {
    options: resvg::usvg::Options<'static>,
}

impl PngRasterizer {
    /// Build a rasteriser with the system fonts loaded.
    #[must_use]
    pub fn new() -> Self {
        use resvg::usvg::fontdb::{Family, Query};

        let mut options = resvg::usvg::Options::default();
        let db = options.fontdb_mut();
        db.load_system_fonts();

        // Our labels ask for the generic `sans-serif`, which fontdb maps to a
        // default family name (`Arial`). A minimal host — e.g. a CI runner —
        // may have fonts but not that exact name, and usvg then drops the text
        // entirely rather than substituting. Point the generic at a present
        // face so labels survive anywhere a font exists.
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

    /// Rasterise `svg` to a PNG byte stream at `scale` × intrinsic size.
    /// A canvas over `render::MAX_CANVAS` is refused before any pixel is
    /// allocated, and a pixmap the allocator cannot provide is an error,
    /// not an abort.
    pub fn to_png(&self, svg: &str, scale: f32) -> ViewResult<Vec<u8>> {
        let tree = self.admitted(svg)?;
        let size = tree.size();
        let w = (size.width() * scale).round().max(1.0) as u32;
        let h = (size.height() * scale).round().max(1.0) as u32;
        let mut pixmap = blank_pixmap(w, h)?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        pixmap
            .encode_png()
            .map_err(|e| ViewErrorKind::PngEncode(e.to_string()))
    }

    /// `svg` parsed, unless its canvas is too large to rasterise: the
    /// check [`Self::to_png`] makes before it allocates anything.
    pub(crate) fn admitted(&self, svg: &str) -> ViewResult<Tree> {
        let tree = Tree::from_str(svg, &self.options)
            .map_err(|e| ViewErrorKind::SvgParse(e.to_string()))?;
        let size = tree.size();
        Canvas::new(f64::from(size.width()), f64::from(size.height())).admit()?;
        Ok(tree)
    }
}

/// A transparent `w` × `h` pixmap. `Pixmap::new` allocates with `vec!`,
/// which aborts the process when the allocator says no; reserving first
/// turns that into an error.
fn blank_pixmap(w: u32, h: u32) -> ViewResult<Pixmap> {
    let refused =
        |why: String| ViewErrorKind::PngEncode(format!("cannot allocate a {w}×{h} pixmap: {why}"));
    let size = IntSize::from_wh(w, h).ok_or_else(|| refused("empty".to_string()))?;
    let len = (w as usize)
        .checked_mul(h as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| refused("too many bytes".to_string()))?;
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|e| refused(e.to_string()))?;
    data.resize(len, 0);
    Pixmap::from_vec(data, size).ok_or_else(|| refused("wider than tiny-skia allows".to_string()))
}

impl Default for PngRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn to_png_emits_a_valid_png() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                       <rect width="10" height="10" fill="red"/></svg>"#;
        let png = PngRasterizer::new()
            .to_png(svg, 2.0)
            .expect("conversion succeeds");
        assert_eq!(&png[..8], PNG_MAGIC);
        // A 20×20 red square encodes to well over the PNG header alone.
        assert!(png.len() > 50);
    }

    #[test]
    fn to_png_renders_text_labels() {
        // An empty fontdb silently drops text; with system fonts loaded the
        // glyphs paint pixels, so the rendered raster is materially larger
        // than the same canvas with no text on it.
        let canvas = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60">"#;
        let blank = format!("{canvas}</svg>");
        let labelled = format!(
            r#"{canvas}<text x="10" y="40" font-family="sans-serif" font-size="20">Hello wirebug</text></svg>"#
        );
        let r = PngRasterizer::new();
        let blank_png = r.to_png(&blank, 2.0).expect("blank renders");
        let labelled_png = r.to_png(&labelled, 2.0).expect("labelled renders");
        assert!(
            labelled_png.len() > blank_png.len(),
            "text should add pixels: blank={} labelled={}",
            blank_png.len(),
            labelled_png.len()
        );
    }

    #[test]
    fn to_png_refuses_a_canvas_over_the_limit_before_allocating() {
        // Rasterised, this would be a 4000000 × 20 pixmap of 320 MB; the
        // refusal comes before any of it is allocated.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2000000 10"/>"#;
        let refused = PngRasterizer::new().to_png(svg, 2.0);
        let Err(ViewErrorKind::CanvasTooLarge {
            width,
            height,
            limit,
        }) = refused
        else {
            panic!("expected CanvasTooLarge, got {refused:?}");
        };
        assert_eq!(
            (width, height, limit),
            (2_000_000.0, 10.0, super::super::MAX_CANVAS)
        );
    }

    #[test]
    fn a_canvas_exactly_at_the_limit_is_admitted() {
        let r = PngRasterizer::new();
        let side = super::super::MAX_CANVAS;
        let at = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {side} 10"/>"#);
        assert!(r.admitted(&at).is_ok());
        let over = at.replace(&format!(" {side} "), &format!(" {} ", side + 1.0));
        assert!(r.admitted(&over).is_err());
    }

    #[test]
    fn a_blank_pixmap_has_the_size_asked_for() {
        let pixmap = blank_pixmap(3, 2).expect("allocates");
        assert_eq!((pixmap.width(), pixmap.height()), (3, 2));
        assert_eq!(pixmap.data().len(), 3 * 2 * 4);
    }

    #[test]
    fn to_png_rejects_malformed_input() {
        assert!(matches!(
            PngRasterizer::new().to_png("not svg at all", 2.0),
            Err(ViewErrorKind::SvgParse(_))
        ));
    }
}
