//! The rows of a pinout's pin table, read off the connector's
//! [`PinTable`]: one per pin a port is bound to, in pin order, then one per
//! port with no pin, each with everything crimped into its cavity — what a
//! harness is crimped from.

use std::fmt;

use crate::dsl::ir::{Instance, Pin, Port, Screened, WireColor};
use crate::export::{CavityDrain, CavityEntry, Contact, FarEnd, FarSplice, PinTable, PortEnd};

use super::fit::{FittedLabel, Font};

/// One row of a pin table: a pin a port is bound to, or a port with no pin
/// yet, the port's label, and what is crimped into the cavity, one entry
/// (a sub-row) each.
pub(super) struct Row<'d> {
    /// `None` for a port the connector assigns no pin.
    pub(super) pin: Option<Pin>,
    pub(super) label: String,
    pub(super) note: Option<Note>,
    /// In wire-list order: none for a pin no conductor reaches, two for a
    /// double crimp.
    pub(super) entries: Vec<Entry<'d>>,
}

impl<'d> Row<'d> {
    /// `table`'s rows: one per pin a port is bound to, sorted by pin (a
    /// ganged port has one on each of its pins, its conductors listed on
    /// each), then its ports with no pin in source order. A cavity no port
    /// uses has no row: the face shows it.
    pub(super) fn all(table: &PinTable<'d>) -> Vec<Self> {
        let pinned = table.cavities.iter().filter_map(|cavity| {
            let port = cavity.port?;
            Some(Self {
                pin: Some(cavity.pin),
                label: Self::label_of(port),
                note: cavity.is_ganged().then(|| Note::ganged(port)),
                entries: cavity.entries.iter().map(Entry::of).collect(),
            })
        });
        let unpinned = table.unpinned.iter().map(|unpinned| Self {
            pin: None,
            label: Self::label_of(unpinned.port),
            note: Some(Note::Unpinned),
            entries: unpinned.entries.iter().map(Entry::of).collect(),
        });
        pinned.chain(unpinned).collect()
    }

    /// The port as a row names it: its name, and its label when that says
    /// something more.
    fn label_of(port: &Port) -> String {
        if port.label == port.name.as_str() {
            port.label.clone()
        } else {
            format!("{} · {}", port.name, port.label)
        }
    }
}

/// What a row says about its port beside the label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Note {
    /// The port occupies several cavities (`pins [46, 48]`): all of its
    /// pins, ascending. Its conductors are listed on each of its rows, so
    /// the mark tells a reader they are one set, not one per row.
    Ganged(Vec<Pin>),
    /// The connector assigns the port no pin yet.
    Unpinned,
}

impl Note {
    fn ganged(port: &Port) -> Self {
        let mut pins = port.pins.clone();
        pins.sort_unstable();
        Self::Ganged(pins)
    }

    /// The note's classes: `pin-note` and its kind's.
    pub(super) fn class(&self) -> &'static str {
        match self {
            Self::Ganged(_) => "pin-note pin-ganged",
            Self::Unpinned => "pin-note pin-unpinned",
        }
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ganged(pins) => {
                let pins: Vec<String> = pins.iter().map(Pin::to_string).collect();
                write!(f, "ganged {}", pins.join("+"))
            }
            Self::Unpinned => f.write_str("no pin"),
        }
    }
}

/// One thing crimped into a cavity: a conductor, with its colour, gauge
/// and far end, or a screen's drain, which has neither colour nor gauge.
pub(super) enum Entry<'d> {
    Conductor {
        color: &'d WireColor,
        gauge: f64,
        far: FarText,
    },
    Drain {
        far: FarText,
    },
}

impl<'d> Entry<'d> {
    fn of(entry: &CavityEntry<'d>) -> Self {
        match entry {
            CavityEntry::Conductor(conductor) => {
                let wire = conductor.conductor.wire;
                Self::Conductor {
                    color: &wire.color,
                    gauge: wire.gauge,
                    far: FarText::of_conductor(&conductor.far),
                }
            }
            CavityEntry::Drain(drain) => Self::Drain {
                far: FarText::of_drain(drain),
            },
        }
    }

    /// The gauge as the table prints it, as the harness annotates a wire
    /// (`0.5mm²`); a drain has none.
    pub(super) fn gauge(&self) -> Option<String> {
        match self {
            Self::Conductor { gauge, .. } => Some(format!("{gauge}mm²")),
            Self::Drain { .. } => None,
        }
    }

    pub(super) fn far(&self) -> &FarText {
        match self {
            Self::Conductor { far, .. } | Self::Drain { far } => far,
        }
    }
}

/// Where an entry goes, as the texts it can be printed as, most detailed
/// first: a column too narrow for one takes the next, and the last is cut.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct FarText {
    /// Never empty.
    texts: Vec<String>,
}

