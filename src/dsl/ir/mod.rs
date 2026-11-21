//! The elaborated intermediate representation.
//!
//! Identifier newtypes live here and are shared by resolution and
//! elaboration, alongside the elaborated [`Design`] — a flat-map,
//! hierarchical model the renderer consumes directly.

use std::fmt;

use indexmap::IndexMap;

use crate::dsl::ast;
use crate::dsl::manifest::Manifest;
use crate::dsl::span::Span;

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
name_newtype!(
    SpliceName,
    "A splice's name: a joint inside a component that conductors end on."
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
    /// Every standard IEC 60757 color, in declaration order — the closed
    /// set completion and documentation enumerate.
    pub const STANDARD: [Self; 14] = [
        Self::Black,
        Self::Brown,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Violet,
        Self::Grey,
        Self::White,
        Self::Pink,
        Self::Turquoise,
        Self::Gold,
        Self::Silver,
    ];

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

    /// The instance `view` is drawn against: the first one of the type it
    /// documents (the root for a view of the root component). Synthetic
    /// inline-connector instances are skipped — their `type_name` is the
    /// inline's designator, which could shadow a real component type, and
    /// an inline is never a view subject. `None` when the design never
    /// instantiates the type.
    pub(crate) fn subject_of(&self, view: &View) -> Option<&Instance> {
        self.instances
            .values()
            .find(|inst| inst.inline.is_none() && inst.type_name == view.subject)
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
    /// `Some` marks this as a synthetic inline-connector instance — a mated
    /// mid-harness pair elaborated from an `inline` member. Its port set is
    /// shared by both housing halves; the halves' part metadata lives here.
    /// `None` for every ordinary instance.
    pub inline: Option<InlineMeta>,
    /// The part data of the instance's type (its `part:`, `manufacturer:`,
    /// … lines, merged across `extend` fragments): every placement of the
    /// type carries the same. Empty for a synthetic inline instance, whose
    /// halves' part data is in `inline`.
    pub part: PartMeta,
    /// The joints declared at this level, keyed by name, in source order.
    /// The conductors ending on one are the wires in `wires` with a
    /// [`WireEnd::Splice`] end naming it ([`Instance::splice_conductors`]).
    pub splices: IndexMap<SpliceName, Splice>,
}

/// A joint inside a component — a crimp or solder splice in a harness —
/// that conductors end on. All conductors ending on one splice are joined
/// electrically; each stays a two-ended [`Wire`] of its own, with its own
/// color, gauge and length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub name: SpliceName,
    /// Optional human name, shown on the splice in a harness drawing.
    pub label: Option<String>,
    /// The part number of the splice itself, from its body's `part:`.
    pub part: Option<String>,
    /// What it is, for humans, from its body's `description:`.
    pub description: Option<String>,
    /// The authored `splice` statement.
    pub span: Span,
}

/// A `splice` member, its body's well-typed `part` and `description` kept
/// (the first of each); unknown keys, values of the wrong kind and repeats
/// are reported in validate.
impl From<&ast::Splice> for Splice {
    fn from(splice: &ast::Splice) -> Self {
        let mut part = None;
        let mut description = None;
        for p in &splice.properties {
            let ast::PropertyValue::Str(value) = &p.value else {
                continue;
            };
            let field = match p.key.node.as_str() {
                "part" => &mut part,
                "description" => &mut description,
                _ => continue,
            };
            field.get_or_insert_with(|| value.node.clone());
        }
        Self {
            name: SpliceName::from(splice.name.node.as_str()),
            label: splice.label.as_ref().map(|l| l.node.clone()),
            part,
            description,
            span: splice.span,
        }
    }
}

/// The conductors one splice joins, through any splice-to-splice links:
/// one electrical net. See [`Instance::splice_net`].
#[derive(Debug, Clone)]
pub struct SpliceNet<'a> {
    /// The splices of the net, the one asked about first, then the ones
    /// reached through links in the order they are found.
    pub splices: Vec<&'a SpliceName>,
    /// Each conductor end of the net that is not a splice, in wire order,
    /// with the conductor it is an end of.
    pub ends: Vec<NetEnd<'a>>,
}

