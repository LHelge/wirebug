//! How wide text is drawn, and fitting it into the width it is given: a
//! harness table's header and columns, a pinout's cavity label into its
//! cavity, a table cell's text into its column. Shared by the renderers,
//! so every one measures alike.
//!
//! SVG text has no clipping and no layout: a label is as wide as its font
//! makes it. PNG and PDF draw it in the bundled DejaVu Sans
//! ([`super::fonts`]); an SVG viewer draws it in whatever `sans-serif`
//! resolves to on its machine. So the width is an upper bound, per
//! character class, on DejaVu Sans's own advances in the face the text is
//! set in (a test holds every class to the bundled faces), and DejaVu Sans
//! is the widest of the common sans-serif faces (wider than Arial,
//! Liberation Sans and Helvetica), so the bound holds for those viewers
//! too. A character outside the classes counts a full em, wider than any
//! Latin glyph and than the box DejaVu Sans draws for a character it has
//! no glyph for, and as wide as a fallback face's CJK glyph. An estimate
//! that errs wide shrinks a label a little early or widens a box; one
//! that errs narrow lets text cross a border.

/// The cavity label's size when it fits, in px (the `.cavity-label` font).
pub(super) const LABEL_SIZE: f64 = 8.0;
/// The smallest size a label shrinks to before it is cut: below it a
/// printed label stops being readable.
pub(super) const MIN_LABEL_SIZE: f64 = 6.0;
/// Sizes step down by this much, so a drawing holds a few distinct sizes
/// rather than one per label.
const SIZE_STEP: f64 = 0.5;
const ELLIPSIS: char = '…';
/// How much wider a bold face sets than its regular one, rounded up from
/// DejaVu Sans Bold's widest classes (capitals, `m`/`w`).
const BOLD_WIDENING: f64 = 1.15;

/// Which of the bundled faces text is set in. The stylesheets set
/// regular, bold and italic text, never bold italic; DejaVu Sans Oblique
/// has the regular face's advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Face {
    Regular,
    Bold,
    Oblique,
}

impl Face {
    /// One character's advance in ems in this face, before a bold face's
    /// [`BOLD_WIDENING`].
    fn char_ems(self, c: char) -> f64 {
        match (self, c) {
            // DejaVu Sans Bold widens `r` more than the rest of its class.
            (Self::Bold, 'r') => 0.43,
            _ => char_ems(c),
        }
    }

    /// `text`'s advance in ems in this face, before a bold face's
    /// widening.
    fn ems(self, text: &str) -> f64 {
        text.chars().map(|c| self.char_ems(c)).sum()
    }
}

/// The font a piece of text is set in, as the stylesheet gives it: its
/// size in px and its face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Font {
    size: f64,
    face: Face,
}

impl Font {
    pub(super) const fn regular(size: f64) -> Self {
        Self {
            size,
            face: Face::Regular,
        }
    }

    pub(super) const fn bold(size: f64) -> Self {
        Self {
            size,
            face: Face::Bold,
        }
    }

    pub(super) const fn italic(size: f64) -> Self {
        Self {
            size,
            face: Face::Oblique,
        }
    }

    /// `text`'s drawn width in this font, in px: never less than DejaVu
    /// Sans draws it.
    pub(super) fn width(&self, text: &str) -> f64 {
        self.face.ems(text) * self.ems_to_px()
    }

    /// `text` as it is drawn in `width` px in this font: whole when it
    /// fits, else as much of its start as fits before an ellipsis. The
    /// size never changes: a table column is sized to its content, and
    /// only text past the column's cap is cut.
    pub(super) fn fit(&self, text: &str, width: f64) -> FittedLabel {
        let text = if self.width(text) <= width {
            text.to_string()
        } else {
            cut(text, width / self.ems_to_px(), self.face)
        };
        FittedLabel {
            text,
            size: self.size,
        }
    }

    fn ems_to_px(&self) -> f64 {
        match self.face {
            Face::Bold => self.size * BOLD_WIDENING,
            Face::Regular | Face::Oblique => self.size,
        }
    }
}

/// A label as it is drawn in a given width: at `size` px, its text cut
/// short with an ellipsis when even [`MIN_LABEL_SIZE`] is too wide.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct FittedLabel {
    pub(super) text: String,
    pub(super) size: f64,
}

impl FittedLabel {
    /// `label` fitted into `width` px: at [`LABEL_SIZE`] when it fits, else
    /// at the largest step down to [`MIN_LABEL_SIZE`] that fits, else at
    /// that floor with as much of its start as fits before an ellipsis.
    pub(super) fn new(label: &str, width: f64) -> Self {
        let ems = Face::Regular.ems(label);
        let mut size = LABEL_SIZE;
        while size > MIN_LABEL_SIZE && ems * size > width {
            size -= SIZE_STEP;
        }
        if ems * size <= width {
            return Self {
                text: label.to_string(),
                size,
            };
        }
        Self {
            text: cut(label, width / MIN_LABEL_SIZE, Face::Regular),
            size: MIN_LABEL_SIZE,
        }
    }

