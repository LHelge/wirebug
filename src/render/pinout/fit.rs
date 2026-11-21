//! Fitting a cavity label into its cavity.
//!
//! SVG text has no clipping and no layout: a label is as wide as its font
//! makes it, and the font is whatever `sans-serif` resolves to on the
//! machine that draws it. So the width is estimated, per character class,
//! from the widest of the common sans-serif faces (DejaVu Sans, wider than
//! Arial/Liberation Sans and Helvetica): an estimate that errs wide shrinks
//! a label a little early, one that errs narrow lets it cross the border.

/// The cavity label's size when it fits, in px (the `.cavity-label` font).
pub(super) const LABEL_SIZE: f64 = 8.0;
/// The smallest size a label shrinks to before it is cut: below it a
/// printed label stops being readable.
pub(super) const MIN_LABEL_SIZE: f64 = 6.0;
/// Sizes step down by this much, so a drawing holds a few distinct sizes
/// rather than one per label.
const SIZE_STEP: f64 = 0.5;
const ELLIPSIS: char = '…';

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
        let ems = em_width(label);
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
        let budget = width / MIN_LABEL_SIZE - char_ems(ELLIPSIS);
        let mut used = 0.0;
        let kept: String = label
            .chars()
            .take_while(|&c| {
                used += char_ems(c);
                used <= budget
            })
            .collect();
        Self {
            text: format!("{}{ELLIPSIS}", kept.trim_end()),
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

/// `text`'s estimated advance, in ems.
fn em_width(text: &str) -> f64 {
    text.chars().map(char_ems).sum()
}

/// One character's advance in ems: the widest of its class in DejaVu Sans,
/// rounded up. Anything outside ASCII letters, digits and common
/// punctuation counts a full em.
fn char_ems(c: char) -> f64 {
    match c {
        'i' | 'j' | 'l' | 'f' | 't' | 'r' | 'I' | 'J' | ' ' | '.' | ',' | ':' | ';' | '\''
        | '!' | '|' | '-' | '(' | ')' | '[' | ']' | '/' | '\\' => 0.42,
        'M' | 'W' | 'm' | 'w' | '@' | '%' | '+' | '=' | '<' | '>' | '~' | '&' => 1.0,
        'a'..='z' | '0'..='9' | 'E' | 'F' | 'L' | 'P' | 'S' | 'T' | 'Y' | '_' => 0.64,
        'A' | 'B' | 'C' | 'K' | 'R' | 'V' | 'X' | 'Z' => 0.7,
        'A'..='Z' | '#' | '$' | '*' | '?' | '"' => 0.79,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl FittedLabel {
        /// The label's estimated drawn width, in px.
        pub(in crate::render::pinout) fn width(&self) -> f64 {
            em_width(&self.text) * self.size
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
        assert!(em_width("IGNITION") * (fitted.size + SIZE_STEP) > 32.0);
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
    fn wide_letters_count_wider_than_narrow_ones() {
        assert!(em_width("WWWW") > em_width("IIII") * 2.0);
        assert!(em_width("ABCD") > em_width("abcd"));
    }
}
