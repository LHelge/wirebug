//! The `.wb` abstract syntax tree.
//!
//! Structural nodes (`File`, `Definition`, `Port`, …) own their source
//! [`Span`]; small leaves and references (identifiers, labels, numbers)
//! are wrapped in [`Spanned`]. Every type reference is a raw, *unresolved*
//! [`Spanned<Ident>`] — the AST holds no resolved pointers, so parsing
//! stays pure and re-runnable. Resolution and elaboration are later passes.

use std::fmt;

use crate::dsl::span::{FileId, Span, Spanned};

/// A single lexed identifier (snake_case name, wire colour, view kind).
/// Never contains `.` — the lexer tokenises that separately.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Ident(pub String);

impl Ident {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Bare name keeps AST snapshots readable.
        f.write_str(&self.0)
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A parsed `.wb` file: its imports, then its top-level definitions and
/// views, in source order.
#[derive(Debug, Clone)]
pub struct File {
    pub id: FileId,
    pub uses: Vec<Use>,
    pub items: Vec<Item>,
    pub span: Span,
}

/// `use <name> from "<path>"` — brings one top-level definition into scope.
#[derive(Debug, Clone)]
pub struct Use {
    pub name: Spanned<Ident>,
    pub path: Spanned<String>,
    pub span: Span,
}

/// A top-level item: a component definition or a view.
#[derive(Debug, Clone)]
pub enum Item {
    Definition(Definition),
    ConnectorType(ConnectorType),
    View(View),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Definition(d) => d.span,
            Item::ConnectorType(c) => c.span,
            Item::View(v) => v.span,
        }
    }
}

/// `connector_type <name> "<description>" { <property>* [layout] }` — a
/// reusable physical connector definition with shared metadata and optional
/// harness-side pinout layout.
#[derive(Debug, Clone)]
pub struct ConnectorType {
    pub name: Spanned<Ident>,
    pub description: Spanned<String>,
    pub properties: Vec<ConnectorProperty>,
    pub layout: Option<ConnectorLayout>,
    pub span: Span,
}

/// Authored connector pinout layout. Coordinates are interpreted from the
/// harness side.
#[derive(Debug, Clone)]
pub enum ConnectorLayout {
    Grid(ConnectorGridLayout),
    Face(ConnectorFaceLayout),
}

/// `layout grid { rows: N; cols: N; [numbering: <mode>;] }`.
#[derive(Debug, Clone)]
pub struct ConnectorGridLayout {
    pub rows: Spanned<u32>,
    pub cols: Spanned<u32>,
    pub numbering: Option<Spanned<Ident>>,
    pub span: Span,
}

/// `layout face { cavity <pin> at (<x>, <y>) [size <name>]; ... }`.
#[derive(Debug, Clone)]
pub struct ConnectorFaceLayout {
    pub cavities: Vec<ConnectorCavity>,
    pub span: Span,
}

/// One authored physical cavity in a connector face layout.
#[derive(Debug, Clone)]
pub struct ConnectorCavity {
    pub pin: Spanned<u32>,
    pub x: Spanned<f64>,
    pub y: Spanned<f64>,
    pub size: Option<Spanned<Ident>>,
    pub span: Span,
}

/// Whether a definition introduces a component type or extends one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// `component <name> { … }` — introduces the type.
    Component,
    /// `extend <name> { … }` — a fragment merged into a same-named component.
    Extend,
}

/// `component <name> { <members> }` (or `extend <name> { … }`) — a
/// component *type*, possibly authored as one of several merged fragments.
#[derive(Debug, Clone)]
pub struct Definition {
    pub kind: DefKind,
    pub name: Spanned<Ident>,
    pub members: Vec<Member>,
    pub span: Span,
}

/// One entry inside a component body.
#[derive(Debug, Clone)]
pub enum Member {
    Port(Port),
    Connector(Connector),
    ConnectorInstance(ConnectorInstance),
    Instance(Instance),
    Wire(Wire),
    Cable(Cable),
    /// A nested (private) definition.
    Definition(Definition),
}

impl Member {
    pub fn span(&self) -> Span {
        match self {
            Member::Port(p) => p.span,
            Member::Connector(c) => c.span,
            Member::ConnectorInstance(c) => c.span,
            Member::Instance(i) => i.span,
            Member::Wire(w) => w.span,
            Member::Cable(c) => c.span,
            Member::Definition(d) => d.span,
        }
    }
}

/// Whether a port is exposed to instantiators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

