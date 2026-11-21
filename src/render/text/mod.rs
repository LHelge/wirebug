//! How wide text is drawn, and fitting it into the width it is given: a
//! harness table's header and columns, a pinout's cavity label into its
//! cavity, a table cell's text into its column. Shared by the renderers,
//! so every one measures alike.
//!
//! SVG text has no clipping and no layout: a label is as wide as its font
//! makes it. PNG and PDF draw it in the bundled DejaVu Sans
//! ([`super::fonts`]); an SVG viewer draws it in whatever `sans-serif`
//! resolves to on its machine. So the width is an upper bound, per
//! character, on how wide the bundled face the text is set in draws it,
//! shaping included; DejaVu Sans is the widest of the common sans-serif
//! faces (wider than Arial, Liberation Sans and Helvetica), so the bound
//! holds for those viewers too.
//!
//! The tests prove the bound from the faces themselves, for every
//! character any of them has a glyph for: each character's estimate is
//! at least the widest glyph its face draws for it, after the default
//! substitutions (Arabic positional forms, localised forms); no default
//! ligature is wider than its parts; and the slack the estimates leave
//! covers every pair the faces' kerning moves apart, but for the few pairs
//! in [`KERNED_APART`], which add an allowance of their own. ASCII and
//! `·` fall in hand-made classes; a wider character in [`wide::WIDE`],
//! a table the tests derive from the faces; anything else counts a full
//! em, which no other glyph of the faces exceeds, which is wider than the
//! box PNG and PDF draw for a character they have no glyph for, and as
//! wide as a viewer's fallback CJK glyph. An estimate that errs wide
//! shrinks a label a little early or widens a box; one that errs narrow
//! lets text cross a border.

#[cfg(test)]
mod bound;
mod wide;

use wide::WideRun;

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
/// What a character outside the classes and [`wide::WIDE`] counts, in
/// ems.
const FALLBACK_EMS: f64 = 1.0;
/// The pairs a face's kerning moves further apart than the slack their two
/// characters' estimates leave. Derived from the faces by the tests: each
/// character's slack goes first to the kerning into it, then to the
/// kerning out of it, and these are the pairs left short.
const KERNED_APART: [(Face, char, char); 3] = [
    (Face::Regular, '-', 'Q'),
    (Face::Regular, 'A', 'A'),
    (Face::Oblique, 'C', 'S'),
];
/// What a pair in [`KERNED_APART`] adds to the estimate, in ems (before a
/// bold face's widening): rounded up from the most one is left short,
/// `AA`'s 0.0119 em.
const KERN_ALLOWANCE: f64 = 0.02;

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

    /// What kerning the pair `(a, b)` beyond the slack of their
    /// estimates adds, in ems, before a bold face's widening.
    fn kerned_apart(self, (a, b): (char, char)) -> Option<f64> {
        KERNED_APART
            .contains(&(self, a, b))
            .then_some(KERN_ALLOWANCE)
    }

    /// `text`'s advance in ems in this face, before a bold face's
    /// widening.
    fn ems(self, text: &str) -> f64 {
        let advances: f64 = text.chars().map(|c| self.char_ems(c)).sum();
        text.chars()
            .zip(text.chars().skip(1))
            .filter_map(|pair| self.kerned_apart(pair))
            .fold(advances, |ems, allowance| ems + allowance)
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
    let mut previous = None;
    let kept: String = text
        .chars()
        .take_while(|&c| {
            used += face.char_ems(c);
            if let Some(allowance) = previous.and_then(|p| face.kerned_apart((p, c))) {
                used += allowance;
            }
            previous = Some(c);
            match face.kerned_apart((c, ELLIPSIS)) {
                Some(allowance) => used + allowance <= budget,
                None => used <= budget,
            }
        })
        .collect();
    format!("{}{ELLIPSIS}", kept.trim_end_matches([' ', '·']))
}

/// One character's advance in ems in the regular face: its class's, a
/// wide character's from [`wide::WIDE`], else [`FALLBACK_EMS`].
fn char_ems(c: char) -> f64 {
    class_ems(c)
        .or_else(|| WideRun::ems_of(c))
        .unwrap_or(FALLBACK_EMS)
}

/// The advance of `c`'s class in ems, when it is in one: the widest of the
/// class in DejaVu Sans, rounded up. The classes hold the ASCII letters,
/// digits and punctuation, and the `·` the tables separate names with.
fn class_ems(c: char) -> Option<f64> {
    Some(match c {
        'i' | 'j' | 'l' | 'f' | 't' | 'r' | 'I' | 'J' | ' ' | '.' | ',' | ':' | ';' | '\''
        | '!' | '|' | '-' | '(' | ')' | '[' | ']' | '/' | '\\' | '·' => 0.42,
        'M' | 'W' | 'm' | 'w' | '@' | '%' | '+' | '=' | '<' | '>' | '~' | '&' | '#' => 1.0,
        'a'..='z' | '0'..='9' | 'E' | 'F' | 'L' | 'P' | 'S' | 'T' | 'Y' | '_' => 0.64,
        'A' | 'B' | 'C' | 'K' | 'R' | 'V' | 'X' | 'Z' => 0.7,
        'A'..='Z' | '$' | '*' | '?' | '"' => 0.79,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::fonts;

    impl Face {
        /// The bundled face this face's text is drawn in.
        pub(super) fn bytes(self) -> &'static [u8] {
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
        /// How wide `text`'s glyphs are in this font's bundled face, in px:
        /// the sum of their advances, its `.notdef` box's for a character
        /// it has no glyph for. Unshaped: what substitutions and kerning do
        /// on top is bounded by the estimate as `bound` proves.
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

    /// Wide glyphs: long arrows, a per-mille sign, `Œ`, Cyrillic capitals
    /// that set wider than an em in bold. Each was once counted an em and
    /// ran past its box.
    #[test]
    fn wide_characters_measure_at_least_as_wide_as_they_draw() {
        for text in ["S⟶E⟶T⟶S⟶E⟶T⟶S⟶E", "5‰ ‱ ₧ ℃ №", "Œuvre œ", "ЖШЩЮЉ"]
        {
            for face in FACES {
                let font = Font { size: 11.0, face };
                assert!(
                    font.width(text) >= font.dejavu_width(text),
                    "{text:?} in {face:?}: estimated {}, drawn {}",
                    font.width(text),
                    font.dejavu_width(text)
                );
            }
        }
    }

    /// A pair a face kerns apart further than its characters' estimates
    /// leave room for counts an allowance on top, and only that pair.
    #[test]
    fn a_pair_kerned_apart_counts_its_allowance() {
        let added = |font: Font, a: &str, b: &str| {
            font.width(&format!("{a}{b}")) - font.width(a) - font.width(b)
        };
        let (regular, italic) = (Font::regular(10.0), Font::italic(10.0));
        assert!((added(regular, "A", "A") - KERN_ALLOWANCE * 10.0).abs() < 1e-9);
        assert!(added(regular, "A", "B").abs() < 1e-9);
        assert!((added(italic, "C", "S") - KERN_ALLOWANCE * 10.0).abs() < 1e-9);
        assert!(added(regular, "C", "S").abs() < 1e-9);
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