/// One end a [`SpliceNet`] reaches beyond its splices: a port, and the
/// conductor that runs from it to one of the net's splices.
#[derive(Debug, Clone, Copy)]
pub struct NetEnd<'a> {
    pub end: &'a WireEnd,
    pub wire: &'a Wire,
}

/// What a component type is bought as, from the `key: value;` lines of its
/// body. Every field is optional: a type without part data is still a
/// component, only one a bill of materials cannot name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartMeta {
    /// The manufacturer part number, from `part:`.
    pub number: Option<String>,
    /// Who makes the part, e.g. `"TE Connectivity"`.
    pub manufacturer: Option<String>,
    /// A human description of the part, e.g. `"Main contactor, 12 V coil"`.
    pub description: Option<String>,
    /// A free-text rating, e.g. `"500 A"`: text, since ratings mix units.
    pub rating: Option<String>,
}

/// A type's part data from its authored property lines, in source order
/// (the root fragment's first). Unknown keys, values of the wrong kind and
/// repeats are reported in validate; here the first well-typed value of
/// each key is kept and the rest dropped.
impl<'a> FromIterator<&'a ast::Property> for PartMeta {
    fn from_iter<I: IntoIterator<Item = &'a ast::Property>>(properties: I) -> Self {
        let mut meta = PartMeta::default();
        for p in properties {
            let ast::PropertyValue::Str(value) = &p.value else {
                continue;
            };
            let field = match p.key.node.as_str() {
                "part" => &mut meta.number,
                "manufacturer" => &mut meta.manufacturer,
                "description" => &mut meta.description,
                "rating" => &mut meta.rating,
                _ => continue,
            };
            field.get_or_insert_with(|| value.node.clone());
        }
        meta
    }
}

impl Instance {
    /// The conductors ending on `splice`, in wire order: each a row of its
    /// own in a wire list, with the splice as one of its ends.
    pub fn splice_conductors<'a, 's>(
        &'a self,
        splice: &'s SpliceName,
    ) -> impl Iterator<Item = &'a Wire> + use<'a, 's> {
        self.wires.iter().filter(move |wire| wire.ends_on(splice))
    }

    /// The net `splice` joins: every splice linked to it by a conductor
    /// from splice to splice, and every other end of the conductors of
    /// those splices. Empty of ends for a splice nothing ends on, and
    /// empty altogether for a name this instance declares no splice of.
    pub fn splice_net(&self, splice: &SpliceName) -> SpliceNet<'_> {
        let mut splices: Vec<&SpliceName> = self
            .splices
            .get_key_value(splice)
            .map(|(name, _)| name)
            .into_iter()
            .collect();
        let mut next = 0;
        while let Some(&current) = splices.get(next) {
            next += 1;
            for wire in self.splice_conductors(current) {
                for end in &wire.endpoints {
                    if let WireEnd::Splice(other) = end
                        && let Some((other, _)) = self.splices.get_key_value(other)
                        && !splices.contains(&other)
                    {
                        splices.push(other);
                    }
                }
            }
        }
        let ends = self
            .wires
            .iter()
            .filter(|wire| splices.iter().any(|s| wire.ends_on(s)))
            .flat_map(|wire| {
                wire.endpoints
                    .iter()
                    .filter(|end| !matches!(end, WireEnd::Splice(_)))
                    .map(move |end| NetEnd { end, wire })
            })
            .collect();
        SpliceNet { splices, ends }
    }

    /// One [`SpliceNet`] per group of linked splices, in the order their
    /// first splice is declared.
    pub fn splice_nets(&self) -> Vec<SpliceNet<'_>> {
        let mut nets: Vec<SpliceNet<'_>> = Vec::new();
        for name in self.splices.keys() {
            if !nets.iter().any(|net| net.splices.contains(&name)) {
                nets.push(self.splice_net(name));
            }
        }
        nets
    }

    /// The length that applies to `wire`, one of this instance's wires: its
    /// own `length`, else that of the cable it belongs to. `None` when
    /// neither sets one.
    pub fn effective_length(&self, wire: &Wire) -> Option<Length> {
        wire.length.or_else(|| {
            wire.cable
                .as_ref()
                .and_then(|cable| self.cables.get(cable))
                .and_then(|meta| meta.length)
        })
    }
}