/// `[pub] port <name> "<label>" [pin N | pins (N, …)] ;`
#[derive(Debug, Clone)]
pub struct Port {
    pub visibility: Visibility,
    pub name: Spanned<Ident>,
    pub label: Spanned<String>,
    /// Empty when no pin clause; one entry for `pin N`; many for `pins (…)`.
    pub pins: Vec<Spanned<u32>>,
    pub span: Span,
}

/// `connector [<name>] "<part>" { <ports> }` — physical grouping metadata.
/// The optional name is a reference designator addressing the connector in
/// a harness view (`include <inst>.<name>`); ports stay flat regardless.
#[derive(Debug, Clone)]
pub struct Connector {
    pub name: Option<Spanned<Ident>>,
    pub part: Spanned<String>,
    pub ports: Vec<Port>,
    pub span: Span,
}

/// `connector <name>: <type> { pin N = <port>; ... }` — a component-owned
/// connector instance backed by a reusable top-level connector type.
#[derive(Debug, Clone)]
pub struct ConnectorInstance {
    pub name: Spanned<Ident>,
    pub type_name: Spanned<Ident>,
    pub pins: Vec<PinBinding>,
    pub span: Span,
}

/// `pin N = <port>;` inside a [`ConnectorInstance`].
#[derive(Debug, Clone)]
pub struct PinBinding {
    pub pin: Spanned<u32>,
    pub port: Spanned<Ident>,
    pub span: Span,
}

/// `<key>: <value>;` inside a top-level `connector_type` body. Keys are
/// intentionally metadata for now; validation can tighten them later.
#[derive(Debug, Clone)]
pub struct ConnectorProperty {
    pub key: Spanned<Ident>,
    pub value: ConnectorPropertyValue,
    pub span: Span,
}

/// The right-hand side of a connector property — a quoted string or number.
#[derive(Debug, Clone)]
pub enum ConnectorPropertyValue {
    Str(Spanned<String>),
    Number(Spanned<f64>),
}

/// `<Type> <name> ["<label>"] ;` — a placement of a definition.
#[derive(Debug, Clone)]
pub struct Instance {
    pub type_name: Spanned<Ident>,
    pub name: Spanned<Ident>,
    pub label: Option<Spanned<String>>,
    pub span: Span,
}

/// `wire <colour>[/<tracer>] <gauge> ["<label>"] [ <endpoint>, … ] ;`
#[derive(Debug, Clone)]
pub struct Wire {
    pub color: Spanned<Ident>,
    /// The tracer (stripe) color of a two-tone wire: the part after the
    /// `/` in `green/yellow`.
    pub tracer: Option<Spanned<Ident>>,
    pub gauge: Spanned<f64>,
    /// Optional signal name, shown on each wire in a harness drawing.
    pub label: Option<Spanned<String>>,
    pub endpoints: Vec<Endpoint>,
    pub span: Span,
}

/// `inst.port` or bare `port` (the enclosing component's own port).
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// `None` for a bare reference to the enclosing component's own port.
    pub instance: Option<Spanned<Ident>>,
    pub port: Spanned<Ident>,
    pub span: Span,
}

/// `cable <name> ["<label>"] { <property>* <wire>* }` — a named bundle of
/// point-to-point conductors carrying physical metadata (`type`, `length`).
/// Each inner wire is a single conductor; arity (exactly two endpoints) and
/// property keys are checked later, so the parse stays faithful.
#[derive(Debug, Clone)]
pub struct Cable {
    pub name: Spanned<Ident>,
    pub label: Option<Spanned<String>>,
    pub properties: Vec<CableProperty>,
    pub members: Vec<CableMember>,
    pub span: Span,
}

impl Cable {
    /// Every conductor of the cable, loose and twisted alike, in source
    /// order — for the passes that don't care about grouping (endpoint
    /// resolution, arity, colors).
    pub fn wires(&self) -> impl Iterator<Item = &Wire> {
        self.members.iter().flat_map(|m| match m {
            CableMember::Wire(w) => std::slice::from_ref(w),
            CableMember::Twisted(t) => t.wires.as_slice(),
        })
    }
}

/// One conductor entry of a cable body: a plain wire, or a `twisted { }`
/// group of wires that are twisted together (a pair, typically).
#[derive(Debug, Clone)]
pub enum CableMember {
    Wire(Wire),
    Twisted(TwistedGroup),
}

/// `twisted { <wire>* }` inside a cable — the wrapped conductors are
/// twisted together. Group arity is checked in validation, not here.
#[derive(Debug, Clone)]
pub struct TwistedGroup {
    pub wires: Vec<Wire>,
    pub span: Span,
}

