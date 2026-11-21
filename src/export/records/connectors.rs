//! The pin tables' records: one per connector (per inline half), with its
//! cavities, its ports without a pin, and what is crimped into each.

use serde::Serialize;

use super::ends::EndRecord;
use super::wires::Screens;
use super::{Sheet, Table, field, pair_number, pin_list};
use crate::dsl::ir::{Pin, Port};
use crate::export::{CavityEntry, FarEnd, HalfFit, PinTable, PinTables};

/// The pin tables as written: `connectors` in table order.
#[derive(Debug, Serialize)]
pub(super) struct ConnectorSheet {
    connectors: Vec<ConnectorRecord>,
}

impl From<&PinTables<'_>> for ConnectorSheet {
    fn from(tables: &PinTables<'_>) -> Self {
        Self {
            connectors: tables.tables().iter().map(ConnectorRecord::from).collect(),
        }
    }
}

/// One connector's pin table.
#[derive(Debug, Serialize)]
struct ConnectorRecord {
    /// The full path of the instance the connector is on.
    instance: String,
    /// The connector's designator; an inline connector's name.
    connector: String,
    /// The housing half of an inline connector this table is of.
    half: Option<&'static str>,
    housing: HousingRecord,
    /// One per cavity, by pin.
    cavities: Vec<CavityRecord>,
    /// The connector's ports that have no pin assigned, in source order.
    unpinned: Vec<UnpinnedRecord>,
}

/// What the table's housing is bought as.
#[derive(Debug, Serialize)]
struct HousingRecord {
    #[serde(rename = "type")]
    connector_type: Option<String>,
    description: Option<String>,
    part: Option<String>,
}

#[derive(Debug, Serialize)]
struct CavityRecord {
    pin: u32,
    /// The other cavities the cavity's port occupies, for a ganged port.
    ganged_with: Vec<u32>,
    /// The port bound to the pin; `None` for a cavity no port uses.
    port: Option<PortRecord>,
    entries: Vec<EntryRecord>,
}

#[derive(Debug, Serialize)]
struct UnpinnedRecord {
    port: PortRecord,
    entries: Vec<EntryRecord>,
}

#[derive(Debug, Serialize)]
struct PortRecord {
    name: String,
    label: String,
}

impl From<&Port> for PortRecord {
    fn from(port: &Port) -> Self {
        Self {
            name: port.name.to_string(),
            label: port.label.clone(),
        }
    }
}

/// One thing crimped into a cavity.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EntryRecord {
    Conductor(ConductorEntry),
    Drain(DrainEntry),
}

/// A conductor in a cavity, with where it goes.
#[derive(Debug, Serialize)]
struct ConductorEntry {
    /// Whether the conductor is known to be in this half: set only on a
    /// table of an inline connector's half.
    half_fit: Option<Fit>,
    owner: String,
    cable: Option<String>,
    twisted_pair: Option<u32>,
    label: Option<String>,
    color: String,
    color_code: String,
    gauge_mm2: f64,
    /// The end across the conductor.
    far: EndRecord,
    /// For a far end on a splice, every other end its net joins.
    beyond: Vec<EndRecord>,
}

/// A screen's drain in a cavity.
#[derive(Debug, Serialize)]
struct DrainEntry {
    half_fit: Option<Fit>,
    owner: String,
    cable: String,
    screens: Screens,
    twisted_pair: Option<u32>,
    /// The screen's label.
    label: Option<String>,
    /// The drain's other end, for a screen grounded at both ends.
    far: Option<EndRecord>,
}

/// [`HalfFit`], as written.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Fit {
    Assigned,
    Undetermined,
}

