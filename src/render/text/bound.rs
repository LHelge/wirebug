//! The proof, from the bundled faces themselves, that [`Font::width`] is
//! never less than the width PNG and PDF draw text at, shaping included.
//!
//! A shaper draws a character as its own glyph or as one a default
//! substitution puts in its place (an Arabic positional form, a localised
//! form), joins some runs into a ligature, and moves neighbouring glyphs
//! closer or apart by the face's kerning. So each face is read as *units*:
//! every character it has a glyph for, drawn as any of the glyphs it can
//! become, and every default ligature, drawn in place of the characters it
//! joins. In each face, every unit's estimate is at least its widest glyph,
//! and the slack the estimates leave over those glyphs covers every pair of
//! units kerned apart, each unit's slack going first to the kerning into it
//! and the rest to the kerning out of it (so no slack counts twice along a
//! line), but for the pairs of [`KERNED_APART`], whose allowance covers the
//! rest. The faces' other default lookups only place marks, which add no
//! advance; a test holds them to that.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::ops::Bound::Excluded;

use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
use ttf_parser::gsub::{SingleSubstitution, SubstitutionSubtable};
use ttf_parser::opentype_layout::LayoutTable;
use ttf_parser::{GlyphId, Tag};

use super::wide::{WIDE, WideRun};
use super::{BOLD_WIDENING, FALLBACK_EMS, Face, Font, KERNED_APART, class_ems};

const FACES: [Face; 3] = [Face::Regular, Face::Bold, Face::Oblique];

/// Whether a shaper applies `feature` only when asked: a renderer drawing
/// an SVG `<text>` never asks.
fn optional(feature: Tag) -> bool {
    const OPTIONAL: [&str; 24] = [
        "aalt", "afrc", "c2sc", "case", "dlig", "dnom", "frac", "hist", "hlig", "lnum", "nalt",
        "numr", "onum", "ordn", "ornm", "pnum", "salt", "sinf", "smcp", "subs", "sups", "swsh",
        "titl", "zero",
    ];
    let tag = feature.to_string();
    OPTIONAL.contains(&tag.as_str()) || tag.starts_with("ss") || tag.starts_with("cv")
}

/// The lookups of `table` a shaper applies by default: every lookup but
/// those only optional features name. One no feature names is reached
/// from a contextual lookup, so it counts.
fn default_lookups(table: &LayoutTable) -> BTreeSet<u16> {
    let mut only_optional: HashMap<u16, bool> = HashMap::new();
    for feature in table.features {
        for lookup in feature.lookup_indices {
            *only_optional.entry(lookup).or_insert(true) &= optional(feature.tag);
        }
    }
    (0..table.lookups.len())
        .filter(|lookup| !only_optional.get(lookup).copied().unwrap_or(false))
        .collect()
}

/// What a shaper draws as one piece: a character, or a ligature of several.
#[derive(Debug)]
struct Unit {
    /// The text it stands for.
    text: String,
    /// Every glyph it can be drawn as.
    glyphs: BTreeSet<GlyphId>,
}

/// A bundled face, as a shaper draws it.
struct Drawn {
    face: Face,
    font: ttf_parser::Face<'static>,
    /// The glyph of every character the face has one for.
    glyphs: BTreeMap<char, GlyphId>,
    /// What default single substitutions turn each glyph into.
    replaced_by: HashMap<GlyphId, Vec<GlyphId>>,
}

impl Drawn {
    fn new(face: Face) -> Self {
        let font = ttf_parser::Face::parse(face.bytes(), 0).expect("a bundled face parses");
        let mut glyphs = BTreeMap::new();
        let cmap = font.tables().cmap.expect("a character map");
        for subtable in cmap.subtables.into_iter().filter(|s| s.is_unicode()) {
            subtable.codepoints(|codepoint| {
                if let (Some(c), Some(glyph)) =
                    (char::from_u32(codepoint), subtable.glyph_index(codepoint))
                {
                    glyphs.insert(c, glyph);
                }
            });
        }
        let mut drawn = Self {
            face,
            font,
            glyphs,
            replaced_by: HashMap::new(),
        };
        for subtable in drawn.substitutions() {
            let SubstitutionSubtable::Single(single) = subtable else {
                continue;
            };
            for from in (0..drawn.font.number_of_glyphs()).map(GlyphId) {
                let to = match single {
                    SingleSubstitution::Format1 { coverage, delta } => coverage
                        .contains(from)
                        .then(|| GlyphId(from.0.wrapping_add_signed(delta))),
                    SingleSubstitution::Format2 {
                        coverage,
                        substitutes,
                    } => coverage.get(from).and_then(|i| substitutes.get(i)),
                };
                if let Some(to) = to {
                    drawn.replaced_by.entry(from).or_default().push(to);
                }
            }
        }
        drawn
    }

