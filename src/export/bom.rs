//! The bill of materials: what a design is built from, counted per
//! placement and grouped into lines of one thing to buy each.

use std::fmt;
use std::hash::Hash;

use indexmap::IndexMap;

use super::wires::{Cable, Contact, Copper, End, Row, RowLength, WireList};
use super::{Housing, PinTable, PinTables};
use crate::dsl::ir::{
    Connector, ConnectorTypeName, Design, Half, Instance, PartMeta, Splice, TypeName, WireColor,
};

/// A design's bill of materials: one [`Line`] per thing to buy, with how
/// many of it the design uses.
///
/// Quantities are per placement: a component type placed six times is six
/// components, its connectors six housings each, its cables and wires six
/// times over. Lines come in [`LineKind`] order — housings, cables, loose
/// wire, components, splices — and within a kind in the order the design
/// first uses each, so the order is the source's. An item without a part
/// number is never dropped: it is a line of its own, not
/// [`Line::is_specified`].
///
/// What the model does not hold is not listed: connector contacts,
/// terminals and seals, and a screen (which is part of its cable).
#[derive(Debug, Clone)]
#[must_use]
pub struct Bom<'d> {
    lines: Vec<Line<'d>>,
}

impl<'d> From<&'d Design> for Bom<'d> {
    fn from(design: &'d Design) -> Self {
        let housings = HousingLine::all(&PinTables::from(design));
        let cables = CableLine::all(design);
        let wires = WireLine::all(&WireList::from(design));
        let components = ComponentLine::all(design);
        let splices = SpliceLine::all(design);
        Self {
            lines: housings
                .into_iter()
                .map(Line::Housing)
                .chain(cables.into_iter().map(Line::Cable))
                .chain(wires.into_iter().map(Line::Wire))
                .chain(components.into_iter().map(Line::Component))
                .chain(splices.into_iter().map(Line::Splice))
                .collect(),
        }
    }
}

impl<'d> Bom<'d> {
    /// Every line, in order.
    pub fn lines(&self) -> &[Line<'d>] {
        &self.lines
    }
}

impl<'a, 'd> IntoIterator for &'a Bom<'d> {
    type Item = &'a Line<'d>;
    type IntoIter = std::slice::Iter<'a, Line<'d>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter()
    }
}

/// What a [`Line`] counts, in the order a [`Bom`] lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LineKind {
    /// Connector housings.
    Housing,
    /// Cables.
    Cable,
    /// Loose wire: conductors in no cable.
    Wire,
    /// Components.
    Component,
    /// Splices.
    Splice,
}

impl LineKind {
    /// The kind's name, as the export writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Housing => "housing",
            Self::Cable => "cable",
            Self::Wire => "wire",
            Self::Component => "component",
            Self::Splice => "splice",
        }
    }
}

