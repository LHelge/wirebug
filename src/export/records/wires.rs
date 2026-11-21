//! The wire list's records: one per conductor and per cable screen.

use serde::Serialize;

use super::ends::EndRecord;
use super::{Sheet, Table, field, pair_number};
use crate::dsl::ir::Screened;
use crate::export::{Cable, Row, RowKind, RowLength, WireList};

/// The wire list as written: `rows` in list order.
#[derive(Debug, Serialize)]
pub(super) struct WireSheet {
    rows: Vec<WireRecord>,
}

impl From<&WireList<'_>> for WireSheet {
    fn from(list: &WireList<'_>) -> Self {
        Self {
            rows: list.rows().iter().map(WireRecord::from).collect(),
        }
    }
}

/// One row of the wire list.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireRecord {
    Conductor(Box<ConductorRecord>),
    Shield(ShieldRecord),
}

/// A conductor: one segment of a wire.
#[derive(Debug, Serialize)]
struct ConductorRecord {
    /// The instance whose body declares the wire.
    owner: String,
    cable: Option<CableRecord>,
    /// The conductor's twisted pair in its cable, from 1.
    twisted_pair: Option<u32>,
    label: Option<String>,
    /// The canonical colour name, `green/yellow` for a two-tone wire.
    color: String,
    /// The IEC 60757 code, `GN/YE` for a two-tone wire.
    color_code: String,
    gauge_mm2: f64,
    #[serde(rename = "type")]
    wire_type: Option<String>,
    part: Option<String>,
    from: EndRecord,
    to: EndRecord,
    /// The length this row adds to a total, in metres: its own, a rail's
    /// whole length on the rail's first segment, else `None`. Summing it
    /// counts each rail once.
    length_m: Option<f64>,
    /// Set on each segment of a rail, a wire with more than two ends.
    rail: Option<RailRecord>,
}

/// A segment of a rail: which one, and the whole rail's length.
#[derive(Debug, Serialize)]
struct RailRecord {
    /// The segment's place in its wire, from 1.
    segment: usize,
    /// The whole rail's length, in metres.
    length_m: f64,
}

/// A cable screen: what it screens and where its drain lands.
#[derive(Debug, Serialize)]
struct ShieldRecord {
    owner: String,
    cable: Option<CableRecord>,
    screens: Screens,
    /// The pair it screens, from 1, for a pair's own screen.
    twisted_pair: Option<u32>,
    label: Option<String>,
    /// The ends its drain is terminated on: none, one or two.
    drain: Vec<EndRecord>,
}

/// What a screen screens.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Screens {
    /// The cable's overall screen.
    Cable,
    /// One twisted pair's own.
    Pair,
}

impl Screens {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Cable => "cable",
            Self::Pair => "pair",
        }
    }

    /// What `screened` screens, and the pair's number for a pair.
    pub(super) fn of(screened: Screened) -> (Self, Option<u32>) {
        match screened {
            Screened::Cable => (Self::Cable, None),
            Screened::Pair(group) => (Self::Pair, Some(pair_number(group))),
        }
    }
}

/// The cable a row runs in.
#[derive(Debug, Serialize)]
struct CableRecord {
    name: String,
    label: Option<String>,
    #[serde(rename = "type")]
    cable_type: Option<String>,
    length_m: Option<f64>,
}

impl From<Cable<'_>> for CableRecord {
    fn from(cable: Cable<'_>) -> Self {
        Self {
            name: cable.name.to_string(),
            label: cable.meta.label.clone(),
            cable_type: cable.meta.r#type.clone(),
            length_m: cable.meta.length.map(|length| length.metres()),
        }
    }
}