/// Physical metadata for a declared cable. Its conductor wires live in
/// [`Instance::wires`], each tagged with the cable's [`CableName`].
#[derive(Debug, Clone)]
pub struct CableMeta {
    pub label: Option<String>,
    /// Free-text construction note, e.g. `"Twisted pair"`.
    pub r#type: Option<String>,
    pub length: Option<Length>,
    /// The cable's overall screen, from a `shield` line of its body. `None`
    /// for an unscreened cable.
    pub shield: Option<Shield>,
    /// The cable's `twisted { }` pairs, indexed by the number its
    /// conductors carry in [`Wire::twisted_group`].
    pub twisted: Vec<TwistedMeta>,
}

impl CableMeta {
    /// The `twisted { }` pair numbered `group` (a conductor's
    /// [`Wire::twisted_group`]).
    pub fn twisted_group(&self, group: u32) -> Option<&TwistedMeta> {
        self.twisted.get(usize::try_from(group).ok()?)
    }

    /// Every screen of the cable: its overall one, then each pair's own,
    /// with what it screens.
    pub fn shields(&self) -> impl Iterator<Item = (Screened, &Shield)> {
        let pairs = (0u32..).zip(&self.twisted).filter_map(|(group, pair)| {
            pair.shield
                .as_ref()
                .map(|shield| (Screened::Pair(group), shield))
        });
        self.shield
            .iter()
            .map(|shield| (Screened::Cable, shield))
            .chain(pairs)
    }
}

/// One `twisted { }` pair of a cable. Its two conductors stay in
/// [`Instance::wires`], each carrying the pair's number.
#[derive(Debug, Clone, Default)]
pub struct TwistedMeta {
    /// The pair's own screen, when it is individually shielded.
    pub shield: Option<Shield>,
}

/// What a [`Shield`] screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screened {
    /// The whole cable: its overall screen.
    Cable,
    /// One twisted pair, by its [`Wire::twisted_group`] number.
    Pair(u32),
}

/// The screen of a cable or of a twisted pair: not a conductor (no color,
/// no gauge), only the pins its drain is terminated on.
#[derive(Debug, Clone)]
pub struct Shield {
    /// Optional name, shown on the drain in a harness drawing.
    pub label: Option<String>,
    pub termination: ShieldTermination,
    /// The authored `shield` statement.
    pub span: Span,
}

impl Shield {
    /// The pins the drain lands on, in source order: none, one or two.
    pub fn ends(&self) -> &[WireEnd] {
        self.termination.ends()
    }
}

/// The pins a shield's drain is terminated on. A screen runs from one end
/// of its cable to the other, so it lands on two pins at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShieldTermination {
    /// Lands on no pin: cut back, or bonded at a gland or housing rather
    /// than on a contact. The model does not know which.
    NoPin,
    /// Grounded at one end, the usual termination.
    Single(WireEnd),
    /// Grounded at both ends.
    Both([WireEnd; 2]),
}

impl ShieldTermination {
    /// The pins the drain lands on, in source order.
    pub fn ends(&self) -> &[WireEnd] {
        match self {
            Self::NoPin => &[],
            Self::Single(end) => std::slice::from_ref(end),
            Self::Both(ends) => ends,
        }
    }
}

/// A termination from the ends a `shield` line lists, the first two of
/// them: validate reports a shield that lists more.
impl FromIterator<WireEnd> for ShieldTermination {
    fn from_iter<I: IntoIterator<Item = WireEnd>>(ends: I) -> Self {
        let mut ends = ends.into_iter();
        match (ends.next(), ends.next()) {
            (None, _) => Self::NoPin,
            (Some(end), None) => Self::Single(end),
            (Some(first), Some(second)) => Self::Both([first, second]),
        }
    }
}

/// A physical length: a cable's or a conductor's. Held as a whole number of
/// micrometres, so lengths authored in different units compare and print
/// exactly — `150mm` is `0.15 m`, never `0.15000000000000002 m` — and a
/// sum of them would too. Always positive: [`Length::new`] refuses the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Length {
    micrometres: u64,
}