impl fmt::Display for LineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One line of a [`Bom`]: one thing to buy, and every placement of it.
#[derive(Debug, Clone)]
pub enum Line<'d> {
    /// Connector housings of one part.
    Housing(HousingLine<'d>),
    /// Cables of one type.
    Cable(CableLine<'d>),
    /// Loose wire of one colour, gauge, type and part.
    Wire(WireLine<'d>),
    /// Components of one part.
    Component(ComponentLine<'d>),
    /// Splices of one part.
    Splice(SpliceLine<'d>),
}

impl Line<'_> {
    /// What the line counts.
    pub fn kind(&self) -> LineKind {
        match self {
            Self::Housing(_) => LineKind::Housing,
            Self::Cable(_) => LineKind::Cable,
            Self::Wire(_) => LineKind::Wire,
            Self::Component(_) => LineKind::Component,
            Self::Splice(_) => LineKind::Splice,
        }
    }

    /// How many of the line's item the design uses: housings, cables,
    /// conductors, component placements or splices.
    pub fn quantity(&self) -> usize {
        match self {
            Self::Housing(line) => line.housings.len(),
            Self::Cable(line) => line.cables.len(),
            Self::Wire(line) => line.rows.len(),
            Self::Component(line) => line.instances.len(),
            Self::Splice(line) => line.splices.len(),
        }
    }

    /// Whether the line names what to buy: a part number for a housing,
    /// a loose wire, a component or a splice; a `type` for a cable, whose
    /// body has no part number. An unspecified line is still listed.
    pub fn is_specified(&self) -> bool {
        match self {
            Self::Housing(line) => line.part.is_some(),
            Self::Cable(line) => line.cable_type.is_some(),
            Self::Wire(line) => line.part.is_some(),
            Self::Component(line) => line.part.number.is_some(),
            Self::Splice(line) => line.part.is_some(),
        }
    }

    /// The line's total length and what it leaves out, for the kinds that
    /// are bought by length: a cable's and a loose wire's.
    pub fn length(&self) -> Option<Copper> {
        match self {
            Self::Cable(line) => Some(line.length()),
            Self::Wire(line) => Some(line.length()),
            Self::Housing(_) | Self::Component(_) | Self::Splice(_) => None,
        }
    }
}

/// Connector housings of one part number. Housings with none are grouped
/// by connector type; those with no type either (a connector block that
/// names none, or an inline connector's half it does not declare) by
/// their description; and those with no description as well are one line
/// together.
#[derive(Debug, Clone)]
pub struct HousingLine<'d> {
    /// The housings' part number, from their connector type's `part:`.
    pub part: Option<&'d str>,
    /// Their connector type: the first housing's, on a line of a part.
    /// `None` on a line of housings of no type.
    pub type_name: Option<&'d ConnectorTypeName>,
    /// The housings' description: their connector type's (the first
    /// housing's, on a line of a part); for housings of no type, the
    /// description they are grouped by — a connector block's own, or an
    /// inline connector's label for a half it does not declare.
    pub description: Option<&'d str>,
    /// Every housing the line counts, in pin-table order: one per
    /// connector per placement, and two per inline connector, male first —
    /// a half it does not declare is still a housing to buy.
    pub housings: Vec<HousingPlacement<'d>>,
}

/// What a housing is grouped by: its part, else its type, else its
/// description, else nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HousingKey<'d> {
    Part(&'d str),
    Type(&'d ConnectorTypeName),
    Described(&'d str),
    Neither,
}

/// One housing as the BOM groups it: what it is bought as, and where.
struct HousingItem<'d> {
    housing: Housing<'d>,
    placement: HousingPlacement<'d>,
}

impl<'d> HousingItem<'d> {
    /// The housing `table` is of: its connector's, or its inline half's,
    /// declared or not ([`PinTable::housing`]).
    fn of(table: &PinTable<'d>) -> Self {
        Self {
            housing: table.housing(),
            placement: HousingPlacement {
                instance: table.instance,
                connector: table.connector,
                half: table.half,
            },
        }
    }

    fn key(&self) -> HousingKey<'d> {
        let housing = &self.housing;
        match (housing.part(), housing.type_name, housing.description) {
            (Some(part), _, _) => HousingKey::Part(part),
            (None, Some(type_name), _) => HousingKey::Type(type_name),
            (None, None, Some(description)) => HousingKey::Described(description),
            (None, None, None) => HousingKey::Neither,
        }
    }
}

impl<'d> HousingLine<'d> {
    /// One line per part number (else connector type, else description),
    /// in the order the pin tables first meet it.
    fn all(tables: &PinTables<'d>) -> Vec<Self> {
        let groups: Groups<_, _, _> = tables
            .tables()
            .iter()
            .map(HousingItem::of)
            .map(|item| {
                let line = Self {
                    part: item.housing.part(),
                    type_name: item.housing.type_name,
                    description: item.housing.description,
                    housings: Vec::new(),
                };
                (item.key(), line, item.placement)
            })
            .collect();
        groups
            .into_iter()
            .map(|(line, housings)| Self { housings, ..line })
            .collect()
    }
}

/// One housing: a connector of a placed instance, or one half of an
/// inline connector.
#[derive(Debug, Clone, Copy)]
pub struct HousingPlacement<'d> {
    /// The instance the connector is on; an inline's synthetic instance.
    pub instance: &'d Instance,
    /// The connector.
    pub connector: &'d Connector,
    /// The half of an inline connector the housing is, declared or not;
    /// `None` for an ordinary connector.
    pub half: Option<Half>,
}

/// Where the housing is: `Vehicle.ecu.x1` for a connector,
/// `Vehicle.ic.male` for a half of an inline connector.
impl fmt::Display for HousingPlacement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.instance.path)?;
        if self.instance.inline.is_none() {
            write!(f, ".{}", self.connector.name)?;
        }
        match self.half {
            Some(half) => write!(f, ".{}", half.as_str()),
            None => Ok(()),
        }
    }
}