impl From<&Row<'_>> for WireRecord {
    fn from(row: &Row<'_>) -> Self {
        let owner = row.owner.path.to_string();
        let cable = row.cable.map(CableRecord::from);
        match &row.kind {
            RowKind::Conductor(conductor) => {
                let wire = conductor.wire;
                let rail = match row.length {
                    RowLength::Rail { total, .. } => Some(RailRecord {
                        segment: conductor.segment.index + 1,
                        length_m: total.metres(),
                    }),
                    RowLength::Own(_) | RowLength::Unset => None,
                };
                let [from, to] = &conductor.ends;
                Self::Conductor(Box::new(ConductorRecord {
                    owner,
                    cable,
                    twisted_pair: wire.twisted_group.map(pair_number),
                    label: wire.label.clone(),
                    color: wire.color.to_string(),
                    color_code: wire.color.code(),
                    gauge_mm2: wire.gauge,
                    wire_type: wire.r#type.clone(),
                    part: wire.part.clone(),
                    from: EndRecord::from(from),
                    to: EndRecord::from(to),
                    length_m: row.length.copper().map(|length| length.metres()),
                    rail,
                }))
            }
            RowKind::Shield(shield) => {
                let (screens, twisted_pair) = Screens::of(shield.screened);
                Self::Shield(ShieldRecord {
                    owner,
                    cable,
                    screens,
                    twisted_pair,
                    label: shield.shield.label.clone(),
                    drain: shield.drain.ends().iter().map(EndRecord::from).collect(),
                })
            }
        }
    }
}

impl Sheet for WireSheet {
    const TABLE: Table = Table::Wires;
    const HEADER: &'static [&'static str] = &[
        "kind",
        "owner",
        "cable",
        "cable_label",
        "cable_type",
        "cable_length_m",
        "twisted_pair",
        "screens",
        "label",
        "color",
        "color_code",
        "gauge_mm2",
        "type",
        "part",
        "from",
        "from_kind",
        "from_instance",
        "from_connector",
        "from_pins",
        "from_port",
        "from_port_label",
        "from_splice",
        "to",
        "to_kind",
        "to_instance",
        "to_connector",
        "to_pins",
        "to_port",
        "to_port_label",
        "to_splice",
        "length_m",
        "rail_length_m",
        "rail_segment",
    ];

    fn csv_rows(&self) -> Vec<Vec<String>> {
        self.rows.iter().map(WireRecord::csv_row).collect()
    }
}

impl WireRecord {
    /// The record as one CSV row: a screen's drain ends in the `from` and
    /// `to` columns, in source order.
    fn csv_row(&self) -> Vec<String> {
        let mut row = Vec::with_capacity(WireSheet::HEADER.len());
        match self {
            Self::Conductor(c) => {
                row.push("conductor".to_string());
                row.push(c.owner.clone());
                row.extend(CableRecord::fields(c.cable.as_ref()));
                row.push(field(c.twisted_pair));
                row.push(String::new());
                row.push(field(c.label.as_ref()));
                row.push(c.color.clone());
                row.push(c.color_code.clone());
                row.push(c.gauge_mm2.to_string());
                row.push(field(c.wire_type.as_ref()));
                row.push(field(c.part.as_ref()));
                row.extend(EndRecord::fields(Some(&c.from)));
                row.extend(EndRecord::fields(Some(&c.to)));
                row.push(field(c.length_m));
                row.push(field(c.rail.as_ref().map(|rail| rail.length_m)));
                row.push(field(c.rail.as_ref().map(|rail| rail.segment)));
            }
            Self::Shield(s) => {
                row.push("shield".to_string());
                row.push(s.owner.clone());
                row.extend(CableRecord::fields(s.cable.as_ref()));
                row.push(field(s.twisted_pair));
                row.push(s.screens.as_str().to_string());
                row.push(field(s.label.as_ref()));
                // Colour, code, gauge, type and part: a screen has none.
                row.extend(std::iter::repeat_n(String::new(), 5));
                row.extend(EndRecord::fields(s.drain.first()));
                row.extend(EndRecord::fields(s.drain.get(1)));
                // A screen is cut with its cable (`cable_length_m`): it is
                // no copper of its own, so it adds nothing to `length_m`.
                row.extend(std::iter::repeat_n(String::new(), 3));
            }
        }
        row
    }
}

impl CableRecord {
    /// The cable columns: designator, label, type and length.
    fn fields(cable: Option<&Self>) -> [String; 4] {
        let Some(cable) = cable else {
            return Default::default();
        };
        [
            cable.name.clone(),
            field(cable.label.as_ref()),
            field(cable.cable_type.as_ref()),
            field(cable.length_m),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{csv_rows, design, json};
    use super::super::{Export, Format};
    use super::*;

    const LIB: &str = "component Ecu {\n\
           connector x1 \"ECU\" {\n\
             pub port pwr \"12V\" pin 1; pub port gnd \"GND\" pins [2, 3];\n\
             pub port can_h \"CAN H\" pin 4; pub port can_l \"CAN L\" pin 5;\n\
             pub port shld \"SHLD\" pin 6;\n\
           }\n\
           pub port wake \"WAKE\";\n\
         }\n";

    const USES: &str = "use Ecu from \"lib.wb\";\n";

    fn rows(main: &str) -> Vec<indexmap::IndexMap<String, String>> {
        let (header, rows) = csv_rows(&design(LIB, &format!("{USES}{main}")), Table::Wires);
        assert_eq!(header, WireSheet::HEADER);
        rows
    }

    fn column<'r>(rows: &'r [indexmap::IndexMap<String, String>], name: &str) -> Vec<&'r str> {
        rows.iter().map(|row| row[name].as_str()).collect()
    }