impl Fit {
    /// `fit` on a table of `half`, written only for an inline's half.
    fn on(half: Option<&str>, fit: HalfFit) -> Option<Self> {
        half.map(|_| match fit {
            HalfFit::Assigned => Self::Assigned,
            HalfFit::Undetermined => Self::Undetermined,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Assigned => "assigned",
            Self::Undetermined => "undetermined",
        }
    }
}

impl From<&PinTable<'_>> for ConnectorRecord {
    fn from(table: &PinTable<'_>) -> Self {
        let half = table.half.map(|half| half.as_str());
        let housing = table.housing();
        let entries = |entries: &[CavityEntry<'_>]| {
            entries
                .iter()
                .map(|entry| EntryRecord::of(entry, half))
                .collect()
        };
        Self {
            instance: table.instance.path.to_string(),
            connector: table.connector.name.to_string(),
            half,
            housing: HousingRecord {
                connector_type: housing.type_name.map(ToString::to_string),
                description: housing.description.map(str::to_string),
                part: housing.part().map(str::to_string),
            },
            cavities: table
                .cavities
                .iter()
                .map(|cavity| CavityRecord {
                    pin: cavity.pin.0,
                    ganged_with: cavity.ganged_with().map(|pin| pin.0).collect(),
                    port: cavity.port.map(PortRecord::from),
                    entries: entries(&cavity.entries),
                })
                .collect(),
            unpinned: table
                .unpinned
                .iter()
                .map(|unpinned| UnpinnedRecord {
                    port: PortRecord::from(unpinned.port),
                    entries: entries(&unpinned.entries),
                })
                .collect(),
        }
    }
}

impl EntryRecord {
    /// `entry`, in a table of `half`.
    fn of(entry: &CavityEntry<'_>, half: Option<&str>) -> Self {
        match entry {
            CavityEntry::Conductor(c) => {
                let wire = c.conductor.wire;
                let beyond = match &c.far {
                    FarEnd::Splice(splice) => splice
                        .beyond
                        .iter()
                        .map(|spliced| EndRecord::from(&spliced.end))
                        .collect(),
                    FarEnd::Port(_) | FarEnd::Boundary(_) | FarEnd::Unresolved(_) => Vec::new(),
                };
                Self::Conductor(ConductorEntry {
                    half_fit: Fit::on(half, c.half),
                    owner: c.owner.path.to_string(),
                    cable: c.cable.map(|cable| cable.name.to_string()),
                    twisted_pair: wire.twisted_group.map(pair_number),
                    label: wire.label.clone(),
                    color: wire.color.to_string(),
                    color_code: wire.color.code(),
                    gauge_mm2: wire.gauge,
                    far: EndRecord::from(&c.far),
                    beyond,
                })
            }
            CavityEntry::Drain(d) => {
                let (screens, twisted_pair) = Screens::of(d.shield.screened);
                Self::Drain(DrainEntry {
                    half_fit: Fit::on(half, d.half),
                    owner: d.owner.path.to_string(),
                    cable: d.cable.name.to_string(),
                    screens,
                    twisted_pair,
                    label: d.shield.shield.label.clone(),
                    far: d.far.as_ref().map(EndRecord::from),
                })
            }
        }
    }

    /// The entry's columns, from `entry` on: one CSV row's tail.
    fn fields(&self) -> Vec<String> {
        let mut row = Vec::with_capacity(ENTRY_COLUMNS);
        match self {
            Self::Conductor(c) => {
                row.push("conductor".to_string());
                row.push(field(c.half_fit.map(Fit::as_str)));
                row.push(c.owner.clone());
                row.push(field(c.cable.as_ref()));
                row.push(field(c.twisted_pair));
                row.push(String::new());
                row.push(field(c.label.as_ref()));
                row.push(c.color.clone());
                row.push(c.color_code.clone());
                row.push(c.gauge_mm2.to_string());
                row.extend(EndRecord::fields(Some(&c.far)));
                row.push(
                    c.beyond
                        .iter()
                        .map(EndRecord::path)
                        .collect::<Vec<_>>()
                        .join(";"),
                );
            }
            Self::Drain(d) => {
                row.push("drain".to_string());
                row.push(field(d.half_fit.map(Fit::as_str)));
                row.push(d.owner.clone());
                row.push(d.cable.clone());
                row.push(field(d.twisted_pair));
                row.push(d.screens.as_str().to_string());
                row.push(field(d.label.as_ref()));
                // A drain has no colour, code or gauge of its own.
                row.extend(std::iter::repeat_n(String::new(), 3));
                row.extend(EndRecord::fields(d.far.as_ref()));
                row.push(String::new());
            }
        }
        row
    }

    /// The tail of a row for a cavity or port with nothing crimped in.
    fn empty() -> Vec<String> {
        let mut row = vec!["empty".to_string()];
        row.resize(ENTRY_COLUMNS, String::new());
        row
    }
}