    /// `glyph`'s advance, in ems.
    fn advance(&self, glyph: GlyphId) -> f64 {
        let units = self
            .font
            .glyph_hor_advance(glyph)
            .expect("a glyph's advance");
        f64::from(units) / f64::from(self.font.units_per_em())
    }

    /// `text`'s estimate in this face, in ems.
    fn estimate(&self, text: &str) -> f64 {
        Font {
            size: 1.0,
            face: self.face,
        }
        .width(text)
    }

    /// The default substitution subtables.
    fn substitutions(&self) -> Vec<SubstitutionSubtable<'static>> {
        let gsub = self.font.tables().gsub.expect("a substitution table");
        default_lookups(&gsub)
            .into_iter()
            .filter_map(|i| gsub.lookups.get(i))
            .flat_map(|lookup| lookup.subtables.into_iter::<SubstitutionSubtable>())
            .collect()
    }

    /// The default positioning subtables.
    fn positionings(&self) -> Vec<PositioningSubtable<'static>> {
        let gpos = self.font.tables().gpos.expect("a positioning table");
        default_lookups(&gpos)
            .into_iter()
            .filter_map(|i| gpos.lookups.get(i))
            .flat_map(|lookup| lookup.subtables.into_iter::<PositioningSubtable>())
            .collect()
    }

    /// Every glyph `glyph` becomes through a chain of default single
    /// substitutions, `glyph` included.
    fn becomes(&self, glyph: GlyphId) -> BTreeSet<GlyphId> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![glyph];
        while let Some(glyph) = pending.pop() {
            if seen.insert(glyph) {
                pending.extend(self.replaced_by.get(&glyph).into_iter().flatten());
            }
        }
        seen
    }

    /// Every unit of the face: each character it has a glyph for, then each
    /// default ligature its characters can form, standing for the text of
    /// least estimate that forms it.
    fn units(&self) -> Vec<Unit> {
        let mut units: Vec<Unit> = self
            .glyphs
            .iter()
            .map(|(&c, &glyph)| Unit {
                text: c.to_string(),
                glyphs: self.becomes(glyph),
            })
            .collect();
        // The narrowest-estimated character drawn as each glyph.
        let mut drawn_from: HashMap<GlyphId, char> = HashMap::new();
        for unit in &units {
            let c = unit.text.chars().next().expect("a character");
            for &glyph in &unit.glyphs {
                let narrowest = drawn_from.entry(glyph).or_insert(c);
                if self.estimate(&c.to_string()) < self.estimate(&narrowest.to_string()) {
                    *narrowest = c;
                }
            }
        }
        for (components, ligature) in self.ligatures() {
            let text: Option<String> = components
                .iter()
                .map(|glyph| drawn_from.get(glyph))
                .collect();
            if let Some(text) = text {
                units.push(Unit {
                    text,
                    glyphs: self.becomes(ligature),
                });
            }
        }
        units
    }

    /// The default ligatures, as their components and the glyph they join
    /// into.
    fn ligatures(&self) -> Vec<(Vec<GlyphId>, GlyphId)> {
        let mut ligatures = Vec::new();
        for subtable in self.substitutions() {
            let SubstitutionSubtable::Ligature(substitution) = subtable else {
                continue;
            };
            for first in (0..self.font.number_of_glyphs()).map(GlyphId) {
                let Some(set) = substitution
                    .coverage
                    .get(first)
                    .and_then(|i| substitution.ligature_sets.get(i))
                else {
                    continue;
                };
                for ligature in set {
                    let components = std::iter::once(first).chain(ligature.components);
                    ligatures.push((components.collect(), ligature.glyph));
                }
            }
        }
        ligatures
    }

    /// The kerning that moves a pair of `glyphs` apart, in ems, from the
    /// default pair positioning and, as a renderer without it would use,
    /// the legacy `kern` table: the larger where both kern a pair.
    fn kerned_apart(&self, glyphs: &BTreeSet<GlyphId>) -> HashMap<(GlyphId, GlyphId), f64> {
        let em = f64::from(self.font.units_per_em());
        let mut kerning: HashMap<(GlyphId, GlyphId), f64> = HashMap::new();
        let mut kern = |pair: (GlyphId, GlyphId), units: f64| {
            let ems = kerning.entry(pair).or_insert(f64::NEG_INFINITY);
            *ems = ems.max(units / em);
        };
        for subtable in self.positionings() {
            let PositioningSubtable::Pair(adjustment) = subtable else {
                continue;
            };
            match adjustment {
                PairAdjustment::Format1 { coverage, sets } => {
                    for &first in glyphs {
                        let Some(set) = coverage.get(first).and_then(|i| sets.get(i)) else {
                            continue;
                        };
                        for &second in glyphs {
                            if let Some((a, b)) = set.get(second) {
                                kern(
                                    (first, second),
                                    f64::from(a.x_advance) + f64::from(b.x_advance),
                                );
                            }
                        }
                    }
                }
                PairAdjustment::Format2 {
                    coverage,
                    classes,
                    matrix,
                } => {
                    let mut by_class: BTreeMap<u16, Vec<GlyphId>> = BTreeMap::new();
                    for &second in glyphs {
                        by_class
                            .entry(classes.1.get(second))
                            .or_default()
                            .push(second);
                    }
                    for &first in glyphs.iter().filter(|&&g| coverage.contains(g)) {
                        let class = classes.0.get(first);
                        for (&second_class, seconds) in &by_class {
                            let Some((a, b)) = matrix.get((class, second_class)) else {
                                continue;
                            };
                            let units = f64::from(a.x_advance) + f64::from(b.x_advance);
                            for &second in seconds {
                                kern((first, second), units);
                            }
                        }
                    }
                }
            }
        }
        if let Some(table) = self.font.tables().kern {
            for subtable in table.subtables.into_iter().filter(|s| s.horizontal) {
                if let ttf_parser::kern::Format::Format0(pairs) = subtable.format {
                    for pair in pairs.pairs {
                        if glyphs.contains(&pair.left()) && glyphs.contains(&pair.right()) {
                            kern((pair.left(), pair.right()), f64::from(pair.value));
                        }
                    }
                }
            }
        }
        kerning.retain(|_, ems| *ems > 0.0);
        kerning
    }
}

