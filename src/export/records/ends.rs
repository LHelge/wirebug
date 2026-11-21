//! Where a conductor or drain end lands, as both tables write it.

use serde::Serialize;

use super::{field, pin_list};
use crate::dsl::ir::Pin;
use crate::export::{Contact, End, FarEnd};

/// What an end lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EndKind {
    /// A port of a component placed in the row's owner (an inline
    /// connector's among them).
    Port,
    /// The owner's own port, where the conductor leaves its component.
    Boundary,
    /// One of the owner's splices.
    Splice,
    /// An end the design does not resolve; a checked design has none.
    Unresolved,
}

impl EndKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Port => "port",
            Self::Boundary => "boundary",
            Self::Splice => "splice",
            Self::Unresolved => "unresolved",
        }
    }
}

/// One end: the full path of what it lands on, and that path split into
/// the parts a spreadsheet filters by.
#[derive(Debug, Clone, Serialize)]
pub(super) struct EndRecord {
    /// `Vehicle.ecu.pwr` for a port, `Vehicle.sp_gnd` for a splice, an
    /// unresolved end as written.
    path: String,
    kind: EndKind,
    /// The instance the port is on, or the splice's owner.
    instance: Option<String>,
    /// The connector designator (an inline connector's name); `None` for a
    /// connectorless port.
    connector: Option<String>,
    /// The pins the port occupies: several for a ganged port, none when it
    /// is on no connector or has no pin assigned.
    pins: Vec<u32>,
    port: Option<String>,
    port_label: Option<String>,
    splice: Option<String>,
}

impl EndRecord {
    /// The CSV fields of `end`, in the order of its columns (`from`,
    /// `from_kind`, `from_instance`, `from_connector`, `from_pins`,
    /// `from_port`, `from_port_label`, `from_splice` — or `to_…`, `far_…`);
    /// all empty for no end.
    pub(super) fn fields(end: Option<&Self>) -> [String; 8] {
        let Some(end) = end else {
            return Default::default();
        };
        [
            end.path.clone(),
            end.kind.as_str().to_string(),
            field(end.instance.as_ref()),
            field(end.connector.as_ref()),
            pin_list(end.pins.iter().copied().map(Pin)),
            field(end.port.as_ref()),
            field(end.port_label.as_ref()),
            field(end.splice.as_ref()),
        ]
    }

    /// The end's full path, as a list of ends writes it.
    pub(super) fn path(&self) -> &str {
        &self.path
    }
}

impl From<&End<'_>> for EndRecord {
    fn from(end: &End<'_>) -> Self {
        let path = end.to_string();
        match end {
            End::Port(at) | End::Boundary(at) => {
                let (connector, pins) = match at.contact() {
                    Contact::Connector { designator, pins } => (
                        Some(designator.to_string()),
                        pins.iter().map(|pin| pin.0).collect(),
                    ),
                    Contact::Connectorless => (None, Vec::new()),
                };
                Self {
                    path,
                    kind: match end {
                        End::Boundary(_) => EndKind::Boundary,
                        _ => EndKind::Port,
                    },
                    instance: Some(at.instance.path.to_string()),
                    connector,
                    pins,
                    port: Some(at.port.name.to_string()),
                    port_label: Some(at.port.label.clone()),
                    splice: None,
                }
            }
            End::Splice(at) => Self {
                path,
                kind: EndKind::Splice,
                instance: Some(at.owner.path.to_string()),
                connector: None,
                pins: Vec::new(),
                port: None,
                port_label: None,
                splice: Some(at.splice.name.to_string()),
            },
            End::Unresolved(_) => Self {
                path,
                kind: EndKind::Unresolved,
                instance: None,
                connector: None,
                pins: Vec::new(),
                port: None,
                port_label: None,
                splice: None,
            },
        }
    }
}

impl From<&FarEnd<'_>> for EndRecord {
    fn from(far: &FarEnd<'_>) -> Self {
        let end = match far {
            FarEnd::Port(at) => End::Port(*at),
            FarEnd::Boundary(at) => End::Boundary(*at),
            FarEnd::Splice(splice) => End::Splice(splice.splice),
            FarEnd::Unresolved(end) => End::Unresolved(end),
        };
        Self::from(&end)
    }
}