/// The columns before an entry's: which connector, which cavity.
const LEAD_COLUMNS: usize = 10;
/// The columns of an entry, from `entry` to `beyond`.
const ENTRY_COLUMNS: usize = 19;

impl Sheet for ConnectorSheet {
    const TABLE: Table = Table::Connectors;
    const HEADER: &'static [&'static str] = &[
        "instance",
        "connector",
        "half",
        "housing_type",
        "housing_description",
        "housing_part",
        "pin",
        "ganged_with",
        "port",
        "port_label",
        "entry",
        "half_fit",
        "owner",
        "cable",
        "twisted_pair",
        "screens",
        "label",
        "color",
        "color_code",
        "gauge_mm2",
        "far",
        "far_kind",
        "far_instance",
        "far_connector",
        "far_pins",
        "far_port",
        "far_port_label",
        "far_splice",
        "beyond",
    ];

    /// One row per entry of each cavity, then of each port without a pin;
    /// one row, of kind `empty`, for a cavity or port with none.
    fn csv_rows(&self) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        for table in &self.connectors {
            let lead = |pin: Option<u32>, ganged_with: &[u32], port: Option<&PortRecord>| {
                let mut row = Vec::with_capacity(LEAD_COLUMNS + ENTRY_COLUMNS);
                row.push(table.instance.clone());
                row.push(table.connector.clone());
                row.push(field(table.half));
                row.push(field(table.housing.connector_type.as_ref()));
                row.push(field(table.housing.description.as_ref()));
                row.push(field(table.housing.part.as_ref()));
                row.push(field(pin));
                row.push(pin_list(ganged_with.iter().copied().map(Pin)));
                row.push(field(port.map(|port| &port.name)));
                row.push(field(port.map(|port| &port.label)));
                row
            };
            let mut push = |lead: Vec<String>, entries: &[EntryRecord]| {
                if entries.is_empty() {
                    rows.push([lead.as_slice(), &EntryRecord::empty()].concat());
                }
                for entry in entries {
                    rows.push([lead.as_slice(), &entry.fields()].concat());
                }
            };
            for cavity in &table.cavities {
                let lead = lead(Some(cavity.pin), &cavity.ganged_with, cavity.port.as_ref());
                push(lead, &cavity.entries);
            }
            for unpinned in &table.unpinned {
                push(lead(None, &[], Some(&unpinned.port)), &unpinned.entries);
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::super::tests::{csv_rows, design, json};
    use super::*;

    const LIB: &str = "connector_type Quad \"Quad 4p\" { part: \"Q-4\"; layout grid { rows: 1; cols: 4; } }\n\
         component Ecu {\n\
           connector x1: Quad {\n\
             pub port pwr \"12V\" pin 1; pub port gnd \"GND\" pins [2, 3];\n\
             pub port shld \"SHLD\" pin 4; pub port aux \"AUX\";\n\
           }\n\
           pub port sense \"SENSE\";\n\
         }\n";

    const USES: &str = "use Ecu from \"lib.wb\";\n";

    const INLINE: &str = "connector_type M2 \"M 2p\" { part: \"M-2\"; }\n\
         connector_type F2 \"F 2p\" { }\n";

    type Rows = Vec<IndexMap<String, String>>;

    fn rows(main: &str) -> Rows {
        let (header, rows) = csv_rows(&design(LIB, &format!("{USES}{main}")), Table::Connectors);
        assert_eq!(header, ConnectorSheet::HEADER);
        assert_eq!(
            LEAD_COLUMNS + ENTRY_COLUMNS,
            ConnectorSheet::HEADER.len(),
            "every column is a lead or an entry column"
        );
        rows
    }

    /// The rows of `instance`'s `connector` (and `half`), each as the
    /// fields of `names`.
    fn of(rows: &Rows, instance: &str, half: &str, names: &[&str]) -> Vec<Vec<String>> {
        rows.iter()
            .filter(|row| row["instance"] == instance && row["half"] == half)
            .map(|row| names.iter().map(|name| row[*name].clone()).collect())
            .collect()
    }

    fn strings(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|field| field.to_string()).collect())
            .collect()
    }

    #[test]
    fn every_cavity_is_listed_and_a_double_crimp_is_two_rows() {
        let rows = rows(
            "component Car { ecu: Ecu; a: Ecu; b: Ecu;\n\
             wire red 1 \"12V\" [ecu.pwr, a.pwr];\n\
             wire red 0.5 [ecu.pwr, b.pwr]; }\n",
        );
        let names = [
            "connector",
            "housing_type",
            "housing_part",
            "pin",
            "ganged_with",
            "port_label",
            "entry",
            "color_code",
            "gauge_mm2",
            "far",
            "far_pins",
        ];
        assert_eq!(
            of(&rows, "Car.ecu", "", &names),
            strings(&[
                &[
                    "x1",
                    "Quad",
                    "Q-4",
                    "1",
                    "",
                    "12V",
                    "conductor",
                    "RD",
                    "1",
                    "Car.a.pwr",
                    "1"
                ],
                &[
                    "x1",
                    "Quad",
                    "Q-4",
                    "1",
                    "",
                    "12V",
                    "conductor",
                    "RD",
                    "0.5",
                    "Car.b.pwr",
                    "1"
                ],
                &[
                    "x1", "Quad", "Q-4", "2", "3", "GND", "empty", "", "", "", ""
                ],
                &[
                    "x1", "Quad", "Q-4", "3", "2", "GND", "empty", "", "", "", ""
                ],
                &[
                    "x1", "Quad", "Q-4", "4", "", "SHLD", "empty", "", "", "", ""
                ],
                // A port without a pin follows the cavities, its pin empty.
                &["x1", "Quad", "Q-4", "", "", "AUX", "empty", "", "", "", ""],
            ])
        );
    }

    #[test]
    fn a_conductor_into_a_port_without_a_pin_is_a_row_with_an_empty_pin() {
        let rows = rows("component Car { a: Ecu; b: Ecu; wire violet 0.5 [a.aux, b.aux]; }\n");
        let names = [
            "pin",
            "port",
            "entry",
            "color_code",
            "far",
            "far_pins",
            "far_port",
        ];
        let aux: Vec<_> = of(&rows, "Car.a", "", &names)
            .into_iter()
            .filter(|row| row[1] == "aux")
            .collect();
        assert_eq!(
            aux,
            strings(&[&["", "aux", "conductor", "VT", "Car.b.aux", "", "aux"]])
        );
    }

    #[test]
    fn a_far_end_on_a_splice_lists_what_lies_beyond_it() {
        let design = design(
            LIB,
            &format!(
                "{USES}component Car {{ ecu: Ecu; a: Ecu; bat: Ecu; splice sp \"GND\";\n\
             wire black 1 [ecu.gnd, sp];\n\
             wire black 0.5 [a.gnd, sp];\n\
             wire black 2.5 [sp, bat.sense]; }}\n"
            ),
        );
        let (_, rows) = csv_rows(&design, Table::Connectors);
        let names = [
            "pin",
            "far",
            "far_kind",
            "far_instance",
            "far_splice",
            "beyond",
        ];
        assert_eq!(
            of(&rows, "Car.ecu", "", &names)[1..3],
            strings(&[
                &[
                    "2",
                    "Car.sp",
                    "splice",
                    "Car",
                    "sp",
                    "Car.a.gnd;Car.bat.sense"
                ],
                &[
                    "3",
                    "Car.sp",
                    "splice",
                    "Car",
                    "sp",
                    "Car.a.gnd;Car.bat.sense"
                ],
            ])
        );
        let document = json(&design, Table::Connectors);
        let ecu = &document["connectors"][0];
        assert_eq!(ecu["instance"], "Car.ecu");
        let entry = &ecu["cavities"][1]["entries"][0];
        assert_eq!(entry["kind"], "conductor");
        assert_eq!(entry["far"]["kind"], "splice");
        let beyond: Vec<&str> = entry["beyond"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|end| end["path"].as_str().expect("a path"))
            .collect();
        assert_eq!(beyond, ["Car.a.gnd", "Car.bat.sense"]);
        assert_eq!(ecu["cavities"][1]["ganged_with"], serde_json::json!([3]));
    }

    #[test]
    fn a_drain_is_an_entry_with_its_screen_and_its_other_end() {
        let rows = rows(
            "component Car { a: Ecu; b: Ecu;\n\
             cable can \"CAN\" { wire yellow 0.5 [a.pwr, b.pwr]; shield \"SHLD\" [a.shld, b.shld]; } }\n",
        );
        let names = [
            "pin",
            "entry",
            "cable",
            "screens",
            "label",
            "color",
            "gauge_mm2",
            "far",
            "far_pins",
            "beyond",
        ];
        let shld: Vec<_> = of(&rows, "Car.a", "", &names)
            .into_iter()
            .filter(|row| row[0] == "4")
            .collect();
        assert_eq!(
            shld,
            strings(&[&[
                "4",
                "drain",
                "can",
                "cable",
                "SHLD",
                "",
                "",
                "Car.b.shld",
                "4",
                ""
            ]])
        );
    }

    #[test]
    fn an_inline_half_names_its_housing_and_flags_an_undetermined_conductor() {
        let design = design(
            LIB,
            &format!(
                "{USES}{INLINE}component Car {{ ecu: Ecu; pedal: Ecu;\n\
               inline ic \"Branch\" {{ male: M2; female: F2; port sig \"SIG\" pin 1; port ret \"RET\" pin 2; }}\n\
               wire green 0.5 \"SIG\" [ecu.pwr, ic.sig];\n\
               wire green 0.5 \"SIG\" [ic.sig, pedal.pwr];\n\
             }}\n\
             view harness \"Main\" for Car {{ include ecu.x1 at (0, 0); include ic.female at (10, 0); }}\n"
            ),
        );
        let (_, rows) = csv_rows(&design, Table::Connectors);
        let names = [
            "housing_type",
            "housing_description",
            "housing_part",
            "pin",
            "entry",
            "half_fit",
            "far",
        ];
        // The main view draws the ECU's conductor into the female half; no
        // view draws the pedal's, so it is listed under both, flagged.
        assert_eq!(
            of(&rows, "Car.ic", "male", &names),
            strings(&[
                &[
                    "M2",
                    "M 2p",
                    "M-2",
                    "1",
                    "conductor",
                    "undetermined",
                    "Car.pedal.pwr"
                ],
                &["M2", "M 2p", "M-2", "2", "empty", "", ""],
            ])
        );
        assert_eq!(
            of(&rows, "Car.ic", "female", &names),
            strings(&[
                &[
                    "F2",
                    "F 2p",
                    "",
                    "1",
                    "conductor",
                    "assigned",
                    "Car.ecu.pwr"
                ],
                &[
                    "F2",
                    "F 2p",
                    "",
                    "1",
                    "conductor",
                    "undetermined",
                    "Car.pedal.pwr"
                ],
                &["F2", "F 2p", "", "2", "empty", "", ""],
            ])
        );
        // An ordinary connector's rows have no half and no fit.
        assert!(
            of(&rows, "Car.ecu", "", &["half_fit"])
                .iter()
                .all(|row| row[0].is_empty())
        );
        let document = json(&design, Table::Connectors);
        let male = document["connectors"]
            .as_array()
            .expect("connectors")
            .iter()
            .find(|t| t["instance"] == "Car.ic" && t["half"] == "male")
            .expect("the male half");
        assert_eq!(male["housing"]["type"], "M2");
        assert_eq!(male["housing"]["part"], "M-2");
        assert_eq!(
            male["cavities"][0]["entries"][0]["half_fit"],
            "undetermined"
        );
        assert_eq!(male["unpinned"], serde_json::json!([]));
    }

    #[test]
    fn json_lists_a_ports_without_a_pin_apart_from_the_cavities() {
        let design = design(
            LIB,
            &format!("{USES}component Car {{ a: Ecu; b: Ecu; wire violet 0.5 [a.aux, b.aux]; }}\n"),
        );
        let document = json(&design, Table::Connectors);
        let a = &document["connectors"][0];
        let pins: Vec<u64> = a["cavities"]
            .as_array()
            .expect("cavities")
            .iter()
            .map(|c| c["pin"].as_u64().expect("a pin"))
            .collect();
        assert_eq!(pins, [1, 2, 3, 4]);
        assert_eq!(a["unpinned"][0]["port"]["label"], "AUX");
        assert_eq!(a["unpinned"][0]["entries"][0]["far"]["path"], "Car.b.aux");
        assert!(a["cavities"][0]["port"]["name"] == "pwr");
        assert!(
            a["cavities"][0]["entries"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
    }
}