/// `unit`'s widest glyph in `drawn`'s face, in ems.
fn widest(drawn: &Drawn, unit: &Unit) -> f64 {
    unit.glyphs
        .iter()
        .map(|&glyph| drawn.advance(glyph))
        .fold(0.0, f64::max)
}

#[test]
fn every_character_and_ligature_measures_at_least_its_widest_glyph() {
    for face in FACES {
        let drawn = Drawn::new(face);
        let short: Vec<String> = drawn
            .units()
            .iter()
            .filter_map(|unit| {
                let (estimate, widest) = (drawn.estimate(&unit.text), widest(&drawn, unit));
                (estimate < widest).then(|| format!("{:?}: {estimate} < {widest}", unit.text))
            })
            .collect();
        assert!(
            short.is_empty(),
            "{face:?}: {} fall short: {short:#?}",
            short.len()
        );
    }
}

#[test]
fn a_character_no_face_draws_counts_more_than_the_box_drawn_for_it() {
    for face in FACES {
        let notdef = Drawn::new(face).advance(GlyphId(0));
        assert!(FALLBACK_EMS >= notdef, "{face:?}: .notdef is {notdef} em");
    }
}

#[test]
fn the_wide_table_is_derived_from_the_faces() {
    // Each wide character's need: the widest glyph any face draws for it,
    // in the estimate's ems (a bold face's divided by its widening).
    let mut needs: BTreeMap<char, f64> = BTreeMap::new();
    let mut drawn_by_any = BTreeSet::new();
    for face in FACES {
        let widening = match face {
            Face::Bold => BOLD_WIDENING,
            Face::Regular | Face::Oblique => 1.0,
        };
        let drawn = Drawn::new(face);
        for (&c, &glyph) in &drawn.glyphs {
            drawn_by_any.insert(c);
            let widest = drawn
                .becomes(glyph)
                .into_iter()
                .map(|glyph| drawn.advance(glyph))
                .fold(0.0, f64::max);
            if class_ems(c).is_none() && widest / widening > FALLBACK_EMS {
                let need = needs.entry(c).or_insert(0.0);
                *need = need.max(widest / widening);
            }
        }
    }
    let mut runs: Vec<WideRun> = Vec::new();
    for (&c, &need) in &needs {
        let ems = (need * 4.0).ceil() / 4.0;
        match runs.last_mut() {
            Some(run)
                if drawn_by_any
                    .range((Excluded(run.last), Excluded(c)))
                    .next()
                    .is_none() =>
            {
                run.last = c;
                run.ems = run.ems.max(ems);
            }
            _ => runs.push(WideRun {
                first: c,
                last: c,
                ems,
            }),
        }
    }
    let mut source = String::new();
    for run in &runs {
        writeln!(
            source,
            "    WideRun::new('\\u{{{:04X}}}', '\\u{{{:04X}}}', {:?}),",
            run.first as u32, run.last as u32, run.ems
        )
        .expect("a string takes writes");
    }
    assert!(
        WIDE == runs.as_slice(),
        "wide::WIDE is not what the faces give; it should read:\n{source}"
    );
}

