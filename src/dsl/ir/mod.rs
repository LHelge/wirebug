//! The elaborated intermediate representation.
//!
//! Identifier newtypes live here and are shared by resolution and
//! elaboration. The elaborated `Design` (a flat-map, hierarchical model)
//! lands in a later change; for now this module defines the names.

use std::fmt;

use indexmap::IndexMap;

macro_rules! name_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_newtype!(TypeName, "The name of a component definition (a type).");
name_newtype!(InstanceName, "The name of an instance within a component.");
name_newtype!(PortName, "The name of a port within a component.");

/// A physical connector pin (a positive integer in the DSL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pin(pub u32);

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The elaborated design: a flat map of every instance keyed by its path,
/// plus the views. The tree lives in each instance's `children` links;
/// no recursive ownership.
#[derive(Debug)]
pub struct Design {
    pub root: InstancePath,
    pub instances: IndexMap<InstancePath, Instance>,
    pub views: Vec<View>,
}

impl Design {
    /// The instance at `path`, if any.
    pub fn get(&self, path: &InstancePath) -> Option<&Instance> {
        self.instances.get(path)
    }
}

/// One elaborated instance (one node per placement).
#[derive(Debug)]
pub struct Instance {
    pub path: InstancePath,
    pub type_name: TypeName,
    pub label: Option<String>,
    pub ports: IndexMap<PortName, Port>,
    /// Local child name → its key into [`Design::instances`].
    pub children: IndexMap<InstanceName, InstancePath>,
    /// Wires declared at this level, resolved against this scope.
    pub wires: Vec<Wire>,
}

/// A materialized port on an instance.
#[derive(Debug)]
pub struct Port {
    pub name: PortName,
    pub label: String,
    pub visibility: Visibility,
    pub connector: Option<ConnectorRef>,
    pub pins: Vec<Pin>,
}

/// Whether a port is exposed to instantiators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

/// A port's connector grouping: the part description and its order index.
#[derive(Debug, Clone)]
pub struct ConnectorRef {
    pub part: String,
    pub index: usize,
}

/// A wire at one hierarchy level, with resolved endpoints.
#[derive(Debug)]
pub struct Wire {
    pub color: String,
    pub gauge: f64,
    pub endpoints: Vec<WireEnd>,
}

/// A resolved wire endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireEnd {
    /// The enclosing component's own port.
    Own(PortName),
    /// A direct child instance's port.
    Child {
        instance: InstanceName,
        port: PortName,
    },
}

/// A dotted instance path, e.g. `vehicle.front.module_1.pack`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct InstancePath(Vec<InstanceName>);

impl InstancePath {
    pub fn root(name: InstanceName) -> Self {
        Self(vec![name])
    }

    /// A child path with `name` appended.
    #[must_use]
    pub fn child(&self, name: InstanceName) -> Self {
        let mut segments = self.0.clone();
        segments.push(name);
        Self(segments)
    }

    pub fn segments(&self) -> &[InstanceName] {
        &self.0
    }
}

impl fmt::Display for InstancePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            f.write_str(seg.as_str())?;
        }
        Ok(())
    }
}

impl fmt::Debug for InstancePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// A view, bound to the component type it documents.
#[derive(Debug)]
pub struct View {
    pub kind: String,
    pub title: String,
    pub grid: Option<f64>,
    pub subject: TypeName,
    pub includes: Vec<Include>,
}

/// A view placement: an instance at grid coordinates, with its ports placed
/// on authored sides in declaration order. `ports` is empty for a bare box.
#[derive(Debug)]
pub struct Include {
    pub instance: InstanceName,
    pub x: f64,
    pub y: f64,
    pub ports: Vec<(PortName, Side)>,
}

/// Which side of a component box a port sits on, named by compass direction.
/// Authored in the view's `ports { }` block. In SVG coordinates y grows
/// downward, so North is the top edge and South the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    West,
    East,
    North,
    South,
}

impl std::str::FromStr for Side {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "west" => Side::West,
            "east" => Side::East,
            "north" => Side::North,
            "south" => Side::South,
            _ => return Err(()),
        })
    }
}
