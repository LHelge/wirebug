//! The bill of materials' records: one per line, every kind under the same
//! fields, so the JSON and the CSV are the same flat table.

use serde::Serialize;

use super::{Sheet, Table, field};
use crate::export::{Bom, Line};

/// The bill of materials as written: `lines` in BOM order.
#[derive(Debug, Serialize)]
pub(super) struct BomSheet {
    lines: Vec<LineRecord>,
}

impl From<&Bom<'_>> for BomSheet {
    fn from(bom: &Bom<'_>) -> Self {
        Self {
            lines: bom.lines().iter().map(LineRecord::from).collect(),
        }
    }
}

/// One line. A field a kind has no use for is `None`.
#[derive(Debug, Default, Serialize)]
struct LineRecord {
    /// `housing`, `cable`, `wire` (loose wire), `component` or `splice`.
    kind: &'static str,
    /// Whether the line names what to buy: its part (a cable's type).
    specified: bool,
    part: Option<String>,
    /// A housing's connector type, a cable's or a wire's `type`, a
    /// component's type.
    #[serde(rename = "type")]
    item_type: Option<String>,
    /// For a splice without a part: the label (else name) it is grouped by.
    label: Option<String>,
    description: Option<String>,
    manufacturer: Option<String>,
    rating: Option<String>,
    color: Option<String>,
    color_code: Option<String>,
    gauge_mm2: Option<f64>,
    quantity: usize,
    /// For a cable or a wire: the summed length, in metres, of those that
    /// have one; `None` when none has.
    total_length_m: Option<f64>,
    /// For a cable or a wire: how many of `quantity` have no length, so
    /// are not in `total_length_m`.
    unmeasured: Option<usize>,
    /// For a wire: how many of `quantity` end on the owning component's
    /// own connectorless port, continuing a conductor the enclosing
    /// component wires to it ([`WireLine::boundary`](crate::export::WireLine::boundary)).
    boundary: Option<usize>,
    /// The placements `quantity` counts, as full paths: empty for a wire,
    /// whose conductors are the wire list's rows.
    #[serde(rename = "where")]
    placements: Vec<String>,
}

impl From<&Line<'_>> for LineRecord {
    fn from(line: &Line<'_>) -> Self {
        let length = line.length();
        let base = Self {
            kind: line.kind().as_str(),
            specified: line.is_specified(),
            quantity: line.quantity(),
            total_length_m: length.and_then(|l| l.length).map(|l| l.metres()),
            unmeasured: length.map(|l| l.unmeasured),
            ..Self::default()
        };
        match line {
            Line::Housing(l) => Self {
                part: l.part.map(str::to_string),
                item_type: l.type_name.map(ToString::to_string),
                description: l.description.map(str::to_string),
                placements: l.housings.iter().map(ToString::to_string).collect(),
                ..base
            },
            Line::Cable(l) => Self {
                item_type: l.cable_type.map(str::to_string),
                placements: l.cables.iter().map(ToString::to_string).collect(),
                ..base
            },
            Line::Wire(l) => Self {
                part: l.part.map(str::to_string),
                item_type: l.wire_type.map(str::to_string),
                color: Some(l.color.to_string()),
                color_code: Some(l.color.code()),
                gauge_mm2: Some(l.gauge),
                boundary: Some(l.boundary()),
                ..base
            },
            Line::Component(l) => Self {
                part: l.part.number.clone(),
                item_type: Some(l.type_name.to_string()),
                description: l.part.description.clone(),
                manufacturer: l.part.manufacturer.clone(),
                rating: l.part.rating.clone(),
                placements: l.instances.iter().map(|i| i.path.to_string()).collect(),
                ..base
            },
            Line::Splice(l) => Self {
                part: l.part.map(str::to_string),
                label: l.label.map(str::to_string),
                description: l.description.map(str::to_string),
                placements: l.splices.iter().map(ToString::to_string).collect(),
                ..base
            },
        }
    }
}

