//! The characters the bundled faces draw wider than an em, which would
//! overrun [`super::FALLBACK_EMS`]: generated from the faces by the tests
//! (`bound::the_wide_table_is_derived_from_the_faces` prints the table
//! when the faces and it disagree). Each character's need is the widest
//! glyph any face draws for it (a bold one divided by the bold widening
//! the estimate applies after), rounded up to a quarter em; a run spans
//! neighbouring wide characters with no narrower character a face draws
//! between them, at the widest need among them.

use std::cmp::Ordering;

/// A run of codepoints, `first..=last`, that count `ems` each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WideRun {
    pub(super) first: char,
    pub(super) last: char,
    pub(super) ems: f64,
}

impl WideRun {
    const fn new(first: char, last: char, ems: f64) -> Self {
        Self { first, last, ems }
    }

    /// What `c` counts in ems when it is a wide character.
    pub(super) fn ems_of(c: char) -> Option<f64> {
        WIDE.binary_search_by(|run| {
            if run.last < c {
                Ordering::Less
            } else if run.first > c {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .ok()
        .map(|i| WIDE[i].ems)
    }
}

/// The wide characters, in codepoint order.
pub(super) const WIDE: &[WideRun] = &[
    WideRun::new('\u{0152}', '\u{0153}', 1.25),
    WideRun::new('\u{01C4}', '\u{01C6}', 1.5),
    WideRun::new('\u{01CA}', '\u{01CB}', 1.25),
    WideRun::new('\u{01F1}', '\u{01F3}', 1.5),
    WideRun::new('\u{01F6}', '\u{01F6}', 1.25),
    WideRun::new('\u{02A3}', '\u{02A5}', 1.25),
    WideRun::new('\u{0409}', '\u{040A}', 1.25),
    WideRun::new('\u{0416}', '\u{0416}', 1.25),
    WideRun::new('\u{0428}', '\u{0429}', 1.25),
    WideRun::new('\u{042E}', '\u{042E}', 1.25),
    WideRun::new('\u{0468}', '\u{0469}', 1.25),
    WideRun::new('\u{046C}', '\u{046C}', 1.25),
    WideRun::new('\u{047C}', '\u{047D}', 1.25),
    WideRun::new('\u{0496}', '\u{0496}', 1.25),
    WideRun::new('\u{04A4}', '\u{04A4}', 1.25),
    WideRun::new('\u{04A6}', '\u{04A6}', 1.25),
    WideRun::new('\u{04C1}', '\u{04C1}', 1.25),
    WideRun::new('\u{04DC}', '\u{04DC}', 1.25),
    WideRun::new('\u{0502}', '\u{0502}', 1.25),
    WideRun::new('\u{0508}', '\u{0508}', 1.25),
    WideRun::new('\u{050A}', '\u{050A}', 1.25),
    WideRun::new('\u{0514}', '\u{0514}', 1.25),
    WideRun::new('\u{0518}', '\u{0518}', 1.25),
    WideRun::new('\u{0520}', '\u{0520}', 1.25),
    WideRun::new('\u{0522}', '\u{0522}', 1.25),
    WideRun::new('\u{060A}', '\u{060A}', 1.25),
    WideRun::new('\u{0633}', '\u{0636}', 1.5),
    WideRun::new('\u{0641}', '\u{0641}', 1.25),
    WideRun::new('\u{069A}', '\u{069E}', 1.5),
    WideRun::new('\u{06A1}', '\u{06A6}', 1.25),
    WideRun::new('\u{06AA}', '\u{06AA}', 1.25),
    WideRun::new('\u{0EDC}', '\u{0EDD}', 1.25),
    WideRun::new('\u{10DA}', '\u{10DA}', 1.25),
    WideRun::new('\u{1413}', '\u{1413}', 1.25),
    WideRun::new('\u{1415}', '\u{1415}', 1.25),
    WideRun::new('\u{1418}', '\u{1418}', 1.25),
    WideRun::new('\u{141A}', '\u{141A}', 1.25),
    WideRun::new('\u{142B}', '\u{142B}', 1.25),
    WideRun::new('\u{142D}', '\u{142E}', 1.25),
    WideRun::new('\u{1441}', '\u{1441}', 1.25),
    WideRun::new('\u{1443}', '\u{1443}', 1.25),
    WideRun::new('\u{1445}', '\u{1445}', 1.25),
    WideRun::new('\u{1447}', '\u{1447}', 1.25),
    WideRun::new('\u{14C9}', '\u{14CE}', 1.25),
    WideRun::new('\u{14DC}', '\u{14E9}', 1.25),
    WideRun::new('\u{1517}', '\u{1524}', 1.25),
    WideRun::new('\u{155B}', '\u{155C}', 1.25),
    WideRun::new('\u{157E}', '\u{1584}', 1.25),
    WideRun::new('\u{158E}', '\u{1594}', 1.5),
    WideRun::new('\u{1596}', '\u{1596}', 1.25),
    WideRun::new('\u{166F}', '\u{1676}', 2.0),
    WideRun::new('\u{1683}', '\u{1685}', 1.75),
    WideRun::new('\u{1688}', '\u{168A}', 1.75),
    WideRun::new('\u{168D}', '\u{168F}', 1.75),
    WideRun::new('\u{1692}', '\u{1694}', 1.75),
    WideRun::new('\u{1698}', '\u{1699}', 1.5),
    WideRun::new('\u{1D14}', '\u{1D14}', 1.25),
    WideRun::new('\u{1F2A}', '\u{1F2D}', 1.25),
    WideRun::new('\u{1F4A}', '\u{1F4B}', 1.25),
    WideRun::new('\u{1F5B}', '\u{1F5D}', 1.25),
    WideRun::new('\u{1F6A}', '\u{1F6B}', 1.25),
    WideRun::new('\u{1F9A}', '\u{1F9D}', 1.25),
    WideRun::new('\u{1FAA}', '\u{1FAB}', 1.25),
    WideRun::new('\u{2030}', '\u{2031}', 1.75),
    WideRun::new('\u{20A7}', '\u{20A8}', 1.5),
    WideRun::new('\u{20AF}', '\u{20AF}', 1.5),
    WideRun::new('\u{2100}', '\u{2101}', 1.25),
    WideRun::new('\u{2103}', '\u{2103}', 1.25),
    WideRun::new('\u{2105}', '\u{2106}', 1.25),
    WideRun::new('\u{2116}', '\u{2116}', 1.25),
    WideRun::new('\u{2120}', '\u{2121}', 1.25),
    WideRun::new('\u{2133}', '\u{2133}', 1.25),
    WideRun::new('\u{213B}', '\u{213B}', 1.25),
    WideRun::new('\u{2152}', '\u{2152}', 1.5),
    WideRun::new('\u{2166}', '\u{2167}', 1.5),
    WideRun::new('\u{216B}', '\u{216B}', 1.25),
    WideRun::new('\u{2176}', '\u{2177}', 1.5),
    WideRun::new('\u{217B}', '\u{217B}', 1.25),
    WideRun::new('\u{2180}', '\u{2180}', 1.25),
    WideRun::new('\u{2182}', '\u{2182}', 1.25),
    WideRun::new('\u{222D}', '\u{222D}', 1.25),
    WideRun::new('\u{2230}', '\u{2230}', 1.25),
    WideRun::new('\u{226A}', '\u{226B}', 1.25),
    WideRun::new('\u{22D8}', '\u{22D9}', 1.5),
    WideRun::new('\u{22F2}', '\u{22F2}', 1.25),
    WideRun::new('\u{22FA}', '\u{22FA}', 1.25),
    WideRun::new('\u{2324}', '\u{232B}', 1.5),
    WideRun::new('\u{2387}', '\u{2387}', 1.25),
    WideRun::new('\u{25EF}', '\u{25EF}', 1.25),
    WideRun::new('\u{260D}', '\u{260F}', 1.5),
    WideRun::new('\u{2639}', '\u{263B}', 1.25),
    WideRun::new('\u{26A2}', '\u{26A4}', 1.25),
    WideRun::new('\u{27F4}', '\u{27FF}', 1.5),
    WideRun::new('\u{2A0C}', '\u{2A0C}', 1.5),
    WideRun::new('\u{2B24}', '\u{2B24}', 1.25),
    WideRun::new('\u{2C72}', '\u{2C72}', 1.25),
    WideRun::new('\u{A64C}', '\u{A650}', 1.25),
    WideRun::new('\u{A654}', '\u{A654}', 1.25),
    WideRun::new('\u{A662}', '\u{A662}', 1.25),
    WideRun::new('\u{A664}', '\u{A664}', 1.25),
    WideRun::new('\u{A666}', '\u{A667}', 1.25),
    WideRun::new('\u{A66C}', '\u{A66D}', 1.5),
    WideRun::new('\u{A698}', '\u{A699}', 1.5),
    WideRun::new('\u{A732}', '\u{A732}', 1.25),
    WideRun::new('\u{A734}', '\u{A734}', 1.25),
    WideRun::new('\u{A736}', '\u{A736}', 1.25),
    WideRun::new('\u{A74E}', '\u{A74F}', 1.5),
    WideRun::new('\u{A7FF}', '\u{A7FF}', 1.25),
    WideRun::new('\u{F40A}', '\u{F40A}', 1.25),
    WideRun::new('\u{FB13}', '\u{FB17}', 1.75),
    WideRun::new('\u{FB6A}', '\u{FB6B}', 1.25),
    WideRun::new('\u{FB6E}', '\u{FB6F}', 1.25),
    WideRun::new('\u{FEB1}', '\u{FEB2}', 1.5),
    WideRun::new('\u{FEB5}', '\u{FEB6}', 1.5),
    WideRun::new('\u{FEB9}', '\u{FEBA}', 1.25),
    WideRun::new('\u{FEBD}', '\u{FEBE}', 1.25),
    WideRun::new('\u{FED1}', '\u{FED2}', 1.25),
    WideRun::new('\u{FFFD}', '\u{FFFD}', 1.25),
    WideRun::new('\u{1030C}', '\u{1030C}', 1.5),
    WideRun::new('\u{1D544}', '\u{1D544}', 1.25),
    WideRun::new('\u{1D54E}', '\u{1D54E}', 1.25),
    WideRun::new('\u{1D55E}', '\u{1D55E}', 1.25),
    WideRun::new('\u{1EE0E}', '\u{1EE0E}', 1.25),
    WideRun::new('\u{1EE10}', '\u{1EE11}', 1.25),
    WideRun::new('\u{1EE14}', '\u{1EE14}', 1.25),
    WideRun::new('\u{1EE19}', '\u{1EE19}', 1.25),
    WideRun::new('\u{1EE1E}', '\u{1EE1E}', 1.25),
    WideRun::new('\u{1EE68}', '\u{1EE68}', 1.25),
    WideRun::new('\u{1EE6E}', '\u{1EE6E}', 1.25),
    WideRun::new('\u{1EE71}', '\u{1EE71}', 1.25),
    WideRun::new('\u{1EE74}', '\u{1EE74}', 1.25),
    WideRun::new('\u{1EE79}', '\u{1EE7A}', 1.25),
    WideRun::new('\u{1F030}', '\u{1F061}', 1.5),
    WideRun::new('\u{1F0A0}', '\u{1F643}', 1.75),
];