impl Length {
    const MICROMETRES_PER_METRE: u64 = 1_000_000;

    /// `value` in `unit`, to the nearest micrometre. `None` unless that is
    /// at least one micrometre: a length is positive.
    #[must_use]
    pub fn new(value: f64, unit: LengthUnit) -> Option<Self> {
        let micrometres = (value * unit.micrometres() as f64).round();
        // A float-to-int `as` saturates; no wiring length comes near.
        (micrometres >= 1.0).then_some(Length {
            micrometres: micrometres as u64,
        })
    }

    /// The length in metres, the unit the IR normalises to.
    pub fn metres(self) -> f64 {
        self.micrometres as f64 / Self::MICROMETRES_PER_METRE as f64
    }
}

/// In metres, with as many decimals as the length has and no more:
/// `2 m`, `0.15 m`, `0.000001 m`.
impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.micrometres / Self::MICROMETRES_PER_METRE;
        let fraction = self.micrometres % Self::MICROMETRES_PER_METRE;
        if fraction == 0 {
            return write!(f, "{whole} m");
        }
        let decimals = format!("{fraction:06}");
        write!(f, "{whole}.{} m", decimals.trim_end_matches('0'))
    }
}

/// A unit a length may be written in, as a suffix touching its number
/// (`150mm`). A bare number is in metres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Metre,
    Centimetre,
    Millimetre,
}

impl LengthUnit {
    /// Every unit, in the order the diagnostics and completion list them.
    pub const ALL: [Self; 3] = [Self::Metre, Self::Centimetre, Self::Millimetre];

    /// The suffix the unit is written as.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Metre => "m",
            Self::Centimetre => "cm",
            Self::Millimetre => "mm",
        }
    }

    /// Every unit's symbol, as a sentence lists them: `` `m`, `cm` or `mm` ``.
    pub fn listed() -> String {
        let symbols: Vec<String> = Self::ALL.iter().map(|u| format!("`{u}`")).collect();
        match symbols.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => symbols.concat(),
        }
    }

    fn micrometres(self) -> u64 {
        match self {
            Self::Metre => Length::MICROMETRES_PER_METRE,
            Self::Centimetre => 10_000,
            Self::Millimetre => 1_000,
        }
    }
}

impl fmt::Display for LengthUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

impl std::str::FromStr for LengthUnit {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|unit| unit.symbol() == s)
            .ok_or(())
    }
}

/// One housing half of a mated inline connector pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Male,
    Female,
}

impl Half {
    pub fn as_str(self) -> &'static str {
        match self {
            Half::Male => "male",
            Half::Female => "female",
        }
    }

    /// The one-letter chip drawn on a harness node's header.
    pub fn badge(self) -> &'static str {
        match self {
            Half::Male => "M",
            Half::Female => "F",
        }
    }
}

impl std::str::FromStr for Half {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "male" => Half::Male,
            "female" => Half::Female,
            _ => return Err(()),
        })
    }
}

/// Pair metadata of a synthetic inline-connector instance: the connector
/// type behind each declared housing half. Both halves share the instance's
/// one port set — mated pin N is electrically pin N on either side.
#[derive(Debug, Clone)]
pub struct InlineMeta {
    pub male: Option<InlineHalfMeta>,
    pub female: Option<InlineHalfMeta>,
}

impl InlineMeta {
    pub fn half(&self, h: Half) -> Option<&InlineHalfMeta> {
        match h {
            Half::Male => self.male.as_ref(),
            Half::Female => self.female.as_ref(),
        }
    }
}

/// One housing half's materialized connector-type metadata (mirrors the
/// type-derived fields of [`Connector`]).
#[derive(Debug, Clone)]
pub struct InlineHalfMeta {
    pub type_name: ConnectorTypeName,
    pub description: String,
    /// Free-form connector metadata from the type; carries `part`.
    pub properties: IndexMap<String, ConnectorPropertyValue>,
    /// Harness-side pinout layout, for a future per-half pinout view.
    pub layout: Option<ConnectorLayout>,
}

