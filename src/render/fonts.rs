//! The font `--png` and `--pdf` draw text with: DejaVu Sans, compiled into
//! the binary (`assets/fonts/dejavu/`, with its licence).
//!
//! Both paths parse SVG with `usvg`, which looks text up in a font
//! database. Filling that database from the system fonts made a drawing
//! depend on whichever face the machine resolves `sans-serif` to; loading
//! only the bundled faces makes PNG and PDF output the same everywhere.
//! The SVG files themselves still ask for `sans-serif` and follow the
//! viewer's font.
//!
//! resvg's and svg2pdf's `usvg` are different crate versions, but they
//! share one `fontdb`, so one database serves both.

use std::sync::Arc;

use resvg::usvg::fontdb::{Database, Source};

/// The family name of every bundled face.
pub(crate) const FAMILY: &str = "DejaVu Sans";

/// The bundled faces: the stylesheets draw regular, bold and italic text,
/// never bold italic.
const FACES: [&[u8]; 3] = [
    include_bytes!("../../assets/fonts/dejavu/DejaVuSans.ttf"),
    include_bytes!("../../assets/fonts/dejavu/DejaVuSans-Bold.ttf"),
    include_bytes!("../../assets/fonts/dejavu/DejaVuSans-Oblique.ttf"),
];

/// A font database holding the bundled faces and nothing else, with every
/// generic family mapped to [`FAMILY`]: our labels ask for `sans-serif`,
/// and usvg's last resort for a family it cannot find is `serif`, so with
/// every generic on the one family no text is ever dropped for want of a
/// face.
///
/// The faces are compile-time constants that a unit test loads, so the
/// load cannot fail at run time.
#[must_use]
pub(crate) fn database() -> Arc<Database> {
    let mut db = Database::new();
    for face in FACES {
        db.load_font_source(Source::Binary(Arc::new(face)));
    }
    db.set_sans_serif_family(FAMILY);
    db.set_serif_family(FAMILY);
    db.set_monospace_family(FAMILY);
    db.set_cursive_family(FAMILY);
    db.set_fantasy_family(FAMILY);
    Arc::new(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use resvg::usvg::fontdb::{Family, Query, Style, Weight};

    #[test]
    fn the_database_holds_exactly_the_bundled_faces() {
        let db = database();
        let mut faces: Vec<String> = db
            .faces()
            .map(|face| {
                let families: Vec<&str> = face.families.iter().map(|(n, _)| n.as_str()).collect();
                format!(
                    "{} {families:?} {} {:?}",
                    face.post_script_name, face.weight.0, face.style
                )
            })
            .collect();
        faces.sort();
        assert_eq!(
            faces,
            [
                r#"DejaVuSans ["DejaVu Sans"] 400 Normal"#,
                r#"DejaVuSans-Bold ["DejaVu Sans"] 700 Normal"#,
                r#"DejaVuSans-Oblique ["DejaVu Sans"] 400 Italic"#,
            ]
        );
    }

    #[test]
    fn every_generic_family_resolves_to_the_bundled_face_of_its_style() {
        let db = database();
        let face_of = |family, weight, style| {
            let id = db
                .query(&Query {
                    families: &[family],
                    weight,
                    style,
                    ..Query::default()
                })
                .expect("a bundled face matches");
            let face = db.face(id).expect("the matched face exists");
            (face.post_script_name.clone(), face.weight, face.style)
        };
        for family in [
            Family::SansSerif,
            Family::Serif,
            Family::Monospace,
            Family::Cursive,
            Family::Fantasy,
        ] {
            assert_eq!(
                face_of(family, Weight::NORMAL, Style::Normal).0,
                "DejaVuSans"
            );
            assert_eq!(
                face_of(family, Weight::BOLD, Style::Normal).0,
                "DejaVuSans-Bold"
            );
            assert_eq!(
                face_of(family, Weight::NORMAL, Style::Italic).0,
                "DejaVuSans-Oblique"
            );
        }
    }
}
