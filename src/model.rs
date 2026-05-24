//! Domain model: components, connectors, ports, and connections.
//!
//! Newtypes give the type-checker something to do — a [`ConnectorId`]
//! cannot be passed where a [`ComponentId`] is expected, and identifier
//! validation (no `.`) happens at the boundary, not at every use site.

use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Generates an `Id(String)` newtype with [`FromStr`] (rejecting any
/// string containing `.`), [`Display`], [`AsRef<str>`], and serde glue
/// that routes through the validated [`FromStr`].
macro_rules! id_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self> {
                if s.contains('.') {
                    Err(Error::DottedIdentifier(s.to_string()))
                } else {
                    Ok(Self(s.to_string()))
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;

            fn try_from(s: String) -> Result<Self> {
                s.parse()
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_newtype!(ComponentId, "Identifier for a component within a model.");
id_newtype!(
    ConnectorId,
    "Identifier for a connector within a component."
);
id_newtype!(PortId, "Identifier for a port within a connector.");

/// Fully-qualified port reference: `component.connector.port`.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PortRef {
    pub component: ComponentId,
    pub connector: ConnectorId,
    pub port: PortId,
}

impl FromStr for PortRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        match parts.as_slice() {
            [c, k, p] => Ok(Self {
                component: c.parse()?,
                connector: k.parse()?,
                port: p.parse()?,
            }),
            _ => Err(Error::MalformedPortRef { raw: s.to_string() }),
        }
    }
}

impl TryFrom<String> for PortRef {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<PortRef> for String {
    fn from(v: PortRef) -> String {
        v.to_string()
    }
}

impl fmt::Display for PortRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.component, self.connector, self.port)
    }
}

/// The source-of-truth description of an electrical system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub components: IndexMap<ComponentId, Component>,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

/// A named block with one or more connectors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub part_number: Option<String>,
    pub connectors: IndexMap<ConnectorId, Connector>,
}

impl Component {
    /// Resolve a `connector.port` within this component to its connector
    /// and pin. Returns `None` if either segment is unknown.
    pub fn lookup(&self, connector: &ConnectorId, port: &PortId) -> Option<PortInfo<'_>> {
        let conn = self.connectors.get(connector)?;
        let pin = conn.ports.get(port)?;
        Some(PortInfo {
            connector: conn,
            pin: pin.as_deref(),
        })
    }
}

/// A physical connector attached to a component, with one or more
/// ports.
///
/// Each port maps to an optional pin label (a free-form string like
/// `"1"`, `"A3"`, or `"B+"`). `None` means the connector has no
/// numbered position for that port — useful for things like bare
/// terminal studs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connector {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub part_number: Option<String>,
    pub ports: IndexMap<PortId, Option<String>>,
}

/// A link from one port to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub from: PortRef,
    pub to: PortRef,
}

impl FromStr for Model {
    type Err = Error;

    /// Parse a model from a YAML string. Errors don't carry a source
    /// path — use [`Model::load`] when you have one.
    fn from_str(text: &str) -> Result<Self> {
        serde_yml::from_str(text).map_err(Error::from)
    }
}

/// A `connector.port` resolved within a [`Component`]: the connector it
/// names and that port's pin, if any.
pub struct PortInfo<'a> {
    pub connector: &'a Connector,
    pub pin: Option<&'a str>,
}

impl Model {
    /// Read and parse a model from a YAML file. Errors carry the
    /// source path.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        text.parse::<Self>().map_err(|err| err.with_path(path))
    }

    /// Check referential integrity of `connections` and report any
    /// ports the model defines but no connection touches.
    ///
    /// Returns `Err` if any connection points at an unknown port.
    /// Otherwise returns a [`ValidationReport`] which may carry
    /// warnings.
    pub fn validate(&self) -> Result<ValidationReport> {
        let mut connected: IndexMap<PortRef, bool> = IndexMap::new();
        for (component_id, component) in &self.components {
            for (connector_id, connector) in &component.connectors {
                for port_id in connector.ports.keys() {
                    let port = PortRef {
                        component: component_id.clone(),
                        connector: connector_id.clone(),
                        port: port_id.clone(),
                    };
                    connected.insert(port, false);
                }
            }
        }

        for connection in &self.connections {
            for endpoint in [&connection.from, &connection.to] {
                match connected.get_mut(endpoint) {
                    Some(seen) => *seen = true,
                    None => {
                        return Err(Error::UnknownConnectionPort {
                            port: endpoint.to_string(),
                        });
                    }
                }
            }
        }

        let warnings = connected
            .into_iter()
            .filter_map(|(port, seen)| (!seen).then_some(Warning::UnconnectedPort(port)))
            .collect();

        Ok(ValidationReport { warnings })
    }
}