    #[test]
    fn a_label_with_a_comma_a_quote_and_a_non_ascii_letter_is_quoted_once() {
        let mut design = design(
            LIB,
            &format!(
                "{USES}component Car {{ a: Ecu; b: Ecu; wire red 1 \"R1, 4.7 kΩ\" [a.pwr, b.pwr]; }}\n"
            ),
        );
        let csv = |design: &crate::dsl::ir::Design| {
            Export::new(design, Table::Wires, Format::Csv)
                .expect("exports")
                .text()
                .to_string()
        };
        // A comma is quoted; a non-ASCII letter is written as it is.
        assert!(
            csv(&design).contains(",\"R1, 4.7 kΩ\","),
            "{}",
            csv(&design)
        );
        // A `.wb` string cannot hold a quote, but a label is free text: a
        // quote in it is doubled inside the quoted field.
        let root = design.root.clone();
        let car = design.instances.get_mut(&root).expect("the root");
        car.wires[0].label = Some("R1, 4.7 kΩ \"pull-up\"".to_string());
        assert!(
            csv(&design).contains(",\"R1, 4.7 kΩ \"\"pull-up\"\"\","),
            "{}",
            csv(&design)
        );
        let (_, rows) = csv_rows(&design, Table::Wires);
        assert_eq!(column(&rows, "label"), ["R1, 4.7 kΩ \"pull-up\""]);
    }

    #[test]
    fn a_conductor_row_splits_each_end_into_what_a_spreadsheet_filters_by() {
        let rows = rows(
            "component Car { a: Ecu; b: Ecu;\n\
             cable can \"CAN\" { type: \"STP\"; length: 2m;\n\
               twisted { wire yellow 0.5 \"CAN H\" [a.can_h, b.can_h]; wire green 0.5 [a.can_l, b.can_l]; }\n\
             }\n\
             wire green/yellow 2.5 \"GND\" [a.gnd, b.wake] { type: \"FLRY-B\"; part: \"W-1\"; length: 150mm; } }\n",
        );
        let first = &rows[0];
        let expect = [
            ("kind", "conductor"),
            ("owner", "Car"),
            ("cable", "can"),
            ("cable_label", "CAN"),
            ("cable_type", "STP"),
            ("cable_length_m", "2"),
            ("twisted_pair", "1"),
            ("screens", ""),
            ("label", "CAN H"),
            ("color", "yellow"),
            ("color_code", "YE"),
            ("gauge_mm2", "0.5"),
            ("from", "Car.a.can_h"),
            ("from_kind", "port"),
            ("from_instance", "Car.a"),
            ("from_connector", "x1"),
            ("from_pins", "4"),
            ("from_port", "can_h"),
            ("from_port_label", "CAN H"),
            ("from_splice", ""),
            ("to", "Car.b.can_h"),
            ("length_m", "2"),
            ("rail_length_m", ""),
            ("rail_segment", ""),
        ];
        for (name, value) in expect {
            assert_eq!(first[name], value, "{name}");
        }
        let gnd = &rows[2];
        let expect = [
            ("cable", ""),
            ("cable_length_m", ""),
            ("twisted_pair", ""),
            ("color", "green/yellow"),
            ("color_code", "GN/YE"),
            ("gauge_mm2", "2.5"),
            ("type", "FLRY-B"),
            ("part", "W-1"),
            ("from_pins", "2;3"),
            ("to_kind", "port"),
            ("to_connector", ""),
            ("to_pins", ""),
            ("to_port", "wake"),
            ("length_m", "0.15"),
        ];
        for (name, value) in expect {
            assert_eq!(gnd[name], value, "{name}");
        }
    }