/// A materialized physical connector on an instance.
#[derive(Debug, Clone)]
pub struct Connector {
    pub name: ConnectorName,
    /// `None` for legacy inline connector blocks; reusable connector
    /// instances carry the top-level connector type name.
    pub type_name: Option<ConnectorTypeName>,
    /// Human-facing description of the physical part, when authored.
    pub description: Option<String>,
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
    pub name: ConnectorName,
    /// Human description of the physical part, when authored (inline
    /// connectors may omit it; typed connectors inherit the type's).
    pub description: Option<String>,
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
    /// The wire's own length, from its body. For a cable conductor it
    /// overrides the cable's; [`Instance::effective_length`] answers the
    /// length that applies.
    pub length: Option<Length>,
    /// Free-text insulation/construction type from the body, e.g. `"FLRY-B"`.
    pub r#type: Option<String>,
    /// Part number from the body.
    pub part: Option<String>,
    /// The authored `wire` statement (inside its cable, for a conductor),
    /// so a check over the drawings can point back at the source.
    pub span: Span,
}

impl Wire {
    /// Whether one of the wire's ends is `splice`.
    pub fn ends_on(&self, splice: &SpliceName) -> bool {
        self.endpoints
            .iter()
            .any(|end| matches!(end, WireEnd::Splice(name) if name == splice))
    }

    /// The end across the wire from `end`, for a two-ended wire (a cable
    /// conductor, or any wire to a splice): where a conductor from a pin
    /// to a splice goes, or which pin a splice's conductor comes from.
    /// `None` when `end` is not one of its two ends.
    pub fn other_end(&self, end: &WireEnd) -> Option<&WireEnd> {
        match self.endpoints.as_slice() {
            [a, b] if a == end => Some(b),
            [a, b] if b == end => Some(a),
            _ => None,
        }
    }
}

/// A resolved wire endpoint. Which an authored bare name is, an own port
/// or a splice, is resolve's to say: `Resolved::wire_end` is the one
/// mapping elaboration and validation share.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WireEnd {
    /// The enclosing component's own port.
    Own(PortName),
    /// A direct child instance's port.
    Child {
        instance: InstanceName,
        port: PortName,
    },
    /// One of the enclosing component's splices.
    Splice(SpliceName),
}

/// The endpoint as authored: `port`, `inst.port` or `splice`.
impl fmt::Display for WireEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Own(port) => write!(f, "{port}"),
            Self::Child { instance, port } => write!(f, "{instance}.{port}"),
            Self::Splice(splice) => write!(f, "{splice}"),
        }
    }
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
    /// The authored `grid:`; `None` leaves the renderer's default.
    pub grid: Option<ViewGrid>,
    /// Where the view's title is written, for a diagnostic about the whole
    /// view.
    pub span: Span,
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

/// A view's authored `grid:` step — drawing units per grid cell — and where
/// its number is written, so a check over the step can point at it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewGrid {
    pub step: f64,
    pub span: Span,
}

/// An annotation box placed at grid coordinates in a schematic view.
#[derive(Debug)]
pub struct TextBox {
    /// The name a box written in the deprecated named form carries, which
    /// the schematic renderer keeps as the box's `data-text`; `None` for the
    /// current, nameless form.
    pub name: Option<String>,
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
    /// The included instance; in a pinout view the subject's connector it
    /// draws, and in a harness view, when `connector` is `None`, the
    /// subject's splice it places (resolve allows nothing else there).
    pub instance: InstanceName,
    /// The connector designator for a harness include; `None` for schematic.
    /// For an inline-connector include this is normalized to the inline's
    /// designator (the authored `male`/`female` segment moves to `half`).
    pub connector: Option<ConnectorName>,
    /// The housing half selected by an inline-connector harness include
    /// (`include ic.female`); `None` for ordinary includes.
    pub half: Option<Half>,
    pub x: f64,
    pub y: f64,
    pub ports: Vec<(PortName, Side)>,
    /// Where the include names its target (`inst` or `inst.connector`), so
    /// a check over the laid-out view can point back at the source.
    pub span: Span,
}

