//! The per-connector pin tables: for each connector of each instance, one
//! row per cavity with everything crimped into it — what is read while
//! crimping a harness.

use indexmap::IndexMap;

use super::WireList;
use super::wires::{Cable, Conductor, Contact, End, PortEnd, RowKind, ShieldRun, SpliceEnd};
use crate::dsl::ir::{
    Connector, ConnectorLayout, ConnectorName, ConnectorPropertyValue, ConnectorTypeName, Design,
    Half, HarnessScope, Instance, InstancePath, Pin, Port, Wire, WireEnd,
};

/// A design's pin tables: one [`PinTable`] per connector of each instance,
/// built from its [`WireList`], so every conductor and drain of the list
/// that lands on a pin is in the table of that pin's connector.
///
/// Tables are per placement, like the wire list: a component type placed
/// twice yields its tables twice, each with its own instance path.
/// Connectorless ports have no table. Order is stable and follows the
/// source: instances in IR order, the root among them, then each
/// instance's connectors in declaration order — an inline connector's
/// halves male first — and within a table its cavities by pin, then its
/// ports without a pin in source order.
#[derive(Debug, Clone)]
#[must_use]
pub struct PinTables<'d> {
    tables: Vec<PinTable<'d>>,
}

impl<'d> From<&'d Design> for PinTables<'d> {
    fn from(design: &'d Design) -> Self {
        let mut tables: Vec<PinTable<'d>> = design
            .instances
            .values()
            .flat_map(PinTable::all_of)
            .collect();
        let mut by_connector: IndexMap<(&InstancePath, &ConnectorName), Vec<usize>> =
            IndexMap::new();
        for (i, table) in tables.iter().enumerate() {
            by_connector
                .entry((&table.instance.path, &table.connector.name))
                .or_default()
                .push(i);
        }
        for landing in Landing::all(design, &WireList::from(design)) {
            let Contact::Connector { designator, .. } = landing.at.contact() else {
                continue;
            };
            let Some(indices) = by_connector.get(&(&landing.at.instance.path, designator)) else {
                continue;
            };
            for &i in indices {
                tables[i].add(&landing);
            }
        }
        Self { tables }
    }
}

impl<'d> PinTables<'d> {
    /// Every table, in order.
    pub fn tables(&self) -> &[PinTable<'d>] {
        &self.tables
    }
}

impl<'a, 'd> IntoIterator for &'a PinTables<'d> {
    type Item = &'a PinTable<'d>;
    type IntoIter = std::slice::Iter<'a, PinTable<'d>>;

    fn into_iter(self) -> Self::IntoIter {
        self.tables.iter()
    }
}

/// One connector's pin table: one [`Cavity`] per pin, sorted by pin
/// ([`Pin`]'s order, numeric), and the connector's ports that have no pin.
#[derive(Debug, Clone)]
pub struct PinTable<'d> {
    /// The instance the connector is on; its `path` is the full one.
    pub instance: &'d Instance,
    /// The connector: designator, description, type and layout. An inline
    /// connector's is the one its synthetic instance carries, named after
    /// it; its housings are its halves' ([`PinTable::housing`]).
    pub connector: &'d Connector,
    /// For an inline connector, the housing half this table is of: each
    /// half it declares has a table of its own. `None` for an ordinary
    /// connector, and for an inline that declares no half (one table).
    pub half: Option<Half>,
    pub cavities: Vec<Cavity<'d>>,
    /// The connector's ports that are assigned no pin (the pin-out is not
    /// known yet), in source order, with what is crimped into them: a
    /// conductor to one of them is listed here rather than dropped.
    pub unpinned: Vec<UnpinnedPort<'d>>,
}

impl<'d> PinTable<'d> {
    /// The empty tables of `instance`'s connectors: one per connector, or
    /// one per declared half of an inline connector.
    fn all_of(instance: &'d Instance) -> impl Iterator<Item = Self> {
        instance.connectors.values().flat_map(move |connector| {
            Self::halves(instance).into_iter().map(move |half| {
                let layout = match (half, &instance.inline) {
                    (Some(half), Some(inline)) => {
                        inline.half(half).and_then(|meta| meta.layout.as_ref())
                    }
                    _ => connector.layout.as_ref(),
                };
                Self {
                    instance,
                    connector,
                    half,
                    cavities: Cavity::all_of(instance, connector, layout),
                    unpinned: UnpinnedPort::all_of(instance, connector),
                }
            })
        })
    }