    #[test]
    fn summing_length_counts_a_rail_once_and_every_segment_carries_the_rail() {
        let rows = rows(
            "component Car { a: Ecu; b: Ecu; c: Ecu; d: Ecu;\n\
             wire white 0.5 [a.wake, b.wake, c.wake, d.wake] { length: 3m; }\n\
             wire red 1 [a.pwr, b.pwr] { length: 40cm; }\n\
             wire black 1 [c.pwr, d.pwr]; }\n",
        );
        assert_eq!(column(&rows, "length_m"), ["3", "", "", "0.4", ""]);
        assert_eq!(column(&rows, "rail_length_m"), ["3", "3", "3", "", ""]);
        assert_eq!(column(&rows, "rail_segment"), ["1", "2", "3", "", ""]);
        let total: f64 = column(&rows, "length_m")
            .iter()
            .filter(|f| !f.is_empty())
            .map(|f| f.parse::<f64>().expect("a number"))
            .sum();
        assert!((total - 3.4).abs() < 1e-9, "{total}");
    }

    #[test]
    fn a_screen_row_names_what_it_screens_and_where_its_drain_lands() {
        let rows = rows(
            "component Car { a: Ecu; b: Ecu;\n\
             cable can \"CAN\" { length: 2m;\n\
               twisted { wire yellow 0.5 [a.can_h, b.can_h]; wire green 0.5 [a.can_l, b.can_l]; shield; }\n\
               shield \"SHLD\" [a.shld, b.shld];\n\
             } }\n",
        );
        assert_eq!(
            column(&rows, "kind"),
            ["conductor", "conductor", "shield", "shield"]
        );
        let overall = &rows[2];
        let expect = [
            ("cable", "can"),
            ("cable_length_m", "2"),
            ("screens", "cable"),
            ("twisted_pair", ""),
            ("label", "SHLD"),
            ("color", ""),
            ("gauge_mm2", ""),
            ("from", "Car.a.shld"),
            ("from_pins", "6"),
            ("to", "Car.b.shld"),
            ("length_m", ""),
        ];
        for (name, value) in expect {
            assert_eq!(overall[name], value, "{name}");
        }
        let pair = &rows[3];
        assert_eq!(
            (pair["screens"].as_str(), pair["twisted_pair"].as_str()),
            ("pair", "1")
        );
        assert_eq!((pair["from"].as_str(), pair["to"].as_str()), ("", ""));
    }

    #[test]
    fn json_nests_the_ends_the_cable_and_the_rail() {
        let design = design(
            LIB,
            &format!(
                "{USES}component Car {{ a: Ecu; b: Ecu; c: Ecu; splice sp \"J\";\n\
             wire white 0.5 [a.wake, b.wake, c.wake] {{ length: 3m; }}\n\
             wire black 1 [a.gnd, sp];\n\
             cable k {{ wire red 1 [a.pwr, b.pwr]; shield [a.shld]; }} }}\n"
            ),
        );
        let document = json(&design, Table::Wires);
        let rows = document["rows"].as_array().expect("rows");
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0]["kind"], "conductor");
        assert_eq!(rows[0]["length_m"], 3.0);
        assert_eq!(rows[0]["rail"]["segment"], 1);
        assert_eq!(rows[0]["rail"]["length_m"], 3.0);
        assert!(rows[1]["length_m"].is_null());
        assert_eq!(rows[1]["rail"]["segment"], 2);
        let gnd = &rows[2];
        assert!(gnd["rail"].is_null());
        assert_eq!(gnd["from"]["pins"], serde_json::json!([2, 3]));
        assert_eq!(gnd["from"]["port_label"], "GND");
        assert_eq!(gnd["to"]["kind"], "splice");
        assert_eq!(gnd["to"]["instance"], "Car");
        assert_eq!(gnd["to"]["splice"], "sp");
        assert_eq!(gnd["to"]["path"], "Car.sp");
        assert_eq!(rows[3]["cable"]["name"], "k");
        assert!(rows[3]["cable"]["length_m"].is_null());
        let shield = &rows[4];
        assert_eq!(shield["kind"], "shield");
        assert_eq!(shield["screens"], "cable");
        assert_eq!(shield["drain"][0]["path"], "Car.a.shld");
        assert_eq!(shield["drain"].as_array().map(Vec::len), Some(1));
    }
}