/// Cables of one `type`; the cables with none are one line together.
#[derive(Debug, Clone)]
pub struct CableLine<'d> {
    /// The cables' `type`, what a cable is bought as.
    pub cable_type: Option<&'d str>,
    /// Every cable the line counts, in IR order of their owners, then in
    /// declaration order.
    pub cables: Vec<CablePlacement<'d>>,
}

impl<'d> CableLine<'d> {
    fn all(design: &'d Design) -> Vec<Self> {
        let groups: Groups<_, _, _> = design
            .instances
            .values()
            .flat_map(|owner| {
                owner.cables.iter().map(move |(name, meta)| {
                    let cable_type = meta.r#type.as_deref();
                    let line = Self {
                        cable_type,
                        cables: Vec::new(),
                    };
                    let placement = CablePlacement {
                        owner,
                        cable: Cable { name, meta },
                    };
                    (cable_type, line, placement)
                })
            })
            .collect();
        groups
            .into_iter()
            .map(|(line, cables)| Self { cables, ..line })
            .collect()
    }

    /// The cables' own lengths added up (not their conductors'), and how
    /// many of them have none.
    pub fn length(&self) -> Copper {
        self.cables
            .iter()
            .map(|placement| {
                placement
                    .cable
                    .meta
                    .length
                    .map_or(RowLength::Unset, RowLength::Own)
            })
            .collect()
    }
}

/// One cable: its owner and the cable.
#[derive(Debug, Clone, Copy)]
pub struct CablePlacement<'d> {
    /// The instance whose body declares the cable.
    pub owner: &'d Instance,
    /// The cable: its designator and metadata.
    pub cable: Cable<'d>,
}

/// The cable's full path: `Vehicle.can`.
impl fmt::Display for CablePlacement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.owner.path, self.cable.name)
    }
}

/// Loose wire of one colour and gauge, and, where the wires set them, one
/// `type` and `part`: two part numbers of one colour and gauge are two
/// lines. A cable's conductors are not loose wire: they come with their
/// cable.
#[derive(Debug, Clone)]
pub struct WireLine<'d> {
    /// The wires' colour.
    pub color: &'d WireColor,
    /// Their cross-section, in mm².
    pub gauge: f64,
    /// The wires' insulation type, from their `type:`.
    pub wire_type: Option<&'d str>,
    /// The wires' part number, from their `part:`.
    pub part: Option<&'d str>,
    /// Every conductor the line counts, as the wire list's rows: one per
    /// segment of a rail.
    pub rows: Vec<Row<'d>>,
}

/// What loose wire is grouped by; the gauge by its bits, since a gauge is
/// a positive number and equal gauges have equal bits.
type WireKey<'d> = (&'d WireColor, u64, Option<&'d str>, Option<&'d str>);

impl<'d> WireLine<'d> {
    fn all(list: &WireList<'d>) -> Vec<Self> {
        let groups: Groups<WireKey<'d>, _, _> = list
            .conductors()
            .filter(|(row, _)| row.cable.is_none())
            .map(|(row, conductor)| {
                let wire = conductor.wire;
                let (wire_type, part) = (wire.r#type.as_deref(), wire.part.as_deref());
                let line = Self {
                    color: &wire.color,
                    gauge: wire.gauge,
                    wire_type,
                    part,
                    rows: Vec::new(),
                };
                let key = (&wire.color, wire.gauge.to_bits(), wire_type, part);
                (key, line, *row)
            })
            .collect();
        groups
            .into_iter()
            .map(|(line, rows)| Self { rows, ..line })
            .collect()
    }

    /// The conductors' copper added up, a rail's length once (on its first
    /// segment, [`RowLength::copper`]), and how many have no length.
    pub fn length(&self) -> Copper {
        self.rows.iter().map(|row| row.length).collect()
    }

    /// How many of the conductors are boundary continuations: they end on
    /// their owner's own port that is on no connector, so they continue a
    /// conductor the enclosing component wires to that port rather than
    /// being a second piece of wire. They are counted in the quantity all
    /// the same; a boundary port on the owner's own connector is a real
    /// termination and does not make one.
    pub fn boundary(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| {
                row.ends().iter().any(|end| {
                    matches!(end, End::Boundary(port) if port.contact() == Contact::Connectorless)
                })
            })
            .count()
    }
}