impl FarText {
    /// A conductor's far end: the port's full path, then the connector and
    /// pin it is crimped into when it is on one; for a splice, its path,
    /// then what its net joins beyond it, named from the splice's
    /// component, collapsed to `+N more` as room runs out.
    fn of_conductor(far: &FarEnd<'_>) -> Self {
        let texts = match far {
            FarEnd::Port(end) | FarEnd::Boundary(end) => Self::port_texts(end),
            FarEnd::Splice(splice) => Self::splice_texts(splice),
            FarEnd::Unresolved(end) => vec![format!("→ {end}")],
        };
        Self { texts }
    }

    /// A drain's other end, for a screen grounded at both ends, and which
    /// screen it is: its cable and its label, else whether it is the
    /// cable's overall screen or a pair's (numbered from 1, as the export
    /// numbers pairs), so two drains in one cavity read apart.
    fn of_drain(drain: &CavityDrain<'_>) -> Self {
        let cable = drain.cable.name;
        let shield = match (&drain.shield.shield.label, drain.shield.screened) {
            (Some(label), _) => format!("{cable} · {label}"),
            (None, Screened::Cable) => format!("{cable} shield"),
            (None, Screened::Pair(pair)) => format!("{cable} pair {} shield", pair + 1),
        };
        let texts = match drain.far {
            Some(end) => vec![format!("→ {end} ({shield})"), format!("→ {end}")],
            None => vec![shield],
        };
        Self { texts }
    }

    fn port_texts(end: &PortEnd<'_>) -> Vec<String> {
        let path = format!("→ {}.{}", end.instance.path, end.port.name);
        match end.contact() {
            Contact::Connector { designator, pins } => {
                let mut pins = pins.to_vec();
                pins.sort_unstable();
                let pins: Vec<String> = pins.iter().map(Pin::to_string).collect();
                let at = if pins.is_empty() {
                    designator.to_string()
                } else {
                    format!("{designator}:{}", pins.join("+"))
                };
                vec![format!("{path} ({at})"), path]
            }
            Contact::Connectorless => vec![path],
        }
    }

    fn splice_texts(far: &FarSplice<'_>) -> Vec<String> {
        let owner = far.splice.owner;
        let name = format!("→ {}.{}", owner.path, far.splice.splice.name);
        let beyond: Vec<String> = far
            .beyond
            .iter()
            .map(|spliced| Self::relative_to(owner, &spliced.end.to_string()))
            .collect();
        let mut texts: Vec<String> = (1..=beyond.len())
            .rev()
            .map(|shown| {
                let mut listed = beyond[..shown].to_vec();
                if shown < beyond.len() {
                    listed.push(format!("+{} more", beyond.len() - shown));
                }
                format!("{name} ({})", listed.join(", "))
            })
            .collect();
        texts.push(name);
        texts
    }

    /// `path`, named from inside `owner` when it lies inside it.
    fn relative_to(owner: &Instance, path: &str) -> String {
        let prefix = format!("{}.", owner.path);
        path.strip_prefix(&prefix).unwrap_or(path).to_string()
    }

    /// The most detailed text.
    pub(super) fn full(&self) -> &str {
        self.texts.first().map_or("", String::as_str)
    }

    /// The most detailed text that fits `width` px in `font`, else the
    /// least detailed one cut to it.
    pub(super) fn fit(&self, font: Font, width: f64) -> FittedLabel {
        let fitting = self.texts.iter().find(|text| font.width(text) <= width);
        let text = fitting.or(self.texts.last()).map_or("", String::as_str);
        font.fit(text, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn far(texts: &[&str]) -> FarText {
        FarText {
            texts: texts.iter().map(|text| text.to_string()).collect(),
        }
    }

    #[test]
    fn a_far_end_takes_the_most_detailed_text_that_fits() {
        let font = Font::regular(10.0);
        let text = far(&["→ Vehicle.lamp.gnd (x1:2)", "→ Vehicle.lamp.gnd"]);
        let roomy = font.width(text.full());
        assert_eq!(text.fit(font, roomy).text, "→ Vehicle.lamp.gnd (x1:2)");
        let short = font.width("→ Vehicle.lamp.gnd");
        assert_eq!(text.fit(font, short).text, "→ Vehicle.lamp.gnd");
        let cut = text.fit(font, short - 10.0);
        assert!(cut.is_cut(), "{}", cut.text);
        assert!(cut.width() <= short - 10.0);
    }

    #[test]
    fn notes_name_every_ganged_pin_or_the_missing_pin() {
        assert_eq!(
            Note::Ganged(vec![Pin(46), Pin(48)]).to_string(),
            "ganged 46+48"
        );
        assert_eq!(Note::Unpinned.to_string(), "no pin");
        assert_eq!(Note::Unpinned.class(), "pin-note pin-unpinned");
    }
}