/// An include displays as its target, as authored: `inst`, `inst.connector`,
/// or, for an inline connector, `inst.male` / `inst.female`.
impl fmt::Display for Include {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.half, &self.connector) {
            (Some(half), _) => write!(f, "{}.{}", self.instance, half.as_str()),
            (None, Some(connector)) => write!(f, "{}.{connector}", self.instance),
            (None, None) => write!(f, "{}", self.instance),
        }
    }
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

    fn length(value: f64, unit: &str) -> Length {
        Length::new(value, unit.parse().expect("a unit")).expect("a positive length")
    }

    #[test]
    fn a_shield_termination_holds_at_most_two_ends() {
        let end = |port: &str| WireEnd::Own(PortName::from(port));
        let of = |ports: &[&str]| ports.iter().map(|p| end(p)).collect::<ShieldTermination>();
        assert_eq!(of(&[]), ShieldTermination::NoPin);
        assert_eq!(of(&["a"]), ShieldTermination::Single(end("a")));
        assert_eq!(
            of(&["a", "b"]),
            ShieldTermination::Both([end("a"), end("b")])
        );
        // A third end is validate's to report; the termination keeps two.
        assert_eq!(of(&["a", "b", "c"]).ends(), [end("a"), end("b")]);
        assert!(ShieldTermination::NoPin.ends().is_empty());
    }

    #[test]
    fn lengths_in_every_unit_normalise_to_metres() {
        assert_eq!(length(2.0, "m").metres(), 2.0);
        assert_eq!(length(150.0, "mm").metres(), 0.15);
        assert_eq!(length(15.0, "cm").metres(), 0.15);
        assert_eq!(length(1.1, "cm").metres(), 0.011);
        assert_eq!(length(150.0, "mm"), length(0.15, "m"));
        assert_eq!(length(15.0, "cm"), length(150.0, "mm"));
    }

    #[test]
    fn a_length_displays_its_metres_without_float_noise() {
        for (value, unit, shown) in [
            (2.0, "m", "2 m"),
            (150.0, "mm", "0.15 m"),
            (15.0, "cm", "0.15 m"),
            (1.1, "cm", "0.011 m"),
            (0.1, "m", "0.1 m"),
            (1234.5, "mm", "1.2345 m"),
            (0.001, "mm", "0.000001 m"),
        ] {
            assert_eq!(length(value, unit).to_string(), shown, "{value}{unit}");
        }
    }

    #[test]
    fn a_length_is_positive_to_the_micrometre() {
        for (value, unit) in [(0.0, "m"), (-2.0, "m"), (-150.0, "mm"), (0.0004, "mm")] {
            let unit = unit.parse().expect("a unit");
            assert_eq!(Length::new(value, unit), None, "{value}{unit}");
        }
        assert_eq!(length(0.0005, "mm").to_string(), "0.000001 m");
    }

    #[test]
    fn lengths_order_by_their_size_whatever_their_unit() {
        let mut lengths = [length(1.0, "m"), length(15.0, "cm"), length(200.0, "mm")];
        lengths.sort();
        assert_eq!(lengths.map(|l| l.to_string()), ["0.15 m", "0.2 m", "1 m"]);
    }

    #[test]
    fn length_units_parse_from_their_symbols_only() {
        for unit in LengthUnit::ALL {
            assert_eq!(unit.symbol().parse(), Ok(unit));
            assert_eq!(unit.to_string(), unit.symbol());
        }
        for other in ["in", "km", "M", "MM", "metre", ""] {
            assert_eq!(other.parse::<LengthUnit>(), Err(()), "{other}");
        }
        assert_eq!(LengthUnit::listed(), "`m`, `cm` or `mm`");
    }

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
    fn include_displays_as_its_authored_target() {
        let include = |connector: Option<&str>, half: Option<Half>| Include {
            instance: InstanceName::from("ecu"),
            connector: connector.map(ConnectorName::from),
            half,
            x: 0.0,
            y: 0.0,
            ports: Vec::new(),
            span: Span::new(crate::dsl::span::FileId(0), 0, 0),
        };
        assert_eq!(include(None, None).to_string(), "ecu");
        assert_eq!(include(Some("x1"), None).to_string(), "ecu.x1");
        // An inline include's connector is normalized to the designator;
        // the half is what the author wrote.
        assert_eq!(
            include(Some("ecu"), Some(Half::Female)).to_string(),
            "ecu.female"
        );
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