/// Components of one part number. A component type without one is a line
/// of its own, its part data (manufacturer, description, rating) still
/// carried. The root, the design itself, is not listed, and neither are
/// inline connectors (their halves are housings) nor assemblies without a
/// part number ([`Instance::is_assembly`]), whose parts are listed.
#[derive(Debug, Clone)]
pub struct ComponentLine<'d> {
    /// The part data of the first component the line counts: the part
    /// number is the line's, and a type without one is a line alone.
    pub part: &'d PartMeta,
    /// The first component's type.
    pub type_name: &'d TypeName,
    /// Every placement the line counts, in IR order.
    pub instances: Vec<&'d Instance>,
}

/// What a component is grouped by: its part number, else its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ComponentKey<'d> {
    Part(&'d str),
    Type(&'d TypeName),
}

impl<'d> ComponentLine<'d> {
    fn all(design: &'d Design) -> Vec<Self> {
        let groups: Groups<_, _, _> = design
            .instances
            .values()
            .filter(|instance| {
                instance.path != design.root
                    && instance.inline.is_none()
                    && (instance.part.number.is_some() || !instance.is_assembly())
            })
            .map(|instance| {
                let key = match &instance.part.number {
                    Some(number) => ComponentKey::Part(number),
                    None => ComponentKey::Type(&instance.type_name),
                };
                let line = Self {
                    part: &instance.part,
                    type_name: &instance.type_name,
                    instances: Vec::new(),
                };
                (key, line, instance)
            })
            .collect();
        groups
            .into_iter()
            .map(|(line, instances)| Self { instances, ..line })
            .collect()
    }
}

/// Splices of one part number. Splices without one are grouped by their
/// label (their name, when they have none).
#[derive(Debug, Clone)]
pub struct SpliceLine<'d> {
    /// The splices' part number, from their `part:`.
    pub part: Option<&'d str>,
    /// On a line without a part number: the label its splices share, or
    /// the name of those without a label. `None` on a line of a part.
    pub label: Option<&'d str>,
    /// The first splice's description.
    pub description: Option<&'d str>,
    /// Every splice the line counts, in IR order of their owners, then in
    /// declaration order.
    pub splices: Vec<SplicePlacement<'d>>,
}

/// What a splice is grouped by: its part, else its label or name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SpliceKey<'d> {
    Part(&'d str),
    Named(&'d str),
}

impl<'d> SpliceLine<'d> {
    fn all(design: &'d Design) -> Vec<Self> {
        let groups: Groups<_, _, _> = design
            .instances
            .values()
            .flat_map(|owner| {
                owner.splices.values().map(move |splice| {
                    let key = match &splice.part {
                        Some(part) => SpliceKey::Part(part),
                        None => SpliceKey::Named(
                            splice.label.as_deref().unwrap_or(splice.name.as_str()),
                        ),
                    };
                    let line = Self {
                        part: splice.part.as_deref(),
                        label: match key {
                            SpliceKey::Named(label) => Some(label),
                            SpliceKey::Part(_) => None,
                        },
                        description: splice.description.as_deref(),
                        splices: Vec::new(),
                    };
                    (key, line, SplicePlacement { owner, splice })
                })
            })
            .collect();
        groups
            .into_iter()
            .map(|(line, splices)| Self { splices, ..line })
            .collect()
    }
}

/// One splice: its owner and the splice.
#[derive(Debug, Clone, Copy)]
pub struct SplicePlacement<'d> {
    /// The instance whose body declares the splice.
    pub owner: &'d Instance,
    /// The splice: its name, label, part and description.
    pub splice: &'d Splice,
}

/// The splice's full path: `Vehicle.sp_gnd`.
impl fmt::Display for SplicePlacement<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.owner.path, self.splice.name)
    }
}