    /// The housing this table is of: an inline connector's half's (its
    /// connector type's), else the connector's own.
    pub fn housing(&self) -> Housing<'d> {
        let half = self
            .half
            .zip(self.instance.inline.as_ref())
            .and_then(|(half, inline)| inline.half(half));
        match half {
            Some(half) => Housing {
                type_name: Some(&half.type_name),
                description: Some(&half.description),
                properties: &half.properties,
            },
            None => Housing {
                type_name: self.connector.type_name.as_ref(),
                description: self.connector.description.as_deref(),
                properties: &self.connector.properties,
            },
        }
    }

    /// The halves `instance`'s connector has a table for: each half an
    /// inline connector declares, else the one connector (`None`).
    fn halves(instance: &Instance) -> Vec<Option<Half>> {
        let declared: Vec<Option<Half>> = instance
            .inline
            .iter()
            .flat_map(|inline| {
                [Half::Male, Half::Female]
                    .into_iter()
                    .filter(|&half| inline.half(half).is_some())
            })
            .map(Some)
            .collect();
        if declared.is_empty() {
            vec![None]
        } else {
            declared
        }
    }

    /// Whether this table is one of two halves of an inline connector,
    /// whose conductors the views may or may not place in a half.
    fn is_one_of_two_halves(&self) -> bool {
        self.instance
            .inline
            .as_ref()
            .is_some_and(|inline| inline.male.is_some() && inline.female.is_some())
    }

    /// List `landing` in the cavities of its port, when it belongs in this
    /// table's half.
    fn add(&mut self, landing: &Landing<'d>) {
        let half = match (self.half, self.is_one_of_two_halves()) {
            (Some(half), true) => match landing.half {
                Some(drawn) if drawn == half => HalfFit::Assigned,
                Some(_) => return,
                None => HalfFit::Undetermined,
            },
            _ => HalfFit::Assigned,
        };
        let port = &landing.at.port.name;
        for cavity in &mut self.cavities {
            if cavity.port.is_some_and(|p| &p.name == port) {
                cavity.entries.push(landing.entry.with_half(half));
            }
        }
        for unpinned in &mut self.unpinned {
            if &unpinned.port.name == port {
                unpinned.entries.push(landing.entry.with_half(half));
            }
        }
    }
}

/// The housing a [`PinTable`] is of, as it is bought: its connector type
/// and that type's metadata. A connector block in a component that names no
/// type has no `type_name`, and only the metadata it writes itself.
#[derive(Debug, Clone, Copy)]
pub struct Housing<'d> {
    /// The connector type, `None` for a connector block without one.
    pub type_name: Option<&'d ConnectorTypeName>,
    /// The housing's description: its type's, or the connector block's.
    pub description: Option<&'d str>,
    /// The type's free-form metadata, `part` among it.
    pub properties: &'d IndexMap<String, ConnectorPropertyValue>,
}

impl<'d> Housing<'d> {
    /// The housing's part number, from its `part:`.
    pub fn part(&self) -> Option<&'d str> {
        match self.properties.get("part") {
            Some(ConnectorPropertyValue::Str(part)) => Some(part),
            _ => None,
        }
    }
}

/// A port of a connector that is assigned no pin, and what is crimped into
/// it: a cavity whose pin is not known yet.
#[derive(Debug, Clone)]
pub struct UnpinnedPort<'d> {
    /// The port, with its name and label; its `pins` are empty.
    pub port: &'d Port,
    /// The conductors and drains landing on the port, in wire-list order.
    pub entries: Vec<CavityEntry<'d>>,
}

impl<'d> UnpinnedPort<'d> {
    /// `connector`'s ports on `instance` that have no pin, in source order.
    fn all_of(instance: &'d Instance, connector: &'d Connector) -> Vec<Self> {
        instance
            .ports
            .values()
            .filter(|port| {
                port.pins.is_empty()
                    && port
                        .connector
                        .as_ref()
                        .is_some_and(|c| c.name == connector.name)
            })
            .map(|port| Self {
                port,
                entries: Vec::new(),
            })
            .collect()
    }
}

/// One cavity of a connector: its pin, the port it carries, and what is
/// crimped into it.
#[derive(Debug, Clone)]
pub struct Cavity<'d> {
    pub pin: Pin,
    /// The port bound to the pin, with its name and label; `None` for a
    /// cavity of the connector's layout that no port uses.
    pub port: Option<&'d Port>,
    /// The conductors and drains crimped into the cavity, in wire-list
    /// order: none for an unused pin, two for a double crimp. A ganged
    /// port's ([`Cavity::is_ganged`]) are listed in each of its cavities,
    /// since the model does not say which cavity takes which.
    pub entries: Vec<CavityEntry<'d>>,
}

impl<'d> Cavity<'d> {
    /// The cavities of `connector`, on `instance`, sorted by pin: one per
    /// pin a port is bound to, and one per cavity of `layout` no port uses.
    fn all_of(
        instance: &'d Instance,
        connector: &'d Connector,
        layout: Option<&'d ConnectorLayout>,
    ) -> Vec<Self> {
        let mut cavities: Vec<Self> = connector
            .pins
            .iter()
            .map(|binding| Self {
                pin: binding.pin,
                port: instance.ports.get(&binding.port),
                entries: Vec::new(),
            })
            .collect();
        let layout_pins: Vec<Pin> = match layout {
            Some(ConnectorLayout::Grid(grid)) => (1..=grid.rows * grid.cols).map(Pin).collect(),
            Some(ConnectorLayout::Face(face)) => face.cavities.iter().map(|c| c.pin).collect(),
            None => Vec::new(),
        };
        for pin in layout_pins {
            if !cavities.iter().any(|cavity| cavity.pin == pin) {
                cavities.push(Self {
                    pin,
                    port: None,
                    entries: Vec::new(),
                });
            }
        }
        cavities.sort_by_key(|cavity| cavity.pin);
        cavities
    }

