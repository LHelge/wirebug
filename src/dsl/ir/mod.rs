//! The elaborated intermediate representation.
//!
//! Identifier newtypes live here and are shared by resolution and
//! elaboration, alongside the elaborated [`Design`] — a flat-map,
//! hierarchical model the renderer consumes directly.

use std::fmt;

use indexmap::IndexMap;

use crate::dsl::manifest::Manifest;

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
name_newtype!(
    ConnectorName,
    "A connector's reference designator (its addressable name in a view)."
);
name_newtype!(
    ConnectorTypeName,
    "The name of a reusable connector type definition."
);
name_newtype!(
    CableName,
    "A cable's designator, grouping its conductor wires."
);
/// One wire color, in the IEC 60757 vocabulary. `Other` preserves a name
/// outside the standard set verbatim (mirroring [`ViewKind::Other`]) —
/// validation warns on it, and `--strict` turns the warning into a
/// failure. Synonyms fold on parse (`purple` → `Violet`, `gray` →
/// `Grey`), so `Display`/`css()` emit one canonical name per color.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ColorName {
    Black,
    Brown,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Violet,
    Grey,
    White,
    Pink,
    Turquoise,
    Gold,
    Silver,
    Other(String),
}

impl ColorName {
    /// The CSS color this name strokes as. Every IEC 60757 color has an
    /// exact CSS name; `Other` passes through verbatim.
    pub fn css(&self) -> &str {
        match self {
            Self::Black => "black",
            Self::Brown => "brown",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Violet => "violet",
            Self::Grey => "grey",
            Self::White => "white",
            Self::Pink => "pink",
            Self::Turquoise => "turquoise",
            Self::Gold => "gold",
            Self::Silver => "silver",
            Self::Other(name) => name,
        }
    }

    /// The IEC 60757 code (`White` → `WH`); `Other` passes through
    /// verbatim, so exotic names stay readable rather than vanishing.
    pub fn code(&self) -> &str {
        match self {
            Self::Black => "BK",
            Self::Brown => "BN",
            Self::Red => "RD",
            Self::Orange => "OG",
            Self::Yellow => "YE",
            Self::Green => "GN",
            Self::Blue => "BU",
            Self::Violet => "VT",
            Self::Grey => "GY",
            Self::White => "WH",
            Self::Pink => "PK",
            Self::Turquoise => "TQ",
            Self::Gold => "GD",
            Self::Silver => "SR",
            Self::Other(name) => name,
        }
    }

    /// False for a name outside the IEC 60757 set (the `Other` escape
    /// hatch) — what validation warns about.
    pub fn is_standard(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

/// Case-insensitive, folding synonyms to their canonical color
/// (`purple` → `Violet`, `gray` → `Grey`). Total: anything unrecognised
/// lands in `Other` carrying the authored spelling.
impl From<&str> for ColorName {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "black" => Self::Black,
            "brown" => Self::Brown,
            "red" => Self::Red,
            "orange" => Self::Orange,
            "yellow" => Self::Yellow,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "violet" | "purple" => Self::Violet,
            "gray" | "grey" => Self::Grey,
            "white" => Self::White,
            "pink" => Self::Pink,
            "turquoise" => Self::Turquoise,
            "gold" => Self::Gold,
            "silver" => Self::Silver,
            _ => Self::Other(s.to_string()),
        }
    }
}

impl fmt::Display for ColorName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.css())
    }
}

/// A wire's color: an IEC 60757 base plus an optional tracer (stripe)
/// for two-tone wires (`green/yellow`). `Display` emits the canonical
/// form (`purple` normalises to `violet`) — the `data-color` value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WireColor {
    base: ColorName,
    tracer: Option<ColorName>,
}

impl WireColor {
    pub fn new(base: &str, tracer: Option<&str>) -> Self {
        Self {
            base: base.into(),
            tracer: tracer.map(ColorName::from),
        }
    }