/// Items grouped by key, in the order each key is first met: for each
/// key the line its first item starts and what the line keeps of every
/// item of it, in order. Collected from `(key, line, kept)` triples.
struct Groups<K, L, P>(IndexMap<K, (L, Vec<P>)>);

impl<K: Hash + Eq, L, P> FromIterator<(K, L, P)> for Groups<K, L, P> {
    fn from_iter<I: IntoIterator<Item = (K, L, P)>>(items: I) -> Self {
        let mut groups: IndexMap<K, (L, Vec<P>)> = IndexMap::new();
        for (key, line, kept) in items {
            groups
                .entry(key)
                .or_insert_with(|| (line, Vec::new()))
                .1
                .push(kept);
        }
        Self(groups)
    }
}

impl<K, L, P> IntoIterator for Groups<K, L, P> {
    type Item = (L, Vec<P>);
    type IntoIter = indexmap::map::IntoValues<K, (L, Vec<P>)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{Length, LengthUnit};
    use crate::export::records::tests::design;

    const LIB: &str = r#"
connector_type Dt04_2p "Deutsch DT04-2P" { part: "DT04-2P"; }
connector_type Dt06_2s "Deutsch DT06-2S" { part: "DT06-2S"; }
connector_type Hdr2 "Header 2p" { }

component Lamp {
    part: "LMP-1";
    description: "Marker lamp";
    connector x1: Dt04_2p { pub port a "A" pin 1; pub port b "B" pin 2; }
}

component Node {
    manufacturer: "Generic";
    connector x1: Hdr2 { pub port a "A" pin 1; pub port b "B" pin 2; }
}

component Plug {
    connector x1 "Plug 2p" { pub port a "A" pin 1; pub port b "B" pin 2; }
}

component Socket {
    connector x1 "Socket 2p" { pub port a "A" pin 1; pub port b "B" pin 2; }
}

component Bare {
    connector x1 { pub port a "A" pin 1; }
}

component Bar {
    left: Lamp; mid: Lamp; right: Lamp;
    wire red 1 [left.a, mid.a, right.a] { length: 2m; }
}

component Pod {
    pub port feed "FEED";
    connector x9 "Pod 1p" { pub port out "OUT" pin 1; }
    lamp: Lamp;
    wire red 1 [feed, lamp.a];
    wire black 0.5 [out, lamp.b];
}
"#;

    const USES: &str = "use Lamp from \"lib.wb\";\nuse Node from \"lib.wb\";\n\
         use Plug from \"lib.wb\";\nuse Bar from \"lib.wb\";\n\
         use Socket from \"lib.wb\";\nuse Bare from \"lib.wb\";\nuse Pod from \"lib.wb\";\n\
         use Dt04_2p from \"lib.wb\";\nuse Dt06_2s from \"lib.wb\";\n";

    fn bom_of(main: &str) -> Design {
        design(LIB, &format!("{USES}{main}"))
    }

    fn metres(value: f64) -> Length {
        Length::new(value, LengthUnit::Metre).expect("a positive length")
    }

    fn paths<T: ToString>(placements: &[T]) -> Vec<String> {
        placements.iter().map(ToString::to_string).collect()
    }

