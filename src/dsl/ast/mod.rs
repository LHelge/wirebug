//! The `.wb` abstract syntax tree.
//!
//! Structural nodes (`File`, `Definition`, `Port`, …) own their source
//! [`Span`]; small leaves and references (identifiers, labels, numbers)
//! are wrapped in [`Spanned`]. Every type reference is a raw, *unresolved*
//! [`Spanned<Ident>`] — the AST holds no resolved pointers, so parsing
//! stays pure and re-runnable. Resolution and elaboration are later passes.

use std::fmt;

use crate::dsl::ir::{Length, LengthUnit};
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

/// A parsed `.wb` file: its imports (`use` and `fragment` statements), then
/// its top-level definitions and views, each in source order.
#[derive(Debug, Clone)]
pub struct File {
    pub id: FileId,
    pub uses: Vec<Use>,
    pub fragments: Vec<Fragment>,
    pub items: Vec<Item>,
    pub span: Span,
}

impl File {
    /// Every statement that names another file, `use` and `fragment` alike,
    /// in source order: the order the loader follows them in.
    pub fn imports(&self) -> Vec<Import<'_>> {
        let mut imports: Vec<Import<'_>> = self
            .uses
            .iter()
            .map(Import::Use)
            .chain(self.fragments.iter().map(Import::Fragment))
            .collect();
        imports.sort_by_key(|import| import.path().span.start);
        imports
    }

    /// Whether the file holds a view.
    pub fn has_views(&self) -> bool {
        self.items.iter().any(|item| matches!(item, Item::View(_)))
    }

    /// Whether a `fragment` of this file can add something to the file that
    /// names it: the file holds a top-level `extend` or a view.
    pub fn is_fragment_target(&self) -> bool {
        self.has_views()
            || self
                .items
                .iter()
                .any(|item| matches!(item, Item::Definition(def) if def.kind == DefKind::Extend))
    }
}

/// `use <name> from "<path>"` — brings one top-level definition into scope.
#[derive(Debug, Clone)]
pub struct Use {
    pub name: Spanned<Ident>,
    pub path: Spanned<String>,
    pub span: Span,
}

/// `fragment "<path>";` — loads a file into the project, as a `use` of it
/// would, without bringing any name into scope: the file's top-level
/// `extend`s merge with the components of their names in this file's
/// scope, and its views render.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub path: Spanned<String>,
    pub span: Span,
}

