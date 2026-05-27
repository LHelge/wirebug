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
    View(View),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Definition(d) => d.span,
            Item::View(v) => v.span,
        }
    }
}

/// `component <name> { <members> }` — a component *type*.
#[derive(Debug, Clone)]
pub struct Definition {
    pub name: Spanned<Ident>,
    pub members: Vec<Member>,
    pub span: Span,
}

/// One entry inside a component body.
#[derive(Debug, Clone)]
pub enum Member {
    Port(Port),
    Connector(Connector),
    Instance(Instance),
    Wire(Wire),
    /// A nested (private) definition.
    Definition(Definition),
}

impl Member {
    pub fn span(&self) -> Span {
        match self {
            Member::Port(p) => p.span,
            Member::Connector(c) => c.span,
            Member::Instance(i) => i.span,
            Member::Wire(w) => w.span,
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

/// `connector "<part>" { <ports> }` — physical grouping metadata.
#[derive(Debug, Clone)]
pub struct Connector {
    pub part: Spanned<String>,
    pub ports: Vec<Port>,
    pub span: Span,
}

/// `<Type> <name> ["<label>"] ;` — a placement of a definition.
#[derive(Debug, Clone)]
pub struct Instance {
    pub type_name: Spanned<Ident>,
    pub name: Spanned<Ident>,
    pub label: Option<Spanned<String>>,
    pub span: Span,
}

/// `wire <colour> <gauge> [ <endpoint>, … ] ;`
#[derive(Debug, Clone)]
pub struct Wire {
    pub color: Spanned<Ident>,
    pub gauge: Spanned<f64>,
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

/// `view <kind> "<title>" { [grid N;] <includes> }`
#[derive(Debug, Clone)]
pub struct View {
    pub kind: Spanned<Ident>,
    pub title: Spanned<String>,
    pub grid: Option<Spanned<f64>>,
    pub includes: Vec<Include>,
    pub span: Span,
}

/// `include <instance> at (x, y) [ports { <side>: <port>, ...; ... }] ;`
#[derive(Debug, Clone)]
pub struct Include {
    pub instance: Spanned<Ident>,
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