    /// Whether the cavity's port occupies other cavities too (`pins [2,
    /// 3]`): its entries are then the port's, listed in each of them, and
    /// must not be counted once per cavity.
    pub fn is_ganged(&self) -> bool {
        self.port.is_some_and(|port| port.pins.len() > 1)
    }

    /// The other cavities the cavity's port occupies, in the order the port
    /// lists them; none unless it is ganged.
    pub fn ganged_with(&self) -> impl Iterator<Item = Pin> + '_ {
        self.port
            .into_iter()
            .flat_map(|port| port.pins.iter().copied())
            .filter(move |&pin| pin != self.pin)
    }
}

/// One thing crimped into a cavity: a conductor, or a cable screen's drain
/// (which has no colour or gauge of its own).
#[derive(Debug, Clone)]
pub enum CavityEntry<'d> {
    Conductor(CavityConductor<'d>),
    Drain(CavityDrain<'d>),
}

impl CavityEntry<'_> {
    /// Whether the entry is known to be in its table's housing half.
    pub fn half(&self) -> HalfFit {
        match self {
            Self::Conductor(conductor) => conductor.half,
            Self::Drain(drain) => drain.half,
        }
    }

    fn with_half(&self, half: HalfFit) -> Self {
        let mut entry = self.clone();
        match &mut entry {
            Self::Conductor(conductor) => conductor.half = half,
            Self::Drain(drain) => drain.half = half,
        }
        entry
    }
}

/// A conductor in a cavity, with where it goes. Its colour (name and IEC
/// 60757 code), gauge, label and twisted-pair number are its wire's fields
/// (`conductor.wire`).
#[derive(Debug, Clone)]
pub struct CavityConductor<'d> {
    /// The instance whose body declares the conductor.
    pub owner: &'d Instance,
    /// The cable it runs in; `None` for a loose wire.
    pub cable: Option<Cable<'d>>,
    /// The conductor, as the wire list has it.
    pub conductor: Conductor<'d>,
    /// The end across the conductor from this cavity.
    pub far: FarEnd<'d>,
    pub half: HalfFit,
}

/// A screen's drain terminated in a cavity.
#[derive(Debug, Clone)]
pub struct CavityDrain<'d> {
    /// The instance whose body declares the cable.
    pub owner: &'d Instance,
    /// The cable the screen is of.
    pub cable: Cable<'d>,
    /// The screen: what it screens, and all the pins its drain lands on.
    pub shield: ShieldRun<'d>,
    /// The drain's other end, for a screen grounded at both ends.
    pub far: Option<End<'d>>,
    pub half: HalfFit,
}

/// Whether an entry is known to be crimped into the housing half its
/// table is of. Outside the tables of an inline connector with two halves
/// it is always [`HalfFit::Assigned`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfFit {
    /// The housing is known: an ordinary connector, an inline that declares
    /// one half, or a half the harness views of the conductor's component
    /// draw it into, and no view draws it into the other.
    Assigned,
    /// The harness views draw the conductor into neither half, or into
    /// both: it is listed under each half, flagged, rather than guessed.
    Undetermined,
}