    /// The CSS color a renderer strokes the wire body with — the base;
    /// a tracer is drawn as an overlay, never as the stroke.
    pub fn css(&self) -> &str {
        self.base.css()
    }

    /// The tracer (stripe) CSS color of a two-tone wire.
    pub fn tracer(&self) -> Option<&str> {
        self.tracer.as_ref().map(ColorName::css)
    }

    /// The IEC 60757 code (`white` → `WH`), a two-tone color as
    /// slash-joined codes (`green/yellow` → `GN/YE` — kept separated
    /// rather than the standard's concatenated `GNYE`, which is harder
    /// to scan on a dense drawing).
    pub fn code(&self) -> String {
        match &self.tracer {
            Some(tracer) => format!("{}/{}", self.base.code(), tracer.code()),
            None => self.base.code().to_string(),
        }
    }
}

/// Splits on `/`, so the authored surface form parses whole
/// (`"green/yellow"`).
impl From<&str> for WireColor {
    fn from(s: &str) -> Self {
        match s.split_once('/') {
            Some((base, tracer)) => Self::new(base, Some(tracer)),
            None => Self::new(s, None),
        }
    }
}

impl From<String> for WireColor {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

impl fmt::Display for WireColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.tracer {
            Some(tracer) => write!(f, "{}/{}", self.base, tracer),
            None => write!(f, "{}", self.base),
        }
    }
}

/// A supported view renderer, or an as-yet unknown kind preserved from the
/// DSL so render can report it precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Schematic,
    Harness,
    Pinout,
    Other(String),
}

impl ViewKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Schematic => "schematic",
            Self::Harness => "harness",
            Self::Pinout => "pinout",
            Self::Other(kind) => kind,
        }
    }

    pub fn is_schematic(&self) -> bool {
        matches!(self, Self::Schematic)
    }

    pub fn is_harness(&self) -> bool {
        matches!(self, Self::Harness)
    }

    pub fn is_pinout(&self) -> bool {
        matches!(self, Self::Pinout)
    }
}

impl From<&str> for ViewKind {
    fn from(kind: &str) -> Self {
        match kind {
            "schematic" => Self::Schematic,
            "harness" => Self::Harness,
            "pinout" => Self::Pinout,
            other => Self::Other(other.to_string()),
        }
    }
}

impl fmt::Display for ViewKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A physical connector pin (a positive integer in the DSL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pin(pub u32);

impl Pin {
    /// Render a connector pin assignment, or `None` when no pins are
    /// assigned.
    pub fn display_list(pins: &[Self]) -> Option<String> {
        if pins.is_empty() {
            return None;
        }
        Some(
            pins.iter()
                .map(Self::to_string)
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The elaborated design: a flat map of every instance keyed by its path,
/// plus the views. The tree lives in each instance's `children` links;
/// no recursive ownership. The project manifest rides along so the
/// renderer can stamp project identity (name, version, revision, …) on
/// every view; it's an `Option` so synthetic test designs don't need to
/// invent one.
#[derive(Debug)]
pub struct Design {
    pub root: InstancePath,
    pub instances: IndexMap<InstancePath, Instance>,
    pub views: Vec<View>,
    pub manifest: Option<Manifest>,
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
    /// Wires declared at this level, resolved against this scope. Wires that
    /// belong to a cable carry its name in [`Wire::cable`]; loose wires don't.
    pub wires: Vec<Wire>,
    /// Cable metadata declared at this level, keyed by designator. The cable's
    /// conductors live in `wires`, each tagged with this name.
    pub cables: IndexMap<CableName, CableMeta>,
    /// Physical connectors declared at this level, keyed by designator.
    pub connectors: IndexMap<ConnectorName, Connector>,
}

/// Physical metadata for a declared cable. Its conductor wires live in
/// [`Instance::wires`], each tagged with the cable's [`CableName`].
#[derive(Debug, Clone)]
pub struct CableMeta {
    pub label: Option<String>,
    /// Free-text construction note, e.g. `"Twisted pair"`.
    pub r#type: Option<String>,
    /// Length in metres.
    pub length: Option<f64>,
}

/// A materialized physical connector on an instance.
#[derive(Debug, Clone)]
pub struct Connector {
    pub name: ConnectorName,
    /// `None` for legacy inline connector blocks; reusable connector
    /// instances carry the top-level connector type name.
    pub type_name: Option<ConnectorTypeName>,
    /// Human-facing connector description or part label.
    pub description: String,
    /// Free-form connector metadata inherited from the connector type.
    pub properties: IndexMap<String, ConnectorPropertyValue>,
    /// Optional harness-side pinout layout inherited from the connector type.
    pub layout: Option<ConnectorLayout>,
    /// Authored pin bindings, preserving source order and allowing several
    /// pins to bind to one port for ganged high-current cavities.
    pub pins: Vec<ConnectorPin>,
}

/// A materialized connector pinout layout.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectorLayout {
    Grid(ConnectorGridLayout),
    Face(ConnectorFaceLayout),
}

/// Rectangular connector cavity layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorGridLayout {
    pub rows: u32,
    pub cols: u32,
    pub numbering: Option<String>,
}

/// Explicit physical connector face layout.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorFaceLayout {
    pub cavities: Vec<ConnectorCavity>,
}

/// One authored physical cavity.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorCavity {
    pub pin: Pin,
    pub x: f64,
    pub y: f64,
    pub size: Option<String>,
}

