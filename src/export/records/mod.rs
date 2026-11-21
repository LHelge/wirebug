//! The build tables' wire format: what `wirebug export` prints and
//! `render --embed` writes beside its manifest.
//!
//! Each table is turned into dedicated record types, built from the typed
//! models ([`WireList`], [`PinTables`], [`Bom`]) and owned by this module, so the
//! models and the IR stay free of serialisation. A table's CSV is the
//! flattening of the same records its JSON nests, one CSV row per record
//! (per cavity entry, for the pin tables), so the two formats carry the
//! same data. Both are a public interface: a column or field is added, not
//! renamed, moved or changed in meaning, without raising
//! [`FORMAT_VERSION`].

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{Bom, PinTables, WireList};
use crate::dsl::ir::{Design, Pin};
use crate::dsl::manifest::Manifest;
use crate::error::{Error, Result};

mod bom;
mod connectors;
mod ends;
mod wires;

use bom::BomSheet;
use connectors::ConnectorSheet;
use wires::WireSheet;

/// The version of the export wire format, carried by every JSON document
/// as `format_version`. Raised when a column or field changes name, place
/// or meaning, so tooling can tell.
pub const FORMAT_VERSION: u32 = 1;

/// A build table `wirebug export` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    /// The wire list (cut list): one row per conductor and per cable screen.
    Wires,
    /// The connector pin tables (crimp sheet): one row per cavity entry.
    Connectors,
    /// The bill of materials: one row per thing to buy, with its quantity.
    Bom,
}

impl Table {
    /// Every table, in the order `render --embed` writes them.
    pub const ALL: [Self; 3] = [Self::Wires, Self::Connectors, Self::Bom];

    /// The table's name: its `export` subcommand, its JSON `table` field
    /// and the stem of its file name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wires => "wires",
            Self::Connectors => "connectors",
            Self::Bom => "bom",
        }
    }

    /// The file name the table is written to in `format`: `wires.json`.
    pub fn filename(self, format: Format) -> String {
        format!("{}.{}", self.name(), format.extension())
    }

    /// What one CSV row of the table is, as a summary counts them.
    fn row_noun(self) -> &'static str {
        match self {
            Self::Wires => "wire-list",
            Self::Connectors => "pin-table",
            Self::Bom => "BOM",
        }
    }
}

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The text format a table is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// RFC 4180 CSV: a header row, then one row per record; fields quoted
    /// only where they need it, `\n` line endings, UTF-8 without a BOM.
    #[default]
    Csv,
    /// One pretty-printed JSON document: the project's identity and the
    /// format version around the table's records.
    Json,
}

impl Format {
    /// The format's file extension, which is also its name.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// One table of a design, written out in one format: the text to print
/// or save.
#[derive(Debug)]
#[must_use]
pub struct Export {
    table: Table,
    format: Format,
    /// How many CSV rows the table has (its records, below the header).
    rows: usize,
    text: String,
}

impl Export {
    /// `table` of `design`, in `format`.
    pub fn new(design: &Design, table: Table, format: Format) -> Result<Self> {
        let project = design.manifest.as_ref();
        match table {
            Table::Wires => Self::of(&WireSheet::from(&WireList::from(design)), project, format),
            Table::Connectors => Self::of(
                &ConnectorSheet::from(&PinTables::from(design)),
                project,
                format,
            ),
            Table::Bom => Self::of(&BomSheet::from(&Bom::from(design)), project, format),
        }
    }

    fn of<S: Sheet>(sheet: &S, project: Option<&Manifest>, format: Format) -> Result<Self> {
        let rows = sheet.csv_rows();
        let text = match format {
            Format::Csv => csv_text(S::HEADER, &rows).map_err(|source| Error::ExportCsv {
                table: S::TABLE,
                source,
            })?,
            Format::Json => {
                let document = Document {
                    table: S::TABLE.name(),
                    format_version: FORMAT_VERSION,
                    project,
                    generator: crate::GENERATOR,
                    sheet,
                };
                let mut json = serde_json::to_string_pretty(&document).map_err(|source| {
                    Error::ExportJson {
                        table: S::TABLE,
                        source,
                    }
                })?;
                json.push('\n');
                json
            }
        };
        Ok(Self {
            table: S::TABLE,
            format,
            rows: rows.len(),
            text,
        })
    }

    /// The table as text, ready to print.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Write the table to `path`, creating its directory as `render`
    /// creates `--out`, and return what was written for the summary line.
    pub fn write(&self, path: &Path) -> Result<Exported> {
        let failed = |source| Error::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(failed)?;
        }
        fs::write(path, &self.text).map_err(failed)?;
        Ok(Exported {
            table: self.table,
            format: self.format,
            rows: self.rows,
            path: path.to_path_buf(),
        })
    }
}

/// A table [`Export::write`] wrote. Its `Display` is the one-line summary
/// `export --out` prints.
#[derive(Debug)]
pub struct Exported {
    table: Table,
    format: Format,
    rows: usize,
    path: PathBuf,
}

impl fmt::Display for Exported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "exported {} {} row(s) as {} to {}",
            self.rows,
            self.table.row_noun(),
            self.format,
            self.path.display()
        )
    }
}

/// A table's records: serialisable as the body of its JSON document, and
/// flattened into CSV rows of [`Sheet::HEADER`]'s columns.
trait Sheet: Serialize {
    const TABLE: Table;
    const HEADER: &'static [&'static str];

    /// The table's CSV rows, each one field per column of the header.
    fn csv_rows(&self) -> Vec<Vec<String>>;
}