/// Where a conductor goes from a cavity: the end across it.
#[derive(Debug, Clone)]
pub enum FarEnd<'d> {
    /// A port of a component placed in the conductor's owner, whose
    /// [`PortEnd::contact`] says which connector pin, or that it is on none.
    Port(PortEnd<'d>),
    /// The owner's own port: where the conductor leaves its component, to
    /// be carried on by a wire of the one it is placed in.
    Boundary(PortEnd<'d>),
    /// A splice, with what it joins beyond it.
    Splice(FarSplice<'d>),
    /// An end the design does not resolve; a checked design has none.
    Unresolved(&'d WireEnd),
}

impl<'d> FarEnd<'d> {
    /// `end`, the far end of `wire`, a conductor of `design`.
    fn of(design: &'d Design, wire: &'d Wire, end: End<'d>) -> Self {
        match end {
            End::Port(port) => Self::Port(port),
            End::Boundary(port) => Self::Boundary(port),
            End::Unresolved(end) => Self::Unresolved(end),
            End::Splice(splice) => Self::Splice(FarSplice::of(design, wire, splice)),
        }
    }
}

/// A splice a conductor runs to, and every other conductor end its net
/// joins: through linked splices, in wire order, the conductor's own end
/// left out.
#[derive(Debug, Clone)]
pub struct FarSplice<'d> {
    pub splice: SpliceEnd<'d>,
    pub beyond: Vec<SplicedEnd<'d>>,
}

impl<'d> FarSplice<'d> {
    /// `splice`, reached by `wire`.
    fn of(design: &'d Design, wire: &'d Wire, splice: SpliceEnd<'d>) -> Self {
        let beyond = splice
            .owner
            .splice_net(&splice.splice.name)
            .ends
            .into_iter()
            .filter(|net| !std::ptr::eq(net.wire, wire))
            .map(|net| SplicedEnd {
                end: End::of(design, splice.owner, net.end),
                wire: net.wire,
            })
            .collect();
        Self { splice, beyond }
    }
}

/// One end a splice's net joins, and the conductor bringing it in.
#[derive(Debug, Clone, Copy)]
pub struct SplicedEnd<'d> {
    pub end: End<'d>,
    pub wire: &'d Wire,
}

/// One conductor or drain end landing on a port, as a table entry: the
/// port, the entry, and the half the harness views draw it into when the
/// port is an inline connector's with two halves.
struct Landing<'d> {
    at: PortEnd<'d>,
    entry: CavityEntry<'d>,
    half: Option<Half>,
}

impl<'d> Landing<'d> {
    /// Every end of `list` that lands on a port, in list order: each
    /// conductor's two ends, then each drain end.
    fn all(design: &'d Design, list: &WireList<'d>) -> Vec<Self> {
        let mut landings = Vec::new();
        for row in list {
            match &row.kind {
                RowKind::Conductor(conductor) => {
                    for (i, end) in conductor.ends.iter().enumerate() {
                        let Some(&at) = end.port() else { continue };
                        let ends = conductor.segment.ends;
                        landings.push(Self {
                            at,
                            half: at.drawn_half(design, row.owner, |scope| scope.draws(ends)),
                            entry: CavityEntry::Conductor(CavityConductor {
                                owner: row.owner,
                                cable: row.cable,
                                conductor: *conductor,
                                far: FarEnd::of(design, conductor.wire, conductor.ends[1 - i]),
                                half: HalfFit::Assigned,
                            }),
                        });
                    }
                }
                RowKind::Shield(shield) => {
                    let Some(cable) = row.cable else { continue };
                    let ends = shield.drain.ends();
                    for (i, (end, written)) in ends.iter().zip(shield.shield.ends()).enumerate() {
                        let Some(&at) = end.port() else { continue };
                        let drawn = |scope: &HarnessScope<'d>| {
                            scope.draws_drain(cable.name, shield.screened, written)
                        };
                        landings.push(Self {
                            at,
                            half: at.drawn_half(design, row.owner, drawn),
                            entry: CavityEntry::Drain(CavityDrain {
                                owner: row.owner,
                                cable,
                                shield: *shield,
                                far: ends.get(1 - i).copied(),
                                half: HalfFit::Assigned,
                            }),
                        });
                    }
                }
            }
        }
        landings
    }
}