/// One statement of a [`File`] that names another file.
#[derive(Debug, Clone, Copy)]
pub enum Import<'a> {
    Use(&'a Use),
    Fragment(&'a Fragment),
}

impl<'a> Import<'a> {
    /// The path the statement names, as written (relative to its file).
    pub fn path(&self) -> &'a Spanned<String> {
        match self {
            Import::Use(use_decl) => &use_decl.path,
            Import::Fragment(fragment) => &fragment.path,
        }
    }
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

impl Definition {
    /// The type's `key: value;` part-data lines (`part: "…";`), in source
    /// order, wherever they sit among the other members.
    pub fn properties(&self) -> impl Iterator<Item = &Property> {
        self.members.iter().filter_map(|m| match m {
            Member::Property(p) => Some(p),
            _ => None,
        })
    }
}

/// One entry inside a component body.
#[derive(Debug, Clone)]
pub enum Member {
    Port(Port),
    Connector(Connector),
    ConnectorInstance(ConnectorInstance),
    Inline(Inline),
    Instance(Instance),
    Wire(Wire),
    Cable(Cable),
    /// A part-data line of the type (`part: "…";`): a `<name>:` followed
    /// by a value, where an instance has a type name. Keys and value kinds
    /// are checked in validate.
    Property(Property),
    /// A nested (private) definition.
    Definition(Definition),
}

impl Member {
    pub fn span(&self) -> Span {
        match self {
            Member::Port(p) => p.span,
            Member::Connector(c) => c.span,
            Member::ConnectorInstance(c) => c.span,
            Member::Inline(i) => i.span,
            Member::Instance(i) => i.span,
            Member::Wire(w) => w.span,
            Member::Cable(c) => c.span,
            Member::Property(p) => p.span,
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

/// `connector <name> ["<description>"] { <ports> }` — physical grouping
/// metadata. The name is the reference designator addressing the connector
/// in harness/pinout views (`include <inst>.<name>`); the optional string
/// describes the physical part for humans. Ports stay flat regardless.
#[derive(Debug, Clone)]
pub struct Connector {
    pub name: Spanned<Ident>,
    pub description: Option<Spanned<String>>,
    pub ports: Vec<Port>,
    pub span: Span,
}

/// `connector <name>: <Type> { <ports> }` — a component-owned connector
/// instance backed by a reusable top-level connector type. The body is the
/// same port-declaration list as an inline [`Connector`]; each port's
/// `pin`/`pins [..]` names the cavities it occupies in the type's layout.
#[derive(Debug, Clone)]
pub struct ConnectorInstance {
    pub name: Spanned<Ident>,
    pub type_name: Spanned<Ident>,
    pub ports: Vec<Port>,
    pub span: Span,
}

/// `inline <name> ["<label>"] { <half>* <port>* }` — an inline (mid-harness)
/// connector member: a mated plug/receptacle pair with one shared port set.
/// Distinct from the "inline connector block" form of [`Connector`] — this is
/// a first-class member addressed like an instance in wire endpoints
/// (`ic_pedal.apps1`) and included by half in harness views
/// (`include ic_pedal.female`).
#[derive(Debug, Clone)]
pub struct Inline {
    pub name: Spanned<Ident>,
    pub label: Option<Spanned<String>>,
    /// `male: <Type>;` / `female: <Type>;` lines, kept faithful — keys are
    /// validated in resolve, not the grammar.
    pub halves: Vec<InlineHalfDecl>,
    pub ports: Vec<Port>,
    pub span: Span,
}

/// One `<key>: <Type>;` housing-half line inside an `inline` body.
#[derive(Debug, Clone)]
pub struct InlineHalfDecl {
    /// `male` or `female` — checked in resolve so other keys can diagnose.
    pub key: Spanned<Ident>,
    /// A `connector_type` reference, resolved through the file's `use` scope.
    pub type_name: Spanned<Ident>,
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

/// `wire <colour>[/<tracer>] <gauge> ["<label>"] [ <endpoint>, … ] ;`, or
/// the same with a `{ <property>* }` body in place of the `;`.
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
    /// The `key: value;` lines of the optional body (`length`, `type`,
    /// `part`), kept faithful like a cable's: keys and value kinds are
    /// checked in validate. Empty when the wire has no body.
    pub properties: Vec<Property>,
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

/// `cable <name> ["<label>"] { <property>* <wire>* <shield>* }` — a named
/// bundle of point-to-point conductors carrying physical metadata (`type`,
/// `length`), and its overall screen. Each inner wire is a single
/// conductor; arity (exactly two endpoints), property keys and the number
/// of shields (at most one) are checked later, so the parse stays faithful.
#[derive(Debug, Clone)]
pub struct Cable {
    pub name: Spanned<Ident>,
    pub label: Option<Spanned<String>>,
    pub properties: Vec<Property>,
    pub members: Vec<CableMember>,
    /// The cable body's own `shield` lines (its overall screen), in source
    /// order; a twisted pair's are in its [`TwistedGroup::shields`].
    pub shields: Vec<Shield>,
    pub span: Span,
}

impl Cable {
    /// Every conductor of the cable, loose and twisted alike, in source
    /// order — for the passes that don't care about grouping (endpoint
    /// resolution, arity, colors).
    pub fn wires(&self) -> impl Iterator<Item = &Wire> {
        self.members.iter().flat_map(|m| match m {
            CableMember::Wire(w) => std::slice::from_ref(&**w),
            CableMember::Twisted(t) => t.wires.as_slice(),
        })
    }

    /// The cable's twisted pairs, in source order.
    pub fn twisted_groups(&self) -> impl Iterator<Item = &TwistedGroup> {
        self.members.iter().filter_map(|m| match m {
            CableMember::Twisted(t) => Some(t),
            _ => None,
        })
    }

    /// Every endpoint the cable names, its conductors' and then its
    /// shields' (the cable's own, then each twisted pair's): the
    /// references that resolve against the enclosing component.
    pub fn endpoints(&self) -> impl Iterator<Item = &Endpoint> {
        let shields = self
            .shields
            .iter()
            .chain(self.twisted_groups().flat_map(|t| &t.shields));
        self.wires()
            .flat_map(|w| &w.endpoints)
            .chain(shields.flat_map(|s| &s.ends))
    }
}

/// One conductor entry of a cable body: a plain wire, or a `twisted { }`
/// group of wires that are twisted together (a pair, typically). The wire
/// is boxed, like the pair, so the two variants stay close in size.
#[derive(Debug, Clone)]
pub enum CableMember {
    Wire(Box<Wire>),
    Twisted(TwistedGroup),
}

/// `twisted { <wire> <wire> <shield>* }` inside a cable — a twisted pair,
/// individually screened when it holds a `shield` line. Exactly two
/// conductors, enforced by the grammar itself: a group of any other size
/// is a parse error, not a representable state. (Widen to a `Vec` if
/// twisted triples/quads ever land.) Boxed so the pair doesn't dominate
/// [`CableMember`]'s size.
#[derive(Debug, Clone)]
pub struct TwistedGroup {
    pub wires: Box<[Wire; 2]>,
    /// The pair's own `shield` lines, in source order; validate allows one.
    pub shields: Vec<Shield>,
    pub span: Span,
}

/// `shield ["<label>"] [<end>, …];`, or `shield ["<label>"];` — the screen
/// of a cable or of a twisted pair, and the pins its drain is terminated
/// on: none (the screen lands on no pin: cut back, or bonded at a gland or
/// housing), one (the usual single-ended termination) or two. It has no color and no gauge. Kept faithful: the
/// number of ends is checked in validate.
#[derive(Debug, Clone)]
pub struct Shield {
    pub label: Option<Spanned<String>>,
    /// Empty for a shield that lands on no pin.
    pub ends: Vec<Endpoint>,
    pub span: Span,
}

/// `<key>: <value>;` inside a `cable` body, a wire's body or a component
/// body. Keys and value kinds are checked in validate, per owner.
#[derive(Debug, Clone)]
pub struct Property {
    pub key: Spanned<Ident>,
    pub value: PropertyValue,
    pub span: Span,
}

/// The right-hand side of a property — a quoted string, a number, or a
/// bare identifier (`twisted: true;` — booleans and future keyword values
/// parse faithfully; validation types them per key).
#[derive(Debug, Clone)]
pub enum PropertyValue {
    Str(Spanned<String>),
    /// Spanned over the number and its unit (`150mm`).
    Number(Spanned<Quantity>),
    Ident(Spanned<Ident>),
}

/// A number as a property value: its value and the unit written directly
/// after it (`mm` in `150mm`), kept faithful like the key. Which units a
/// key takes is checked in validate.
#[derive(Debug, Clone)]
pub struct Quantity {
    pub value: f64,
    pub unit: Option<Spanned<String>>,
}

impl Quantity {
    /// The unit this quantity is a length in: metres when it has none, and
    /// `None` when its unit is no length unit.
    pub fn length_unit(&self) -> Option<LengthUnit> {
        self.unit
            .as_ref()
            .map_or(Some(LengthUnit::Metre), |unit| unit.node.parse().ok())
    }

    /// The length this quantity is; `None` when its unit is no length unit
    /// or the length is not positive (validate reports both).
    pub fn length(&self) -> Option<Length> {
        Length::new(self.value, self.length_unit()?)
    }
}

/// As written, unit included: `150mm`, `-2`.
impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = self.unit.as_ref().map_or("", |unit| unit.node.as_str());
        write!(f, "{}{unit}", self.value)
    }
}

impl PropertyValue {
    pub fn span(&self) -> Span {
        match self {
            PropertyValue::Str(s) => s.span,
            PropertyValue::Number(n) => n.span,
            PropertyValue::Ident(i) => i.span,
        }
    }
}

/// `view <kind> "<title>" [for <Component>] { <item>* }` where each item is
/// a `grid N;`, an `enclosure { ... }` block, an `include`, or a `text` box,
/// in any order.
#[derive(Debug, Clone)]
pub struct View {
    pub kind: Spanned<Ident>,
    pub title: Spanned<String>,
    /// The component named after `for`: the view's explicit subject. `None`
    /// when `for` is left out, and the view documents the file's single
    /// top-level component.
    pub subject: Option<Spanned<Ident>>,
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

/// `text "<label>" at (<x>, <y>);` — an annotation box placed at grid
/// coordinates in a schematic view.
#[derive(Debug, Clone)]
pub struct TextBox {
    /// The name of the deprecated form, `text <name> at (<x>, <y>)
    /// "<label>";`, which resolve warns about; `None` for the current form,
    /// whose box has no name.
    pub name: Option<Spanned<Ident>>,
    pub x: Spanned<f64>,
    pub y: Spanned<f64>,
    pub label: Spanned<String>,
    pub span: Span,
}

impl TextBox {
    /// The statement in the current form, `text "<label>" at (<x>, <y>);`,
    /// whatever form it was written in.
    pub fn current_form(&self) -> String {
        format!(
            "text \"{}\" at ({}, {});",
            self.label.node, self.x.node, self.y.node
        )
    }
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
