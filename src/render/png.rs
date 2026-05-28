//! SVG → PNG rasterisation for `render --png`.
//!
//! A thin wrapper over [`resvg`]: parse the SVG with `usvg`, allocate a
//! `tiny_skia::Pixmap` sized to `scale` × the SVG's intrinsic dimensions,
//! render, and encode to PNG bytes. The renderer is unchanged — this only
//! runs in the CLI's `--png` mode, after `render_views` has produced SVGs.

use crate::error::{Error, Result};

/// Rasterise `svg` to a PNG byte stream at `scale` × intrinsic size.
pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())
        .map_err(|e| Error::SvgParse(e.to_string()))?;
    let size = tree.size();
    let w = (size.width() * scale).round().max(1.0) as u32;
    let h = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| Error::PngEncode(format!("pixmap allocation failed for {w}x{h}")))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| Error::PngEncode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn svg_to_png_emits_a_valid_png() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                       <rect width="10" height="10" fill="red"/></svg>"#;
        let png = svg_to_png(svg, 2.0).expect("conversion succeeds");
        assert_eq!(&png[..8], PNG_MAGIC);
        // A 20×20 red square encodes to well over the PNG header alone.
        assert!(png.len() > 50);
    }

    #[test]
    fn svg_to_png_rejects_malformed_input() {
        assert!(matches!(
            svg_to_png("not svg at all", 2.0),
            Err(Error::SvgParse(_))
        ));
    }
}