impl<'d> PortEnd<'d> {
    /// The housing half the harness views of `owner`'s component draw this
    /// end into, when it is on an inline connector and they agree: each
    /// view that `drawn` says draws it includes the inline through one
    /// half, and the answer is that half when there is exactly one. `None`
    /// when no view draws it, when views draw it into different halves, and
    /// for an end on an ordinary connector.
    fn drawn_half(
        &self,
        design: &'d Design,
        owner: &'d Instance,
        drawn: impl Fn(&HarnessScope<'d>) -> bool,
    ) -> Option<Half> {
        self.inline()?;
        let inline = self.instance.path.segments().last()?;
        let mut halves: Vec<Half> = Vec::new();
        for view in &design.views {
            if !view.kind.is_harness() || view.subject != owner.type_name {
                continue;
            }
            let scope = HarnessScope::new(design, owner, view);
            if let Some(half) = scope.half(inline)
                && drawn(&scope)
                && !halves.contains(&half)
            {
                halves.push(half);
            }
        }
        match halves[..] {
            [half] => Some(half),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::InstanceName;

    /// A checked design of `main` (its one top-level component is the root)
    /// beside `lib.wb`, which holds the parts it `use`s. Panics on an error;
    /// warnings (a conductor no harness view draws) are expected.
    fn design(main: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in [
            (
                "wirebug.toml",
                "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
            ),
            ("main.wb", main),
            ("lib.wb", LIB),
        ] {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let report = crate::check_project(Some(dir.path()));
        let errors: Vec<_> = report.problems.iter().filter(|p| p.is_error()).collect();
        assert!(errors.is_empty(), "the source checks: {errors:?}");
        report.design.expect("a design")
    }

    const LIB: &str = r#"
connector_type Quad "Quad 4p" {
    layout grid { rows: 1; cols: 4; }
}

component Ecu {
    connector x1 "ECU 10p" {
        pub port shld "SHLD" pin 10;
        pub port gnd  "GND"  pins [2, 3];
        pub port pwr  "PWR"  pin 1;
        pub port can  "CAN"  pin 4;
    }
    connector x2: Quad {
        pub port aux "AUX" pin 3;
        pub port en  "EN"  pin 1;
    }
    pub port sense "SENSE";
}

component Dev {
    connector x1 "X" {
        pub port a "A";
        pub port b "B" pin 2;
        pub port c "C";
    }
}

component Loom {
    connector x9 "Loom 1p" {
        pub port out "OUT" pin 1;
    }
    lamp: Ecu "Lamp";
    wire black 0.5 "OUT" [out, lamp.pwr];
}
"#;

    const USES: &str = "use Ecu from \"lib.wb\";\nuse Loom from \"lib.wb\";\n";

    fn path(dotted: &str) -> InstancePath {
        let mut segments = dotted.split('.').map(InstanceName::from);
        let root = InstancePath::root(segments.next().expect("a root"));
        segments.fold(root, |path, segment| path.child(segment))
    }

    /// The table of `connector` on `instance` (and `half`, for an inline).
    fn table<'t, 'd>(
        tables: &'t PinTables<'d>,
        instance: &str,
        connector: &str,
        half: Option<Half>,
    ) -> &'t PinTable<'d> {
        tables
            .tables()
            .iter()
            .find(|t| {
                t.instance.path == path(instance)
                    && t.connector.name.as_str() == connector
                    && t.half == half
            })
            .unwrap_or_else(|| panic!("a table of {instance}.{connector} ({half:?})"))
    }

    fn cavity<'t, 'd>(table: &'t PinTable<'d>, pin: u32) -> &'t Cavity<'d> {
        table
            .cavities
            .iter()
            .find(|c| c.pin == Pin(pin))
            .unwrap_or_else(|| panic!("cavity {pin}"))
    }

    fn conductors<'c, 'd>(cavity: &'c Cavity<'d>) -> Vec<&'c CavityConductor<'d>> {
        cavity
            .entries
            .iter()
            .filter_map(|entry| match entry {
                CavityEntry::Conductor(conductor) => Some(conductor),
                CavityEntry::Drain(_) => None,
            })
            .collect()
    }

    /// A far end as `kind:path`, for comparing.
    fn far(end: &FarEnd<'_>) -> String {
        match end {
            FarEnd::Port(port) => format!("port:{}.{}", port.instance.path, port.port.name),
            FarEnd::Boundary(port) => {
                format!("boundary:{}.{}", port.instance.path, port.port.name)
            }
            FarEnd::Splice(splice) => format!(
                "splice:{}.{}",
                splice.splice.owner.path, splice.splice.splice.name
            ),
            FarEnd::Unresolved(end) => format!("unresolved:{end}"),
        }
    }

    fn fars(cavity: &Cavity<'_>) -> Vec<String> {
        conductors(cavity).iter().map(|c| far(&c.far)).collect()
    }

    #[test]
    fn cavities_are_sorted_by_pin_and_unused_ones_are_listed_empty() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; node: Ecu; wire red 1 [ecu.pwr, node.pwr]; }}\n"
        ));
        let tables = PinTables::from(&design);
        let x1 = table(&tables, "Car.ecu", "x1", None);
        let rows: Vec<(u32, Option<&str>, usize)> = x1
            .cavities
            .iter()
            .map(|c| (c.pin.0, c.port.map(|p| p.label.as_str()), c.entries.len()))
            .collect();
        assert_eq!(
            rows,
            [
                (1, Some("PWR"), 1),
                (2, Some("GND"), 0),
                (3, Some("GND"), 0),
                (4, Some("CAN"), 0),
                (10, Some("SHLD"), 0),
            ]
        );
        // A typed connector's layout lists the cavities no port uses too.
        let x2 = table(&tables, "Car.ecu", "x2", None);
        let rows: Vec<(u32, Option<&str>)> = x2
            .cavities
            .iter()
            .map(|c| (c.pin.0, c.port.map(|p| p.name.as_str())))
            .collect();
        assert_eq!(
            rows,
            [(1, Some("en")), (2, None), (3, Some("aux")), (4, None)]
        );
    }

    #[test]
    fn every_connector_of_every_instance_has_a_table_in_ir_order() {
        let design = design(&format!(
            "{USES}component Car {{\n\
               connector j1 \"Car 1p\" {{ port feed \"FEED\" pin 1; }}\n\
               port free \"FREE\";\n\
               front: Loom; rear: Loom;\n\
             }}\n"
        ));
        let tables = PinTables::from(&design);
        let names: Vec<String> = tables
            .into_iter()
            .map(|t| format!("{}.{}", t.instance.path, t.connector.name))
            .collect();
        assert_eq!(
            names,
            [
                "Car.j1",
                "Car.front.x9",
                "Car.front.lamp.x1",
                "Car.front.lamp.x2",
                "Car.rear.x9",
                "Car.rear.lamp.x1",
                "Car.rear.lamp.x2",
            ]
        );
        // Each placement's table holds its own placement's conductor.
        for loom in ["Car.front", "Car.rear"] {
            let out = cavity(table(&tables, loom, "x9", None), 1);
            assert_eq!(fars(out), [format!("port:{loom}.lamp.pwr")]);
            let pwr = cavity(table(&tables, &format!("{loom}.lamp"), "x1", None), 1);
            assert_eq!(fars(pwr), [format!("boundary:{loom}.out")]);
        }
    }

    #[test]
    fn a_double_crimp_lists_both_conductors_in_wire_order() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; a: Ecu; b: Ecu;\n\
             wire black 1   \"PWR A\" [ecu.pwr, a.pwr];\n\
             cable c \"C\" {{ twisted {{\n\
               wire brown 0.5 \"PWR B\" [ecu.pwr, b.pwr];\n\
               wire white 0.5 [ecu.can, b.can];\n\
             }} }} }}\n"
        ));
        let tables = PinTables::from(&design);
        let pwr = cavity(table(&tables, "Car.ecu", "x1", None), 1);
        let entries: Vec<_> = conductors(pwr)
            .iter()
            .map(|c| {
                let wire = c.conductor.wire;
                (
                    wire.color.code(),
                    wire.gauge,
                    wire.label.as_deref(),
                    c.cable.map(|cable| cable.name.as_str()),
                    wire.twisted_group,
                    far(&c.far),
                    c.half,
                )
            })
            .collect();
        assert_eq!(
            entries,
            [
                (
                    "BK".to_string(),
                    1.0,
                    Some("PWR A"),
                    None,
                    None,
                    "port:Car.a.pwr".to_string(),
                    HalfFit::Assigned,
                ),
                (
                    "BN".to_string(),
                    0.5,
                    Some("PWR B"),
                    Some("c"),
                    Some(0),
                    "port:Car.b.pwr".to_string(),
                    HalfFit::Assigned,
                ),
            ]
        );
        // The far side lists it from its end.
        let b = cavity(table(&tables, "Car.b", "x1", None), 1);
        assert_eq!(fars(b), ["port:Car.ecu.pwr"]);
    }

    #[test]
    fn a_ganged_port_is_listed_in_each_of_its_cavities_marked_as_ganged() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; bat: Ecu; wire black 2.5 [ecu.gnd, bat.gnd]; }}\n"
        ));
        let tables = PinTables::from(&design);
        let x1 = table(&tables, "Car.ecu", "x1", None);
        for (pin, other) in [(2, 3), (3, 2)] {
            let gnd = cavity(x1, pin);
            assert!(gnd.is_ganged());
            assert_eq!(gnd.ganged_with().collect::<Vec<_>>(), [Pin(other)]);
            assert_eq!(fars(gnd), ["port:Car.bat.gnd"]);
        }
        let pwr = cavity(x1, 1);
        assert!(!pwr.is_ganged());
        assert_eq!(pwr.ganged_with().count(), 0);
    }

    #[test]
    fn a_drain_on_a_pin_is_an_entry_of_its_own_with_its_other_end() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; node: Ecu; lamp: Ecu;\n\
             cable can \"CAN\" {{\n\
               wire yellow 0.5 [ecu.can, node.can];\n\
               shield \"SHLD\" [ecu.shld, node.shld];\n\
             }}\n\
             cable other \"Other\" {{\n\
               wire green 0.5 [ecu.aux, lamp.aux];\n\
               shield [lamp.shld];\n\
             }} }}\n"
        ));
        let tables = PinTables::from(&design);
        let drains = |instance: &str| -> Vec<(String, Option<String>)> {
            let shld = cavity(table(&tables, instance, "x1", None), 10);
            assert!(conductors(shld).is_empty(), "a drain is not a conductor");
            shld.entries
                .iter()
                .filter_map(|entry| match entry {
                    CavityEntry::Drain(drain) => Some((
                        format!(
                            "{}:{}",
                            drain.cable.name,
                            drain.shield.shield.label.as_deref().unwrap_or("-")
                        ),
                        drain.far.map(|end| end.to_string()),
                    )),
                    CavityEntry::Conductor(_) => None,
                })
                .collect()
        };
        assert_eq!(
            drains("Car.ecu"),
            [("can:SHLD".to_string(), Some("Car.node.shld".to_string()))]
        );
        assert_eq!(
            drains("Car.node"),
            [("can:SHLD".to_string(), Some("Car.ecu.shld".to_string()))]
        );
        assert_eq!(drains("Car.lamp"), [("other:-".to_string(), None)]);
    }

    #[test]
    fn a_conductor_to_a_splice_names_the_splice_and_what_its_net_joins() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; a: Ecu; bat: Ecu;\n\
             splice sp \"GND\"; splice sp2;\n\
             wire black 1   [ecu.gnd, sp];\n\
             wire black 0.5 [a.gnd, sp];\n\
             wire black 2.5 [sp, sp2];\n\
             wire black 2.5 [sp2, bat.gnd];\n\
             wire black 0.5 [ecu.sense, sp2]; }}\n"
        ));
        let tables = PinTables::from(&design);
        let gnd = cavity(table(&tables, "Car.ecu", "x1", None), 2);
        let [conductor] = conductors(gnd)[..] else {
            panic!("one conductor");
        };
        let FarEnd::Splice(splice) = &conductor.far else {
            panic!("a splice far end");
        };
        assert_eq!(splice.splice.splice.name.as_str(), "sp");
        assert_eq!(splice.splice.splice.label.as_deref(), Some("GND"));
        let beyond: Vec<(String, f64)> = splice
            .beyond
            .iter()
            .map(|b| (b.end.to_string(), b.wire.gauge))
            .collect();
        // Its own end is left out; the linked splice's ends, a
        // connectorless one among them, are in.
        assert_eq!(
            beyond,
            [
                ("Car.a.gnd".to_string(), 0.5),
                ("Car.bat.gnd".to_string(), 2.5),
                ("Car.ecu.sense".to_string(), 0.5),
            ]
        );
    }

    #[test]
    fn a_boundary_port_on_the_owners_connector_lists_the_inside_and_the_outside() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; loom: Loom; wire red 1 [ecu.pwr, loom.out]; }}\n"
        ));
        let tables = PinTables::from(&design);
        let out = cavity(table(&tables, "Car.loom", "x9", None), 1);
        let entries: Vec<(String, String)> = conductors(out)
            .iter()
            .map(|c| (c.owner.path.to_string(), far(&c.far)))
            .collect();
        assert_eq!(
            entries,
            [
                ("Car".to_string(), "port:Car.ecu.pwr".to_string()),
                ("Car.loom".to_string(), "port:Car.loom.lamp.pwr".to_string()),
            ]
        );
    }

    const INLINE: &str = "connector_type M2 \"M 3p\" { layout grid { rows: 1; cols: 3; } }\n\
         connector_type F2 \"F 3p\" { }\n";

    #[test]
    fn an_inline_has_a_table_per_half_and_the_views_place_its_conductors() {
        let design = design(&format!(
            "{USES}{INLINE}component Car {{ ecu: Ecu; pedal: Ecu; lamp: Ecu; bcm: Ecu;\n\
               inline ic \"Branch\" {{ male: M2; female: F2;\n\
                 port sig \"SIG\" pin 1; port gnd \"GND\" pin 2; port drn \"DRN\" pin 3; }}\n\
               wire green 0.5 \"ECU SIG\"   [ecu.pwr, ic.sig];\n\
               cable can \"CAN\" {{ wire yellow 0.5 [ecu.can, bcm.can]; shield [ic.drn]; }}\n\
               wire green 0.5 \"PEDAL SIG\" [ic.sig, pedal.pwr];\n\
               wire black 0.5 \"LAMP GND\"  [ic.gnd, lamp.pwr];\n\
               wire black 0.5 \"BOTH GND\"  [ic.gnd, bcm.pwr];\n\
             }}\n\
             view harness \"ECU side\" {{ include ecu.x1 at (0, 0); include ic.female at (10, 0);\n\
               include bcm.x1 at (20, 0); }}\n\
             view harness \"Pedal side\" {{ include ic.male at (0, 0); include pedal.x1 at (10, 0); }}\n\
             view harness \"BCM\" {{ include ic.male at (0, 0); include bcm.x1 at (10, 0); }}\n"
        ));
        let tables = PinTables::from(&design);
        let halves: Vec<Option<Half>> = tables
            .into_iter()
            .filter(|t| t.instance.path == path("Car.ic"))
            .map(|t| t.half)
            .collect();
        assert_eq!(halves, [Some(Half::Male), Some(Half::Female)]);

        let listed = |half: Half, pin: u32| -> Vec<(Option<&str>, HalfFit)> {
            conductors(cavity(table(&tables, "Car.ic", "ic", Some(half)), pin))
                .iter()
                .map(|c| (c.conductor.wire.label.as_deref(), c.half))
                .collect()
        };
        // A view decides: each loom's conductor is in the half its view
        // includes.
        assert_eq!(
            listed(Half::Female, 1),
            [(Some("ECU SIG"), HalfFit::Assigned)]
        );
        assert_eq!(
            listed(Half::Male, 1),
            [(Some("PEDAL SIG"), HalfFit::Assigned)]
        );
        // No view draws the lamp's, and two views draw the BCM's, one into
        // each half: both are listed under each half, flagged.
        let undecided = [
            (Some("LAMP GND"), HalfFit::Undetermined),
            (Some("BOTH GND"), HalfFit::Undetermined),
        ];
        assert_eq!(listed(Half::Male, 2), undecided);
        assert_eq!(listed(Half::Female, 2), undecided);
        // Each half's table takes its own housing's layout.
        let pins = |half| -> Vec<u32> {
            table(&tables, "Car.ic", "ic", Some(half))
                .cavities
                .iter()
                .map(|c| c.pin.0)
                .collect()
        };
        assert_eq!(pins(Half::Male), [1, 2, 3]);
        assert_eq!(pins(Half::Female), [1, 2, 3]);
        // A drain is placed like a conductor: the main view draws its cable
        // and includes the female half; the BCM view does not draw the
        // cable.
        let drains = |half| -> Vec<HalfFit> {
            cavity(table(&tables, "Car.ic", "ic", Some(half)), 3)
                .entries
                .iter()
                .map(CavityEntry::half)
                .collect()
        };
        assert_eq!(drains(Half::Female), [HalfFit::Assigned]);
        assert_eq!(drains(Half::Male), []);
    }

    #[test]
    fn a_conductor_into_a_port_without_a_pin_is_listed_under_the_unpinned_ports() {
        let design = design(
            "use Dev from \"lib.wb\";\n\
             component Car { ecu: Dev; lamp: Dev; wire red 0.5 [ecu.a, lamp.a]; }\n",
        );
        let tables = PinTables::from(&design);
        for (instance, other) in [("Car.ecu", "Car.lamp"), ("Car.lamp", "Car.ecu")] {
            let x1 = table(&tables, instance, "x1", None);
            let cavities: Vec<(u32, usize)> = x1
                .cavities
                .iter()
                .map(|c| (c.pin.0, c.entries.len()))
                .collect();
            assert_eq!(cavities, [(2, 0)]);
            let unpinned: Vec<(&str, Vec<String>)> = x1
                .unpinned
                .iter()
                .map(|u| {
                    let fars = u
                        .entries
                        .iter()
                        .map(|entry| match entry {
                            CavityEntry::Conductor(c) => far(&c.far),
                            CavityEntry::Drain(_) => "drain".to_string(),
                        })
                        .collect();
                    (u.port.label.as_str(), fars)
                })
                .collect();
            // Every pinless port is listed, in source order, used or not.
            assert_eq!(
                unpinned,
                [("A", vec![format!("port:{other}.a")]), ("C", Vec::new()),]
            );
        }
    }

    #[test]
    fn a_tables_housing_is_its_connector_type_or_its_inline_halfs() {
        let design = design(&format!(
            "{USES}{INLINE}connector_type Box \"Box 2p\" {{ part: \"BX-2\"; }}\n\
             component Car {{ ecu: Ecu; pedal: Ecu;\n\
               connector j1: Box {{ port a \"A\" pin 1; }}\n\
               inline ic \"Branch\" {{ male: M2; female: F2; port sig \"SIG\" pin 1; }}\n\
               wire green 0.5 [ecu.pwr, ic.sig];\n\
               wire green 0.5 [ic.sig, pedal.pwr];\n\
             }}\n"
        ));
        let tables = PinTables::from(&design);
        let housing = |instance: &str, connector: &str, half| {
            let housing = table(&tables, instance, connector, half).housing();
            (
                housing.type_name.map(|t| t.as_str().to_string()),
                housing.description.map(str::to_string),
                housing.part().map(str::to_string),
            )
        };
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            housing("Car", "j1", None),
            (some("Box"), some("Box 2p"), some("BX-2"))
        );
        assert_eq!(
            housing("Car.ecu", "x1", None),
            (None, some("ECU 10p"), None)
        );
        assert_eq!(
            housing("Car.ic", "ic", Some(Half::Male)),
            (some("M2"), some("M 3p"), None)
        );
        assert_eq!(
            housing("Car.ic", "ic", Some(Half::Female)),
            (some("F2"), some("F 3p"), None)
        );
    }

    #[test]
    fn an_inline_with_one_half_has_one_table_with_all_its_conductors() {
        let design = design(&format!(
            "{USES}{INLINE}component Car {{ ecu: Ecu; pedal: Ecu;\n\
               inline ic \"Branch\" {{ female: F2; port sig \"SIG\" pin 1; }}\n\
               wire green 0.5 [ecu.pwr, ic.sig];\n\
               wire green 0.5 [ic.sig, pedal.pwr];\n\
             }}\n"
        ));
        let tables = PinTables::from(&design);
        let ic: Vec<&PinTable> = tables
            .into_iter()
            .filter(|t| t.instance.path == path("Car.ic"))
            .collect();
        let [only] = ic[..] else {
            panic!("one table");
        };
        assert_eq!(only.half, Some(Half::Female));
        let sig = cavity(only, 1);
        assert_eq!(fars(sig), ["port:Car.ecu.pwr", "port:Car.pedal.pwr"]);
        assert!(conductors(sig).iter().all(|c| c.half == HalfFit::Assigned));
    }
}