/// A connector metadata property value.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectorPropertyValue {
    Str(String),
    Number(f64),
}

/// One physical connector pin bound to a component port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorPin {
    pub pin: Pin,
    pub port: PortName,
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

/// A port's connector grouping: the optional designator, part description,
/// and its order index among the component's connectors.
#[derive(Debug, Clone)]
pub struct ConnectorRef {
    pub name: Option<ConnectorName>,
    pub part: String,
    pub index: usize,
}

/// A wire at one hierarchy level, with resolved endpoints.
#[derive(Debug)]
pub struct Wire {
    pub color: WireColor,
    pub gauge: f64,
    /// Optional signal name, shown on each wire in a harness drawing.
    pub label: Option<String>,
    pub endpoints: Vec<WireEnd>,
    /// The cable this conductor belongs to, if any. Loose wires are `None`.
    pub cable: Option<CableName>,
    /// The `twisted { }` group this conductor belongs to, numbered per
    /// cable in source order. Wires of one cable sharing an index are
    /// twisted together; the harness renderer braids two-conductor groups.
    pub twisted_group: Option<u32>,
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
    pub kind: ViewKind,
    pub title: String,
    pub grid: Option<f64>,
    pub subject: TypeName,
    /// Whether the view authored an `enclosure { }` block. Kept separate from
    /// the port list so an empty enclosure still draws the subject boundary.
    pub has_enclosure: bool,
    /// The subject's own ports drawn on the enclosure boundary (a box that
    /// wraps the schematic). Empty when no `enclosure { }` block is authored
    /// or when the authored block lists no ports.
    pub enclosure: Vec<EnclosurePort>,
    pub includes: Vec<Include>,
    pub texts: Vec<TextBox>,
}

/// A named annotation box placed at grid coordinates in a schematic view.
#[derive(Debug)]
pub struct TextBox {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub label: String,
}

/// One of the subject component's own ports, placed on the enclosure
/// boundary. `side` names the edge it sits on; `coord` is its position along
/// that edge in grid units (the free axis: y for west/east, x for
/// north/south).
#[derive(Debug)]
pub struct EnclosurePort {
    pub port: PortName,
    pub side: Side,
    pub coord: f64,
}