/// Outcome of validating a [`Model`] or [`crate::view::View`]. Errors
/// are surfaced as `Err`; non-fatal issues live here as warnings.
#[derive(Debug, Default)]
#[must_use]
pub struct ValidationReport {
    pub warnings: Vec<Warning>,
}

impl ValidationReport {
    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Append warnings from another report. Useful when validating the
    /// model and then a view in sequence.
    pub fn extend(&mut self, other: ValidationReport) {
        self.warnings.extend(other.warnings);
    }
}

/// A non-fatal issue surfaced by validation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Warning {
    /// A port is defined in the model but no connection touches it.
    UnconnectedPort(PortRef),
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnconnectedPort(port) => {
                write!(f, "port {port} is defined but not connected")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_ref_roundtrips_through_string() {
        let raw = "pack.hv.pos";
        let parsed: PortRef = raw.parse().expect("parses");
        assert_eq!(parsed.to_string(), raw);
    }

    #[test]
    fn port_ref_rejects_two_segments() {
        assert!(matches!(
            "pack.hv".parse::<PortRef>(),
            Err(Error::MalformedPortRef { .. })
        ));
    }

    #[test]
    fn port_ref_rejects_four_segments() {
        assert!(matches!(
            "pack.hv.pos.extra".parse::<PortRef>(),
            Err(Error::MalformedPortRef { .. })
        ));
    }

    #[test]
    fn component_id_rejects_dot() {
        assert!(matches!(
            "pa.ck".parse::<ComponentId>(),
            Err(Error::DottedIdentifier(_))
        ));
    }

    #[test]
    fn validate_warns_on_unconnected_port() {
        let yaml = r#"
components:
  pack:
    connectors:
      hv:
        ports:
          pos: "1"
          neg: "2"
connections:
  - { from: pack.hv.pos, to: pack.hv.neg }
"#;
        // pos and neg are mutually connected -> no warnings.
        let model: Model = yaml.parse().expect("parses");
        let report = model.validate().expect("validates");
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn validate_errors_on_dangling_connection() {
        let yaml = r#"
components:
  pack:
    connectors:
      hv:
        ports:
          pos: "1"
connections:
  - { from: pack.hv.pos, to: pack.hv.neg }
"#;
        let model: Model = yaml.parse().expect("parses");
        let err = model.validate().expect_err("dangling neg should fail");
        assert!(matches!(err, Error::UnknownConnectionPort { .. }));
    }

    #[test]
    fn from_str_on_bad_yaml_returns_yaml_error_without_path() {
        let bad = "components: [this is not a map\n";
        let err = bad.parse::<Model>().expect_err("bad yaml");
        assert!(matches!(err, Error::Yaml { path: None, .. }));
    }

    #[test]
    fn load_on_missing_file_returns_io_error() {
        let err = Model::load("/wirebug/definitely/not/a/file.yaml").expect_err("missing file");
        assert!(matches!(err, Error::Io { .. }));
    }

    #[test]
    fn load_attaches_path_to_yaml_error() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut f = NamedTempFile::new().expect("tempfile");
        writeln!(f, "components: [not a map").expect("write");

        let err = Model::load(f.path()).expect_err("bad yaml");
        let Error::Yaml {
            path: Some(path), ..
        } = err
        else {
            panic!("expected Yaml error with path");
        };
        assert_eq!(path, f.path());
    }

    #[test]
    fn validate_collects_unconnected_warning() {
        let yaml = r#"
components:
  pack:
    connectors:
      hv:
        ports:
          pos: "1"
          neg: "2"
connections: []
"#;
        let model: Model = yaml.parse().expect("parses");
        let report = model.validate().expect("validates");
        assert_eq!(report.warnings.len(), 2);
    }
}