/// `<key>: <value>;` inside a `cable` body. Keys are validated in elaboration.
#[derive(Debug, Clone)]
pub struct CableProperty {
    pub key: Spanned<Ident>,
    pub value: CablePropertyValue,
    pub span: Span,
}

/// The right-hand side of a cable property — a quoted string, a number,
/// or a bare identifier (`twisted: true;` — booleans and future keyword
/// values parse faithfully; validation types them per key).
#[derive(Debug, Clone)]
pub enum CablePropertyValue {
    Str(Spanned<String>),
    Number(Spanned<f64>),
    Ident(Spanned<Ident>),
}

impl CablePropertyValue {
    pub fn span(&self) -> Span {
        match self {
            CablePropertyValue::Str(s) => s.span,
            CablePropertyValue::Number(n) => n.span,
            CablePropertyValue::Ident(i) => i.span,
        }
    }
}

/// `view <kind> "<title>" { <item>* }` where each item is a `grid N;`, an
/// `enclosure { ... }` block, an `include`, or a `text` box, in any order.
#[derive(Debug, Clone)]
pub struct View {
    pub kind: Spanned<Ident>,
    pub title: Spanned<String>,
    pub grid: Option<Spanned<f64>>,
    /// Whether an `enclosure { }` block was authored. Kept separate from the
    /// port list so an empty block still draws a boundary.
    pub has_enclosure: bool,
    /// The subject component's own ports drawn on the enclosure boundary.
    /// Empty when the `enclosure { }` block is absent. Refs and anchor shape
    /// are left unresolved; resolve validates them. Also empty for an authored
    /// empty enclosure.
    pub enclosure: Vec<EnclosurePort>,
    pub includes: Vec<Include>,
    pub texts: Vec<TextBox>,
    /// Spans of any `grid`/`enclosure` declared beyond the first (tagged with
    /// the item's kind). First-wins; resolve reports each as a duplicate.
    pub duplicate_items: Vec<Spanned<&'static str>>,
    pub span: Span,
}

/// `text <name> at (<x>, <y>) "<label>";` — a named annotation box placed at
/// grid coordinates in a schematic view.
#[derive(Debug, Clone)]
pub struct TextBox {
    pub name: Spanned<Ident>,
    pub x: Spanned<f64>,
    pub y: Spanned<f64>,
    pub label: Spanned<String>,
    pub span: Span,
}

/// One `<port> at (<x>, <y>)` placement inside the `enclosure { }` block,
/// where exactly one of `x`/`y` is a side keyword (west/east in the x slot,
/// north/south in the y slot) and the other a grid coordinate. The side names
/// the edge; the coordinate positions the port along the free axis. Resolve
/// enforces the shape.
#[derive(Debug, Clone)]
pub struct EnclosurePort {
    pub port: Spanned<Ident>,
    pub x: Anchor,
    pub y: Anchor,
    pub span: Span,
}

/// One slot of an enclosure port's `at (x, y)` anchor: a grid coordinate or
/// a side keyword pinning that axis to the enclosure edge.
#[derive(Debug, Clone)]
pub enum Anchor {
    Coord(Spanned<f64>),
    Edge(Spanned<Ident>),
}

impl Anchor {
    /// The slot's span, for diagnostics.
    pub fn span(&self) -> Span {
        match self {
            Anchor::Coord(c) => c.span,
            Anchor::Edge(e) => e.span,
        }
    }
}

/// `include <instance>[.<connector>] at (x, y) [ports { ... }] ;`
///
/// A schematic include names a bare instance and carries `ports`
/// placements; a harness include names `instance.connector` and carries no
/// `ports` block. Resolve enforces the per-kind shape.
#[derive(Debug, Clone)]
pub struct Include {
    pub instance: Spanned<Ident>,
    /// The connector designator for a harness include; `None` for schematic.
    pub connector: Option<Spanned<Ident>>,
    pub x: Spanned<f64>,
    pub y: Spanned<f64>,
    /// Authored port placements, flattened across the `ports { }` lines in
    /// declaration order. Empty when the block is absent. Side and port are
    /// left unresolved (`Spanned<Ident>`); resolve validates them.
    pub ports: Vec<PortPlacement>,
    pub span: Span,
}

/// One `<side>: <port>` placement inside an include's `ports { }` block. A
/// line `west: a, b;` expands to one placement per port, each tagged `west`.
#[derive(Debug, Clone)]
pub struct PortPlacement {
    pub side: Spanned<Ident>,
    pub port: Spanned<Ident>,
    pub span: Span,
}