/// A view placement at grid coordinates.
///
/// A schematic include names a bare instance and authors port placements
/// in `ports` (side + order; empty for a bare box). A harness include names
/// a connector (`connector` is `Some`) and leaves `ports` empty; the whole
/// connector's pins are drawn.
#[derive(Debug)]
pub struct Include {
    pub instance: InstanceName,
    /// The connector designator for a harness include; `None` for schematic.
    pub connector: Option<ConnectorName>,
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

impl Side {
    /// The opposing edge. An inverted boundary port faces this way, so it
    /// labels like a normal port placed on the opposite side.
    pub fn opposite(self) -> Side {
        match self {
            Side::West => Side::East,
            Side::East => Side::West,
            Side::North => Side::South,
            Side::South => Side::North,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_display_list_is_absent_when_unassigned() {
        assert_eq!(Pin::display_list(&[]), None);
    }

    #[test]
    fn pin_display_list_joins_assigned_pins() {
        assert_eq!(
            Pin::display_list(&[Pin(1), Pin(2), Pin(10)]),
            Some("1,2,10".to_string())
        );
    }

    #[test]
    fn wire_color_codes_follow_iec_60757() {
        assert_eq!(WireColor::from("white").code(), "WH");
        assert_eq!(WireColor::from("green").code(), "GN");
        assert_eq!(WireColor::from("gray").code(), "GY");
        assert_eq!(WireColor::from("grey").code(), "GY");
        assert_eq!(WireColor::from("purple").code(), "VT");
    }

    #[test]
    fn wire_color_code_matching_ignores_case() {
        assert_eq!(WireColor::from("White").code(), "WH");
        assert_eq!(WireColor::from("RED").code(), "RD");
    }

    #[test]
    fn unknown_wire_color_code_passes_through() {
        assert_eq!(WireColor::from("chartreuse").code(), "chartreuse");
    }

    #[test]
    fn two_tone_color_joins_codes_with_a_slash() {
        assert_eq!(WireColor::from("green/yellow").code(), "GN/YE");
        assert_eq!(WireColor::from("white/blue").code(), "WH/BU");
    }

    #[test]
    fn two_tone_color_splits_base_and_tracer() {
        let color = WireColor::from("green/yellow");
        assert_eq!(color.css(), "green");
        assert_eq!(color.tracer(), Some("yellow"));
        assert_eq!(WireColor::from("green").tracer(), None);
    }

    #[test]
    fn wire_color_display_emits_the_canonical_form() {
        assert_eq!(WireColor::from("green/yellow").to_string(), "green/yellow");
        assert_eq!(WireColor::from("orange").to_string(), "orange");
        // Synonyms normalise — data-color themes against one vocabulary.
        assert_eq!(WireColor::from("purple").to_string(), "violet");
        assert_eq!(WireColor::from("gray/PURPLE").to_string(), "grey/violet");
        // Non-standard names keep their authored spelling.
        assert_eq!(WireColor::from("chartreuse").to_string(), "chartreuse");
    }

    #[test]
    fn every_standard_color_name_round_trips_via_css() {
        use ColorName::*;
        for color in [
            Black, Brown, Red, Orange, Yellow, Green, Blue, Violet, Grey, White, Pink, Turquoise,
            Gold, Silver,
        ] {
            assert_eq!(ColorName::from(color.css()), color);
            assert!(color.is_standard());
            assert_eq!(color.code().len(), 2, "{color:?}");
        }
        assert!(!ColorName::from("chartreuse").is_standard());
    }

    #[test]
    fn gold_and_silver_complete_the_iec_set() {
        assert_eq!(ColorName::from("gold").code(), "GD");
        assert_eq!(ColorName::from("silver").code(), "SR");
    }

    #[test]
    fn view_kind_classifies_known_kinds_and_preserves_unknown_ones() {
        assert_eq!(ViewKind::from("schematic"), ViewKind::Schematic);
        assert_eq!(ViewKind::from("harness"), ViewKind::Harness);
        assert_eq!(ViewKind::from("pinout"), ViewKind::Pinout);
        assert_eq!(ViewKind::from("bom"), ViewKind::Other("bom".to_string()));
        assert_eq!(ViewKind::from("bom").as_str(), "bom");
    }
}