    /// Whether the label lost characters to the ellipsis.
    pub(super) fn is_cut(&self) -> bool {
        self.text.ends_with(ELLIPSIS)
    }

    /// Whether the label is drawn smaller than [`LABEL_SIZE`].
    pub(super) fn is_shrunk(&self) -> bool {
        self.size < LABEL_SIZE
    }
}

/// As much of `text`'s start as fits in `ems` of `face` together with an
/// ellipsis, then the ellipsis; no space or `·` separator is left before
/// it, so a cut never ends on a dangling separator.
fn cut(text: &str, ems: f64, face: Face) -> String {
    let budget = ems - face.char_ems(ELLIPSIS);
    let mut used = 0.0;
    let kept: String = text
        .chars()
        .take_while(|&c| {
            used += face.char_ems(c);
            used <= budget
        })
        .collect();
    format!("{}{ELLIPSIS}", kept.trim_end_matches([' ', '·']))
}

/// One character's advance in ems: the widest of its class in DejaVu Sans,
/// rounded up. Anything outside ASCII letters, digits and common
/// punctuation (and the `·` the tables separate names with) counts a full
/// em.
fn char_ems(c: char) -> f64 {
    match c {
        'i' | 'j' | 'l' | 'f' | 't' | 'r' | 'I' | 'J' | ' ' | '.' | ',' | ':' | ';' | '\''
        | '!' | '|' | '-' | '(' | ')' | '[' | ']' | '/' | '\\' | '·' => 0.42,
        'M' | 'W' | 'm' | 'w' | '@' | '%' | '+' | '=' | '<' | '>' | '~' | '&' | '#' => 1.0,
        'a'..='z' | '0'..='9' | 'E' | 'F' | 'L' | 'P' | 'S' | 'T' | 'Y' | '_' => 0.64,
        'A' | 'B' | 'C' | 'K' | 'R' | 'V' | 'X' | 'Z' => 0.7,
        'A'..='Z' | '$' | '*' | '?' | '"' => 0.79,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::fonts;

    impl Face {
        /// The bundled face this face's text is drawn in.
        fn bytes(self) -> &'static [u8] {
            match self {
                Self::Regular => fonts::REGULAR,
                Self::Bold => fonts::BOLD,
                Self::Oblique => fonts::OBLIQUE,
            }
        }

        /// Whether the bundled face has a glyph for `c`.
        fn draws(self, c: char) -> bool {
            let face = ttf_parser::Face::parse(self.bytes(), 0).expect("a bundled face parses");
            face.glyph_index(c).is_some()
        }
    }

    impl Font {
        /// How wide PNG and PDF draw `text` in this font, in px: the sum of
        /// the bundled face's advances, its `.notdef` box's for a character
        /// it has no glyph for. Unkerned: DejaVu Sans's kerning only pulls
        /// pairs closer.
        pub(in crate::render) fn dejavu_width(&self, text: &str) -> f64 {
            let face =
                ttf_parser::Face::parse(self.face.bytes(), 0).expect("a bundled face parses");
            let units: u32 = text
                .chars()
                .map(|c| {
                    let glyph = face.glyph_index(c).unwrap_or(ttf_parser::GlyphId(0));
                    u32::from(
                        face.glyph_hor_advance(glyph)
                            .expect("every glyph has an advance"),
                    )
                })
                .sum();
            f64::from(units) * self.size / f64::from(face.units_per_em())
        }
    }

    const FACES: [Face; 3] = [Face::Regular, Face::Bold, Face::Oblique];

    #[test]
    fn every_character_class_bounds_dejavu_sans_in_each_face() {
        let chars = (' '..='~').chain("·…²→×µΩ°±".chars());
        for c in chars {
            for face in FACES {
                let font = Font { size: 100.0, face };
                let (estimate, drawn) = (
                    font.width(&c.to_string()),
                    font.dejavu_width(&c.to_string()),
                );
                assert!(
                    estimate >= drawn,
                    "{c:?} in {face:?}: estimated {estimate}, drawn {drawn}"
                );
            }
        }
    }

    #[test]
    fn known_strings_measure_at_least_their_dejavu_width_and_not_far_past_it() {
        for (text, font, drawn) in [
            ("High-mount stop lamp", Font::bold(13.0), 163.81),
            ("OBC/DC-DC", Font::bold(13.0), 81.31),
            ("x1 · Lamp 2p", Font::regular(10.0), 65.49),
            (
                "Shielded twisted pair · 2 m · ×3",
                Font::regular(10.0),
                158.57,
            ),
            ("Pair shield", Font::italic(9.0), 46.65),
        ] {
            assert!(
                (font.dejavu_width(text) - drawn).abs() < 0.01,
                "{text:?} in {font:?} draws {} wide",
                font.dejavu_width(text)
            );
            let estimate = font.width(text);
            assert!(
                (drawn..=drawn * 1.3).contains(&estimate),
                "{text:?} in {font:?}: estimated {estimate}, drawn {drawn}"
            );
        }
    }

    #[test]
    fn italic_sets_as_wide_as_regular() {
        let text = "Rear lamp ground";
        assert_eq!(
            Font::italic(9.0).width(text),
            Font::regular(9.0).width(text)
        );
        assert_eq!(
            Font::italic(9.0).dejavu_width(text),
            Font::regular(9.0).dejavu_width(text)
        );
    }

    #[test]
    fn a_character_no_face_draws_counts_a_full_em() {
        let han = '漢';
        for face in FACES {
            assert!(!face.draws(han), "{face:?} has no glyph for {han}");
            let font = Font { size: 10.0, face };
            let estimate = font.width(&han.to_string());
            // Wider than the empty box PNG and PDF draw, and as wide as
            // a viewer's fallback CJK glyph, a full em.
            assert!(estimate >= font.dejavu_width(&han.to_string()));
            assert!(estimate >= 10.0, "{face:?}: {estimate}");
        }
    }

    #[test]
    fn a_cut_never_ends_on_a_separator() {
        let font = Font::regular(9.0);
        let text = "WAKE · 0.5mm² · WH";
        let fitted = font.fit(text, font.width("WAKE · 0"));
        assert_eq!(fitted.text, "WAKE…");
        assert!(font.width(&fitted.text) <= font.width("WAKE · 0"));
    }

    impl FittedLabel {
        /// The label's estimated drawn width, in px.
        pub(in crate::render) fn width(&self) -> f64 {
            Face::Regular.ems(&self.text) * self.size
        }
    }

    #[test]
    fn a_label_that_fits_keeps_its_size() {
        let fitted = FittedLabel::new("EX+", 32.0);
        assert_eq!(fitted.text, "EX+");
        assert_eq!(fitted.size, LABEL_SIZE);
        assert!(!fitted.is_shrunk() && !fitted.is_cut());
    }

    #[test]
    fn a_long_label_shrinks_in_steps_until_it_fits() {
        let fitted = FittedLabel::new("IGNITION", 32.0);
        assert_eq!(fitted.text, "IGNITION");
        assert!(fitted.is_shrunk() && !fitted.is_cut());
        assert!(fitted.size >= MIN_LABEL_SIZE);
        assert_eq!(fitted.size % SIZE_STEP, 0.0);
        assert!(fitted.width() <= 32.0);
        // The next step up would not have fitted.
        assert!(Face::Regular.ems("IGNITION") * (fitted.size + SIZE_STEP) > 32.0);
    }

    #[test]
    fn a_label_too_long_for_the_floor_size_is_cut_with_an_ellipsis() {
        let fitted = FittedLabel::new("RESOLVER SHIELD DRAIN", 32.0);
        assert_eq!(fitted.size, MIN_LABEL_SIZE);
        assert!(fitted.is_cut());
        assert!(fitted.text.starts_with("RES"));
        assert!(fitted.width() <= 32.0, "{} is too wide", fitted.text);
    }

    #[test]
    fn a_cut_does_not_leave_a_space_before_the_ellipsis() {
        // At the floor size the space after the third W is the last
        // character that fits.
        let fitted = FittedLabel::new("WWW WWW", 32.0);
        assert_eq!(fitted.text, "WWW…");
        assert!(fitted.width() <= 32.0);
    }

    #[test]
    fn no_room_at_all_leaves_only_the_ellipsis() {
        let fitted = FittedLabel::new("WWW", 4.0);
        assert_eq!(fitted.text, "…");
    }

    #[test]
    fn table_text_that_fits_its_column_is_kept_whole_at_its_size() {
        let font = Font::regular(10.0);
        let fitted = font.fit("→ Vehicle.ecu.pwr", 200.0);
        assert_eq!(fitted.text, "→ Vehicle.ecu.pwr");
        assert_eq!(fitted.size, 10.0);
        assert!(!fitted.is_cut());
    }

    #[test]
    fn table_text_past_its_column_is_cut_never_shrunk() {
        let font = Font::regular(10.0);
        let fitted = font.fit("→ Vehicle.front_sensor.sig (x1:1)", 100.0);
        assert!(fitted.is_cut(), "{}", fitted.text);
        assert_eq!(fitted.size, 10.0);
        assert!(fitted.width() <= 100.0, "{} is too wide", fitted.text);
        assert!(fitted.text.starts_with("→ Vehicle"));
    }

    #[test]
    fn bold_text_counts_wider_than_regular() {
        let text = "Inverter · control";
        assert!(Font::bold(13.0).width(text) > Font::regular(13.0).width(text));
        assert_eq!(
            Font::regular(10.0).width(text),
            Face::Regular.ems(text) * 10.0
        );
    }

    #[test]
    fn wide_letters_count_wider_than_narrow_ones() {
        assert!(Face::Regular.ems("WWWW") > Face::Regular.ems("IIII") * 2.0);
        assert!(Face::Regular.ems("ABCD") > Face::Regular.ems("abcd"));
    }
}