/// The JSON document a table is written as: its name, the format version
/// and who produced it (as the embed manifest names them), then the
/// sheet's own fields.
#[derive(Serialize)]
struct Document<'a, S> {
    table: &'static str,
    format_version: u32,
    project: Option<&'a Manifest>,
    generator: &'static str,
    #[serde(flatten)]
    sheet: &'a S,
}

/// `rows` under `header`, as CSV text. Every row must have a field per
/// column: the writer refuses a row of another length.
fn csv_text(header: &[&str], rows: &[Vec<String>]) -> csv::Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    writer.write_record(header)?;
    for row in rows {
        writer.write_record(row)?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|error| csv::Error::from(error.into_error()))?;
    // Every field written is a Rust string, so the bytes are UTF-8.
    String::from_utf8(bytes).map_err(|error| {
        csv::Error::from(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    })
}

/// Pins as one CSV field: `2;3`, empty for none.
fn pin_list(pins: impl IntoIterator<Item = Pin>) -> String {
    pins.into_iter()
        .map(|pin| pin.to_string())
        .collect::<Vec<_>>()
        .join(";")
}

/// An optional value as a CSV field, empty when absent.
fn field<T: ToString>(value: Option<T>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// A twisted pair's number as the export shows it: the `twisted { }` group
/// of its cable, counted from 1 in source order.
fn pair_number(group: u32) -> u32 {
    group + 1
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A checked design of `main` (its one top-level component is the
    /// root) beside `lib.wb`, holding `lib`. Panics on an error; warnings
    /// are allowed.
    pub(in crate::export) fn design(lib: &str, main: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"t\"\nversion = \"0.0.0\"\nrevision = \"r\"\n",
        )
        .expect("write manifest");
        std::fs::write(dir.path().join("main.wb"), main).expect("write main.wb");
        std::fs::write(dir.path().join("lib.wb"), lib).expect("write lib.wb");
        let report = crate::check_project(Some(dir.path()));
        let errors: Vec<_> = report.problems.iter().filter(|p| p.is_error()).collect();
        assert!(errors.is_empty(), "the source checks: {errors:?}");
        report.design.expect("a design")
    }

    /// The CSV text of `table`, parsed back into its header and one map of
    /// column to field per row.
    pub(in crate::export) fn csv_rows(
        design: &Design,
        table: Table,
    ) -> (Vec<String>, Vec<indexmap::IndexMap<String, String>>) {
        let export = Export::new(design, table, Format::Csv).expect("exports");
        let mut reader = csv::Reader::from_reader(export.text().as_bytes());
        let header: Vec<String> = reader
            .headers()
            .expect("a header")
            .iter()
            .map(str::to_string)
            .collect();
        let rows = reader
            .records()
            .map(|record| {
                let record = record.expect("a row");
                assert_eq!(record.len(), header.len(), "a field per column");
                header
                    .iter()
                    .cloned()
                    .zip(record.iter().map(str::to_string))
                    .collect()
            })
            .collect();
        (header, rows)
    }

    pub(in crate::export) fn json(design: &Design, table: Table) -> serde_json::Value {
        let export = Export::new(design, table, Format::Json).expect("exports");
        serde_json::from_str(export.text()).expect("valid JSON")
    }

    const LIB: &str = "component Ecu { connector x1 \"ECU\" { pub port pwr \"12V\" pin 1; } }\n";
    const SIMPLE: &str = "use Ecu from \"lib.wb\";\n\
         component Car { a: Ecu; b: Ecu; wire red 1 [a.pwr, b.pwr]; }\n";

    #[test]
    fn a_json_document_carries_the_table_the_version_and_the_project() {
        let design = design(LIB, SIMPLE);
        for table in Table::ALL {
            let document = json(&design, table);
            assert_eq!(document["table"], table.name());
            assert_eq!(document["format_version"], FORMAT_VERSION);
            assert_eq!(document["project"]["name"], "t");
            assert_eq!(document["project"]["revision"], "r");
            assert_eq!(document["generator"], crate::GENERATOR);
        }
    }

    #[test]
    fn csv_has_a_header_and_lf_line_endings_and_no_byte_order_mark() {
        let design = design(LIB, SIMPLE);
        let export = Export::new(&design, Table::Wires, Format::Csv).expect("exports");
        let text = export.text();
        assert!(text.starts_with("kind,owner,"), "{text}");
        assert!(text.ends_with('\n'));
        assert!(!text.contains('\r'));
        assert_eq!(text.lines().count(), 2, "a header and one conductor");
    }

    #[test]
    fn a_written_table_is_summarised_with_its_row_count_and_path() {
        let design = design(LIB, SIMPLE);
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("pins.csv");
        let export = Export::new(&design, Table::Connectors, Format::Csv).expect("exports");
        let written = export.write(&path).expect("writes");
        assert_eq!(
            written.to_string(),
            format!("exported 2 pin-table row(s) as csv to {}", path.display())
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("written"),
            export.text()
        );
    }

    #[test]
    fn writing_creates_the_directory_and_a_failure_names_the_file() {
        let design = design(LIB, SIMPLE);
        let tmp = tempfile::tempdir().expect("tempdir");
        let export = Export::new(&design, Table::Wires, Format::Csv).expect("exports");
        let path = tmp.path().join("tables/wires.csv");
        export.write(&path).expect("writes");
        assert!(path.is_file());
        // A directory where the file should go cannot be written over.
        let error = export.write(tmp.path()).expect_err("a directory");
        assert!(matches!(&error, Error::Write { path: p, .. } if p == tmp.path()));
    }
}
