//! Build data derived from an elaborated [`Design`](crate::dsl::ir::Design):
//! the tables a harness is cut, crimped and ordered from.
//!
//! Each table is a typed model built from the IR — no layout, no drawing —
//! so it lists everything the design holds, whether or not a view draws it.
//! [`WireList`] is the cut list: one [`Row`] per physical conductor and per
//! cable screen. [`PinTables`], built from it, is the crimp sheet: one
//! [`PinTable`] per connector (per housing half of an inline connector),
//! one [`Cavity`] per pin. Writing a table out (CSV, JSON) is the CLI's
//! business, not this module's.

mod pins;
mod wires;

pub use pins::{
    Cavity, CavityConductor, CavityDrain, CavityEntry, FarEnd, FarSplice, HalfFit, Housing,
    PinTable, PinTables, SplicedEnd, UnpinnedPort,
};
pub use wires::{
    Cable, Conductor, Contact, Copper, Drain, End, PortEnd, Row, RowKind, RowLength, ShieldRun,
    SpliceEnd, WireList,
};
