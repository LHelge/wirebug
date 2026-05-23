//! Renderable view over a [`crate::model::Model`].
//!
//! A view owns presentation: which renderer to dispatch, the title, the
//! position of each component box, and which side of each box every
//! port appears on (and in what order). The model knows nothing about
//! any of that.

use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{ComponentId, ConnectorId, Model, PortId, ValidationReport};

/// A renderable description of part (or all) of a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    pub kind: ViewKind,
    #[serde(default)]
    pub title: Option<String>,
    pub layout: IndexMap<ComponentId, Point>,
    #[serde(default)]
    pub ports: IndexMap<ComponentId, ComponentPortLayout>,
}

/// Which renderer should handle this view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewKind {
    Schematic,
}

/// 2D point in SVG user-space units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Per-component placement of ports on the four sides of its rectangle.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComponentPortLayout {
    #[serde(default)]
    pub left: Vec<ConnectorPortRef>,
    #[serde(default)]
    pub right: Vec<ConnectorPortRef>,
    #[serde(default)]
    pub top: Vec<ConnectorPortRef>,
    #[serde(default)]
    pub bottom: Vec<ConnectorPortRef>,
}

impl ComponentPortLayout {
    /// Iterate the four sides in a fixed order (Left, Right, Top,
    /// Bottom). Useful for both validation and rendering.
    pub fn sides(&self) -> [(Side, &Vec<ConnectorPortRef>); 4] {
        [
            (Side::Left, &self.left),
            (Side::Right, &self.right),
            (Side::Top, &self.top),
            (Side::Bottom, &self.bottom),
        ]
    }
}

/// Which side of a component rectangle a port sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Component-scoped port reference: `connector.port` (the component is
/// implicit from context).
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ConnectorPortRef {
    pub connector: ConnectorId,
    pub port: PortId,
}

impl FromStr for ConnectorPortRef {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        match parts.as_slice() {
            [k, p] => Ok(Self {
                connector: k.parse()?,
                port: p.parse()?,
            }),
            _ => Err(Error::MalformedConnectorPortRef { raw: s.to_string() }),
        }
    }
}

impl TryFrom<String> for ConnectorPortRef {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<ConnectorPortRef> for String {
    fn from(v: ConnectorPortRef) -> String {
        v.to_string()
    }
}

impl fmt::Display for ConnectorPortRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.connector, self.port)
    }
}

impl FromStr for View {
    type Err = Error;

    /// Parse a view from a YAML string. Errors don't carry a source
    /// path — use [`View::load`] when you have one.
    fn from_str(text: &str) -> Result<Self> {
        serde_yml::from_str(text).map_err(Error::from)
    }
}

impl View {
    /// Read and parse a view from a YAML file. Errors carry the source
    /// path.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        text.parse::<Self>().map_err(|err| err.with_path(path))
    }

    /// Components included in this view, in their layout-declaration
    /// order.
    pub fn component_ids(&self) -> impl Iterator<Item = &ComponentId> {
        self.layout.keys()
    }

    /// Check that every reference in the view resolves against the
    /// given model. A view's `ports:` keys must appear in `layout:`,
    /// and every per-side ref must exist on the named component.
    pub fn validate(&self, model: &Model) -> Result<ValidationReport> {
        for component_id in self.layout.keys() {
            if !model.components.contains_key(component_id) {
                return Err(Error::UnknownLayoutComponent {
                    component: component_id.to_string(),
                });
            }
        }

        for (component_id, layout) in &self.ports {
            if !self.layout.contains_key(component_id) {
                return Err(Error::PortsWithoutLayout {
                    component: component_id.to_string(),
                });
            }
            let component =
                model
                    .components
                    .get(component_id)
                    .ok_or_else(|| Error::UnknownViewComponent {
                        component: component_id.to_string(),
                    })?;

            for (_side, refs) in layout.sides() {
                for cp in refs {
                    let connector = component.connectors.get(&cp.connector).ok_or_else(|| {
                        Error::UnknownViewPort {
                            component: component_id.to_string(),
                            connector: cp.connector.to_string(),
                            port: cp.port.to_string(),
                        }
                    })?;
                    if !connector.ports.contains_key(&cp.port) {
                        return Err(Error::UnknownViewPort {
                            component: component_id.to_string(),
                            connector: cp.connector.to_string(),
                            port: cp.port.to_string(),
                        });
                    }
                }
            }
        }

        Ok(ValidationReport::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_model() -> Model {
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
        yaml.parse().expect("model parses")
    }

    #[test]
    fn connector_port_ref_roundtrips() {
        let raw = "hv.pos";
        let parsed: ConnectorPortRef = raw.parse().expect("parses");
        assert_eq!(parsed.to_string(), raw);
    }

    #[test]
    fn connector_port_ref_rejects_three_segments() {
        assert!(matches!(
            "hv.pos.extra".parse::<ConnectorPortRef>(),
            Err(Error::MalformedConnectorPortRef { .. })
        ));
    }

    #[test]
    fn view_validate_accepts_well_formed_subset() {
        let view_yaml = r#"
kind: schematic
layout:
  pack: { x: 0, y: 0 }
ports:
  pack:
    right: [hv.pos, hv.neg]
"#;
        let view: View = view_yaml.parse().expect("parses");
        let report = view.validate(&tiny_model()).expect("validates");
        assert!(report.is_empty());
    }

    #[test]
    fn view_validate_rejects_layout_pointing_at_unknown_component() {
        let view_yaml = r#"
kind: schematic
layout:
  mystery: { x: 0, y: 0 }
"#;
        let view: View = view_yaml.parse().expect("parses");
        let err = view.validate(&tiny_model()).expect_err("unknown component");
        assert!(matches!(err, Error::UnknownLayoutComponent { .. }));
    }

    #[test]
    fn view_validate_rejects_ports_without_layout() {
        let view_yaml = r#"
kind: schematic
layout: {}
ports:
  pack:
    right: [hv.pos]
"#;
        let view: View = view_yaml.parse().expect("parses");
        let err = view.validate(&tiny_model()).expect_err("ports w/o layout");
        assert!(matches!(err, Error::PortsWithoutLayout { .. }));
    }

    #[test]
    fn view_validate_rejects_unknown_port() {
        let view_yaml = r#"
kind: schematic
layout:
  pack: { x: 0, y: 0 }
ports:
  pack:
    right: [hv.ghost]
"#;
        let view: View = view_yaml.parse().expect("parses");
        let err = view.validate(&tiny_model()).expect_err("unknown port");
        assert!(matches!(err, Error::UnknownViewPort { .. }));
    }
}