#[test]
fn the_faces_default_lookups_widen_text_only_as_bounded_here() {
    for face in FACES {
        let drawn = Drawn::new(face);
        for subtable in drawn.substitutions() {
            assert!(
                matches!(
                    subtable,
                    SubstitutionSubtable::Single(_)
                        | SubstitutionSubtable::Ligature(_)
                        | SubstitutionSubtable::Context(_)
                        | SubstitutionSubtable::ChainContext(_)
                ),
                "{face:?}: an unbounded substitution {subtable:?}"
            );
        }
        for subtable in drawn.positionings() {
            assert!(
                matches!(
                    subtable,
                    PositioningSubtable::Pair(_)
                        | PositioningSubtable::MarkToBase(_)
                        | PositioningSubtable::MarkToLigature(_)
                        | PositioningSubtable::MarkToMark(_)
                ),
                "{face:?}: an unbounded positioning {subtable:?}"
            );
        }
    }
}

#[test]
fn kerning_moves_no_pair_apart_past_the_estimate() {
    for face in FACES {
        let drawn = Drawn::new(face);
        let units = drawn.units();
        let slack: Vec<f64> = units
            .iter()
            .map(|unit| drawn.estimate(&unit.text) - widest(&drawn, unit))
            .collect();
        let mut drawn_as: HashMap<GlyphId, Vec<usize>> = HashMap::new();
        for (i, unit) in units.iter().enumerate() {
            for &glyph in &unit.glyphs {
                drawn_as.entry(glyph).or_default().push(i);
            }
        }
        let glyphs: BTreeSet<GlyphId> = drawn_as.keys().copied().collect();
        // The most each pair of units is kerned apart.
        let mut apart: HashMap<(usize, usize), f64> = HashMap::new();
        for ((first, second), ems) in drawn.kerned_apart(&glyphs) {
            for &a in &drawn_as[&first] {
                for &b in &drawn_as[&second] {
                    let most = apart.entry((a, b)).or_insert(0.0);
                    *most = most.max(ems);
                }
            }
        }
        let mut into = vec![0.0_f64; units.len()];
        for (&(_, b), &ems) in &apart {
            into[b] = into[b].max(ems.min(slack[b]));
        }
        let mut short = BTreeSet::new();
        for (&(a, b), &ems) in &apart {
            let covered = (slack[a] - into[a]) + into[b];
            if ems > covered {
                let (first, second) = (&units[a].text, &units[b].text);
                let allowance = drawn.estimate(&format!("{first}{second}"))
                    - drawn.estimate(first)
                    - drawn.estimate(second);
                assert!(
                    covered + allowance >= ems,
                    "{face:?} {first:?}{second:?}: kerned {ems} em apart, covered {covered} + {allowance}"
                );
                short.insert((first.clone(), second.clone()));
            }
        }
        let listed: BTreeSet<(String, String)> = KERNED_APART
            .iter()
            .filter(|(f, _, _)| *f == face)
            .map(|&(_, a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(listed, short, "{face:?}: the pairs that need an allowance");
    }
}