impl Sheet for BomSheet {
    const TABLE: Table = Table::Bom;
    const HEADER: &'static [&'static str] = &[
        "kind",
        "specified",
        "part",
        "type",
        "label",
        "description",
        "manufacturer",
        "rating",
        "color",
        "color_code",
        "gauge_mm2",
        "quantity",
        "total_length_m",
        "unmeasured",
        "boundary",
        "where",
    ];

    fn csv_rows(&self) -> Vec<Vec<String>> {
        self.lines
            .iter()
            .map(|line| {
                vec![
                    line.kind.to_string(),
                    line.specified.to_string(),
                    field(line.part.as_ref()),
                    field(line.item_type.as_ref()),
                    field(line.label.as_ref()),
                    field(line.description.as_ref()),
                    field(line.manufacturer.as_ref()),
                    field(line.rating.as_ref()),
                    field(line.color.as_ref()),
                    field(line.color_code.as_ref()),
                    field(line.gauge_mm2),
                    line.quantity.to_string(),
                    field(line.total_length_m),
                    field(line.unmeasured),
                    field(line.boundary),
                    line.placements.join(";"),
                ]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{csv_rows, design, json};
    use super::*;

    const LIB: &str = "connector_type Dt04_2p \"Deutsch DT04-2P\" { part: \"DT04-2P\"; }\n\
         component Lamp { part: \"LMP-1\"; manufacturer: \"Acme\"; description: \"Lamp, red\"; rating: \"5 W\";\n\
           connector x1: Dt04_2p { pub port a \"A\" pin 1; pub port b \"B\" pin 2; } }\n\
         component Plug { connector x1 \"Plug 2p\" { pub port a \"A\" pin 1; pub port b \"B\" pin 2; } }\n";

    const MAIN: &str = "use Lamp from \"lib.wb\";\nuse Plug from \"lib.wb\";\n\
         component Car { l1: Lamp; l2: Lamp; p: Plug;\n\
           splice sp \"Feed\";\n\
           wire red 1 [l1.a, sp] { length: 150mm; }\n\
           wire red 1 [l2.a, sp];\n\
           wire red 1 [sp, p.a] { length: 2m; }\n\
           cable k { type: \"2-core\"; length: 1.5m; wire black 0.5 [l1.b, p.b]; wire black 0.5 [l2.b, p.b]; } }\n";

    fn rows() -> Vec<indexmap::IndexMap<String, String>> {
        let (header, rows) = csv_rows(&design(LIB, MAIN), Table::Bom);
        assert_eq!(header, BomSheet::HEADER);
        rows
    }

    fn fields(row: &indexmap::IndexMap<String, String>, names: &[&str]) -> Vec<String> {
        names.iter().map(|name| row[*name].clone()).collect()
    }

    #[test]
    fn a_csv_row_per_line_with_what_its_total_leaves_out() {
        let rows = rows();
        let names = [
            "kind",
            "specified",
            "part",
            "type",
            "label",
            "description",
            "quantity",
            "total_length_m",
            "unmeasured",
            "where",
        ];
        let got: Vec<Vec<String>> = rows.iter().map(|row| fields(row, &names)).collect();
        let expect: [[&str; 10]; 6] = [
            [
                "housing",
                "true",
                "DT04-2P",
                "Dt04_2p",
                "",
                "Deutsch DT04-2P",
                "2",
                "",
                "",
                "Car.l1.x1;Car.l2.x1",
            ],
            [
                "housing", "false", "", "", "", "Plug 2p", "1", "", "", "Car.p.x1",
            ],
            [
                "cable", "true", "", "2-core", "", "", "1", "1.5", "0", "Car.k",
            ],
            // Three conductors, one without a length: counted, but not in
            // the total, and the row says how many are left out.
            ["wire", "false", "", "", "", "", "3", "2.15", "1", ""],
            [
                "component",
                "true",
                "LMP-1",
                "Lamp",
                "",
                "Lamp, red",
                "2",
                "",
                "",
                "Car.l1;Car.l2",
            ],
            [
                "component",
                "false",
                "",
                "Plug",
                "",
                "",
                "1",
                "",
                "",
                "Car.p",
            ],
        ];
        assert_eq!(got[..6], expect.map(|row| row.map(str::to_string).to_vec()));
        assert_eq!(
            fields(&rows[6], &names),
            ["splice", "false", "", "", "Feed", "", "1", "", "", "Car.sp"]
        );
        assert_eq!(rows.len(), 7);
        assert_eq!(
            fields(&rows[3], &["color", "color_code", "gauge_mm2", "boundary"]),
            ["red", "RD", "1", "0"]
        );
        // Only a wire row counts boundary continuations.
        assert!(
            rows.iter()
                .filter(|row| row["kind"] != "wire")
                .all(|row| row["boundary"].is_empty())
        );
        assert_eq!(
            fields(&rows[4], &["manufacturer", "rating"]),
            ["Acme", "5 W"]
        );
    }

    #[test]
    fn json_lists_the_lines_with_their_placements_and_nulls_where_a_field_does_not_apply() {
        let document = json(&design(LIB, MAIN), Table::Bom);
        let lines = document["lines"].as_array().expect("lines");
        assert_eq!(lines.len(), 7);
        let housing = &lines[0];
        assert_eq!(housing["kind"], "housing");
        assert_eq!(housing["specified"], true);
        assert_eq!(housing["part"], "DT04-2P");
        assert_eq!(housing["quantity"], 2);
        assert_eq!(
            housing["where"],
            serde_json::json!(["Car.l1.x1", "Car.l2.x1"])
        );
        assert!(housing["total_length_m"].is_null());
        assert!(housing["unmeasured"].is_null());
        let wire = &lines[3];
        assert_eq!(wire["kind"], "wire");
        assert_eq!(wire["specified"], false);
        assert!(wire["part"].is_null());
        assert_eq!(wire["gauge_mm2"], 1.0);
        assert_eq!(wire["total_length_m"], 2.15);
        assert_eq!(wire["unmeasured"], 1);
        assert_eq!(wire["boundary"], 0);
        assert!(housing["boundary"].is_null());
        assert_eq!(wire["where"], serde_json::json!([]));
        // Every line has every field, a column each of the CSV.
        let mut columns = BomSheet::HEADER.to_vec();
        columns.sort_unstable();
        for line in lines {
            let mut fields: Vec<&str> = line
                .as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect();
            fields.sort_unstable();
            assert_eq!(fields, columns, "the JSON mirrors the CSV");
        }
    }
}
