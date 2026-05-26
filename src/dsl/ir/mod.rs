//! The elaborated intermediate representation.
//!
//! Identifier newtypes live here and are shared by resolution and
//! elaboration. The elaborated `Design` (a flat-map, hierarchical model)
//! lands in a later change; for now this module defines the names.

use std::fmt;

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