    fn housings<'b, 'd>(bom: &'b Bom<'d>) -> Vec<&'b HousingLine<'d>> {
        bom.lines()
            .iter()
            .filter_map(|line| match line {
                Line::Housing(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    fn cables<'b, 'd>(bom: &'b Bom<'d>) -> Vec<&'b CableLine<'d>> {
        bom.lines()
            .iter()
            .filter_map(|line| match line {
                Line::Cable(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    fn wires<'b, 'd>(bom: &'b Bom<'d>) -> Vec<&'b WireLine<'d>> {
        bom.lines()
            .iter()
            .filter_map(|line| match line {
                Line::Wire(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    fn components<'b, 'd>(bom: &'b Bom<'d>) -> Vec<&'b ComponentLine<'d>> {
        bom.lines()
            .iter()
            .filter_map(|line| match line {
                Line::Component(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    fn splices<'b, 'd>(bom: &'b Bom<'d>) -> Vec<&'b SpliceLine<'d>> {
        bom.lines()
            .iter()
            .filter_map(|line| match line {
                Line::Splice(line) => Some(line),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_type_placed_six_times_counts_six_with_its_housings() {
        let design = bom_of("component Car { front: Bar; rear: Bar; }\n");
        let bom = Bom::from(&design);
        let [lamps] = components(&bom)[..] else {
            panic!("one component line: the bars are assemblies");
        };
        assert_eq!(lamps.part.number.as_deref(), Some("LMP-1"));
        assert_eq!(lamps.type_name.as_str(), "Lamp");
        assert_eq!(lamps.instances.len(), 6);
        let [housing] = housings(&bom)[..] else {
            panic!("one housing line");
        };
        assert_eq!(housing.part, Some("DT04-2P"));
        assert_eq!(housing.housings.len(), 6);
        assert_eq!(
            paths(&housing.housings)[..2],
            ["Car.front.left.x1", "Car.front.mid.x1"]
        );
    }

    #[test]
    fn an_inline_contributes_each_of_its_halves_as_a_housing() {
        let design = bom_of(
            "component Car { lamp: Lamp; node: Node;\n\
               inline ic \"Branch\" { male: Dt04_2p; female: Dt06_2s; port sig \"SIG\" pin 1; }\n\
               wire green 0.5 [lamp.a, ic.sig]; wire green 0.5 [ic.sig, node.a]; }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = housings(&bom)
            .iter()
            .map(|line| (line.part, line.housings.len(), paths(&line.housings)))
            .collect();
        assert_eq!(
            lines,
            [
                (
                    Some("DT04-2P"),
                    2,
                    vec!["Car.lamp.x1".to_string(), "Car.ic.male".to_string()]
                ),
                (None, 1, vec!["Car.node.x1".to_string()]),
                (Some("DT06-2S"), 1, vec!["Car.ic.female".to_string()]),
            ]
        );
        // The inline is a connector, not a component to buy.
        let types: Vec<&str> = components(&bom)
            .iter()
            .map(|line| line.type_name.as_str())
            .collect();
        assert_eq!(types, ["Lamp", "Node"]);
    }

    #[test]
    fn a_housing_without_a_part_is_grouped_by_its_type_else_by_its_description() {
        let design = bom_of(
            "component Car { a: Node; p: Plug; s: Socket; b: Node; q: Plug; x: Bare; y: Bare; }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = housings(&bom)
            .iter()
            .map(|line| {
                (
                    line.part,
                    line.type_name.map(ConnectorTypeName::as_str),
                    line.description,
                    line.housings.len(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                (None, Some("Hdr2"), Some("Header 2p"), 2),
                // Connector blocks that name no type, by their description…
                (None, None, Some("Plug 2p"), 2),
                (None, None, Some("Socket 2p"), 1),
                // …and those with none either, together.
                (None, None, None, 2),
            ]
        );
        assert!(
            bom.lines()
                .iter()
                .filter(|line| line.kind() == LineKind::Housing)
                .all(|line| !line.is_specified())
        );
    }

    #[test]
    fn cables_of_one_type_sum_their_own_lengths_and_count_the_unmeasured() {
        let design = bom_of(
            "component Car { a: Node; b: Node; c: Node;\n\
             cable k1 { type: \"2-core\"; length: 3m;\n\
               wire red 1 [a.a, b.a] { length: 10m; } wire black 1 [a.b, b.b]; shield; }\n\
             cable k2 { type: \"2-core\"; wire red 1 [b.a, c.a]; wire black 1 [b.b, c.b]; }\n\
             cable k3 { length: 50cm; wire red 1 [a.a, c.a]; } }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = cables(&bom)
            .iter()
            .map(|line| (line.cable_type, paths(&line.cables), line.length()))
            .collect();
        assert_eq!(
            lines,
            [
                (
                    Some("2-core"),
                    vec!["Car.k1".to_string(), "Car.k2".to_string()],
                    // The cable's 3 m, not its conductors' 10 m and 3 m.
                    Copper {
                        length: Some(metres(3.0)),
                        unmeasured: 1
                    }
                ),
                (
                    None,
                    vec!["Car.k3".to_string()],
                    Copper {
                        length: Some(metres(0.5)),
                        unmeasured: 0
                    }
                ),
            ]
        );
        let specified: Vec<bool> = bom
            .lines()
            .iter()
            .filter(|line| line.kind() == LineKind::Cable)
            .map(Line::is_specified)
            .collect();
        assert_eq!(specified, [true, false]);
        // A cable's conductors are not loose wire, and its screen is part
        // of it: neither is a line.
        assert!(wires(&bom).is_empty());
    }

    #[test]
    fn loose_wire_of_one_colour_and_gauge_sums_a_rail_once_and_counts_the_unmeasured() {
        let design = bom_of(
            "component Car { a: Node; b: Node; c: Node; bar: Bar;\n\
             wire red 1 [a.a, b.a] { length: 40cm; }\n\
             wire red 1 [b.a, c.a];\n\
             wire red 0.5 [a.b, b.b] { length: 1m; } }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = wires(&bom)
            .iter()
            .map(|line| {
                (
                    line.color.code(),
                    line.gauge,
                    line.rows.len(),
                    line.length(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                // Two own conductors and the bar's rail of two segments,
                // whose 2 m is the whole rail's, counted once.
                (
                    "RD".to_string(),
                    1.0,
                    4,
                    Copper {
                        length: Some(metres(2.4)),
                        unmeasured: 1
                    }
                ),
                (
                    "RD".to_string(),
                    0.5,
                    1,
                    Copper {
                        length: Some(metres(1.0)),
                        unmeasured: 0
                    }
                ),
            ]
        );
    }

    #[test]
    fn two_wire_part_numbers_of_one_colour_and_gauge_are_two_lines() {
        let design = bom_of(
            "component Car { a: Node; b: Node;\n\
             wire black 0.5 [a.a, b.a] { part: \"W-1\"; }\n\
             wire black 0.5 [a.b, b.b] { part: \"W-2\"; }\n\
             wire black 0.5 [a.a, b.b] { part: \"W-1\"; type: \"FLRY-B\"; }\n\
             wire black 0.5 [a.b, b.a] { part: \"W-1\"; } }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = wires(&bom)
            .iter()
            .map(|line| (line.part, line.wire_type, line.rows.len()))
            .collect();
        assert_eq!(
            lines,
            [
                (Some("W-1"), None, 2),
                (Some("W-2"), None, 1),
                (Some("W-1"), Some("FLRY-B"), 1)
            ]
        );
    }

    #[test]
    fn a_component_without_a_part_number_is_a_line_of_its_type_and_assemblies_are_not() {
        let design = bom_of("component Car { a: Node; b: Node; p: Plug; bar: Bar; pod: Pod; }\n");
        let bom = Bom::from(&design);
        let lines: Vec<_> = components(&bom)
            .iter()
            .map(|line| {
                (
                    line.type_name.as_str(),
                    line.part.number.as_deref(),
                    line.part.manufacturer.as_deref(),
                    paths(&line.instances.iter().map(|i| &i.path).collect::<Vec<_>>()),
                )
            })
            .collect();
        let strings = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        // Neither the root nor the bar, which only places lamps, is listed;
        // the pod has a connector of its own, so it may be a part.
        assert_eq!(
            lines,
            [
                ("Node", None, Some("Generic"), strings(&["Car.a", "Car.b"])),
                ("Plug", None, None, strings(&["Car.p"])),
                (
                    "Lamp",
                    Some("LMP-1"),
                    None,
                    strings(&[
                        "Car.bar.left",
                        "Car.bar.mid",
                        "Car.bar.right",
                        "Car.pod.lamp"
                    ])
                ),
                ("Pod", None, None, strings(&["Car.pod"])),
            ]
        );
        let specified: Vec<bool> = bom
            .lines()
            .iter()
            .filter(|line| line.kind() == LineKind::Component)
            .map(Line::is_specified)
            .collect();
        assert_eq!(specified, [false, false, true, false]);
    }

    #[test]
    fn splices_are_grouped_by_part_else_by_label_or_name() {
        let design = bom_of(
            "component Car { a: Node; b: Node;\n\
             splice s1 \"Ground\" { part: \"D-436\"; description: \"Solder sleeve\"; }\n\
             splice s2 { part: \"D-436\"; }\n\
             splice s3 \"Feed\"; splice s4 \"Feed\"; splice s5;\n\
             wire black 1 [a.a, s1]; wire black 1 [s1, s2]; wire black 1 [s2, b.a];\n\
             wire red 1 [a.b, s3]; wire red 1 [s3, s4]; wire red 1 [s4, s5]; wire red 1 [s5, b.b]; }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = splices(&bom)
            .iter()
            .map(|line| {
                (
                    line.part,
                    line.label,
                    line.description,
                    paths(&line.splices),
                )
            })
            .collect();
        let strings = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(
            lines,
            [
                (
                    Some("D-436"),
                    None,
                    Some("Solder sleeve"),
                    strings(&["Car.s1", "Car.s2"])
                ),
                (None, Some("Feed"), None, strings(&["Car.s3", "Car.s4"])),
                (None, Some("s5"), None, strings(&["Car.s5"])),
            ]
        );
    }

    #[test]
    fn lines_come_by_kind_then_in_order_of_first_use() {
        let design = bom_of(
            "component Car { node: Node; lamp: Lamp; splice sp;\n\
             cable k { type: \"1-core\"; wire red 1 [node.a, sp]; }\n\
             wire red 1 [sp, lamp.a]; wire black 1 [node.b, lamp.b]; }\n",
        );
        let bom = Bom::from(&design);
        let kinds: Vec<LineKind> = bom.lines().iter().map(Line::kind).collect();
        use LineKind::*;
        assert_eq!(
            kinds,
            [
                Housing, Housing, Cable, Wire, Wire, Component, Component, Splice
            ]
        );
        assert!(kinds.is_sorted());
        let quantities: Vec<usize> = bom.lines().iter().map(Line::quantity).collect();
        assert_eq!(quantities, [1, 1, 1, 1, 1, 1, 1, 1]);
        // Only what is bought by length has a length.
        let measured: Vec<bool> = bom.lines().iter().map(|l| l.length().is_some()).collect();
        assert_eq!(
            measured,
            [false, false, true, true, true, false, false, false]
        );
    }

    #[test]
    fn an_inline_is_two_housings_even_when_it_declares_a_half_or_none() {
        let design = bom_of(
            "component Car { a: Node; b: Node; c: Node;\n\
               inline one \"One half\" { female: Dt06_2s; port s \"S\" pin 1; }\n\
               inline none \"No halves\" { port s \"S\" pin 1; }\n\
               wire green 0.5 [a.a, one.s]; wire green 0.5 [one.s, b.a];\n\
               wire green 0.5 [b.b, none.s]; wire green 0.5 [none.s, c.a]; }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = housings(&bom)
            .iter()
            .map(|line| (line.part, line.description, paths(&line.housings)))
            .collect();
        let strings = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        assert_eq!(
            lines,
            [
                (
                    None,
                    Some("Header 2p"),
                    strings(&["Car.a.x1", "Car.b.x1", "Car.c.x1"])
                ),
                // The undeclared half of each inline is listed under its
                // label, male first; the declared one under its part.
                (None, Some("One half"), strings(&["Car.one.male"])),
                (
                    Some("DT06-2S"),
                    Some("Deutsch DT06-2S"),
                    strings(&["Car.one.female"])
                ),
                (
                    None,
                    Some("No halves"),
                    strings(&["Car.none.male", "Car.none.female"])
                ),
            ]
        );
        // Each housing is one pin table: the BOM and the pin tables agree.
        assert_eq!(PinTables::from(&design).tables().len(), 7);
    }

    #[test]
    fn a_wire_line_counts_the_conductors_that_continue_at_a_connectorless_boundary() {
        // The pod's `feed` is on no connector: its wire continues the one
        // the car wires to `feed`. Its `x9` port is a real termination.
        let design = bom_of(
            "component Car { a: Node; b: Node; pod: Pod;\n\
             wire red 1 [a.a, pod.feed];\n\
             wire red 1 [b.a, pod.out]; }\n",
        );
        let bom = Bom::from(&design);
        let lines: Vec<_> = wires(&bom)
            .iter()
            .map(|line| (line.color.code(), line.rows.len(), line.boundary()))
            .collect();
        assert_eq!(lines, [("RD".to_string(), 3, 1), ("BK".to_string(), 1, 0)]);
    }
}
