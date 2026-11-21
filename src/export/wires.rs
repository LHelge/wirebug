//! The wire list: every physical conductor of a design, one row each, plus
//! one row per cable screen, in source order.

use std::fmt;

use crate::dsl::ir::{
    CableMeta, CableName, ConnectorName, Design, InlineMeta, Instance, Length, Pin, Port, Screened,
    Shield, ShieldTermination, Splice, Wire, WireEnd, WireSegment,
};

/// A design's wire list (cut list): one [`Row`] per physical conductor —
/// each segment of each wire, a multi-endpoint rail chain-decomposed by
/// [`Wire::segments`] exactly as the drawings decompose it — and one per
/// cable screen ([`RowKind::Shield`]).
///
/// Rows cover every instance, the root and each nested one, and carry the
/// instance that declares them ([`Row::owner`]). Rows are per placement: a
/// component type placed twice yields its rows twice, each with its own
/// owner path. Order is stable and follows the source: instances in IR
/// order, then within an instance its wires in declaration order (each
/// wire's segments in endpoint order), then the screens of its cables in
/// cable order (each cable's overall screen before its pairs').
#[derive(Debug, Clone)]
#[must_use]
pub struct WireList<'d> {
    rows: Vec<Row<'d>>,
}

impl<'d> From<&'d Design> for WireList<'d> {
    fn from(design: &'d Design) -> Self {
        Self {
            rows: design
                .instances
                .values()
                .flat_map(|owner| Row::all_of(design, owner))
                .collect(),
        }
    }
}

impl<'d> WireList<'d> {
    /// Every row, in list order.
    pub fn rows(&self) -> &[Row<'d>] {
        &self.rows
    }

    /// The conductor rows, in list order, each with its conductor.
    pub fn conductors(&self) -> impl Iterator<Item = (&Row<'d>, &Conductor<'d>)> {
        self.rows.iter().filter_map(|row| match &row.kind {
            RowKind::Conductor(conductor) => Some((row, conductor)),
            RowKind::Shield(_) => None,
        })
    }

    /// The screen rows, in list order, each with its screen.
    pub fn shields(&self) -> impl Iterator<Item = (&Row<'d>, &ShieldRun<'d>)> {
        self.rows.iter().filter_map(|row| match &row.kind {
            RowKind::Shield(shield) => Some((row, shield)),
            RowKind::Conductor(_) => None,
        })
    }
}

impl<'a, 'd> IntoIterator for &'a WireList<'d> {
    type Item = &'a Row<'d>;
    type IntoIter = std::slice::Iter<'a, Row<'d>>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}

/// One line of the wire list: a conductor or a screen, with what the two
/// share.
#[derive(Debug, Clone, Copy)]
pub struct Row<'d> {
    /// The instance whose component body declares the conductor or screen.
    /// Its ends are named relative to it (`Conductor::segment`), and each
    /// [`End`] also knows the full path of what it lands on.
    pub owner: &'d Instance,
    /// The cable the row runs in; `None` for a loose wire.
    pub cable: Option<Cable<'d>>,
    /// The length that applies: a conductor's own, else its cable's
    /// ([`Instance::effective_length`]); a screen's is its cable's. Each
    /// segment of a rail carries its wire's length. `None` when unset.
    pub length: Option<Length>,
    /// A conductor or a screen, with what is particular to it.
    pub kind: RowKind<'d>,
}

impl<'d> Row<'d> {
    /// The rows `owner` declares: its conductors, then its cables' screens.
    fn all_of(design: &'d Design, owner: &'d Instance) -> impl Iterator<Item = Self> {
        let conductors = owner.wires.iter().flat_map(move |wire| {
            let cable = wire.cable.as_ref().and_then(|name| Cable::of(owner, name));
            let length = owner.effective_length(wire);
            wire.segments().map(move |segment| Self {
                owner,
                cable,
                length,
                kind: RowKind::Conductor(Conductor {
                    wire,
                    segment,
                    ends: segment.ends.map(|end| End::of(design, owner, end)),
                }),
            })
        });
        let shields = owner.cables.iter().flat_map(move |(name, meta)| {
            meta.shields().map(move |(screened, shield)| Self {
                owner,
                cable: Some(Cable { name, meta }),
                length: meta.length,
                kind: RowKind::Shield(ShieldRun {
                    screened,
                    shield,
                    drain: Drain::of(design, owner, &shield.termination),
                }),
            })
        });
        conductors.chain(shields)
    }

    /// Where the row lands: a conductor's two ends, or the pins a screen's
    /// drain is terminated on (none, one or two).
    pub fn ends(&self) -> &[End<'d>] {
        match &self.kind {
            RowKind::Conductor(conductor) => &conductor.ends,
            RowKind::Shield(shield) => shield.drain.ends(),
        }
    }
}

/// The cable a row runs in: its designator and its metadata.
#[derive(Debug, Clone, Copy)]
pub struct Cable<'d> {
    /// The cable's designator.
    pub name: &'d CableName,
    /// Its label, `type`, length and screens.
    pub meta: &'d CableMeta,
}

impl<'d> Cable<'d> {
    /// `owner`'s cable `name`; `None` when it declares none of that name.
    fn of(owner: &'d Instance, name: &CableName) -> Option<Self> {
        owner
            .cables
            .get_key_value(name)
            .map(|(name, meta)| Self { name, meta })
    }
}

/// What a [`Row`] is: a conductor to cut, or a cable screen.
#[derive(Debug, Clone, Copy)]
pub enum RowKind<'d> {
    Conductor(Conductor<'d>),
    /// A screen is not a conductor (no colour, no gauge), but it is cut
    /// with its cable and its drain lands on pins, so it has a row.
    Shield(ShieldRun<'d>),
}

/// One physical conductor: one segment of a wire. Its colour (name and
/// IEC 60757 code, [`WireColor`](crate::dsl::ir::WireColor)), gauge, label,
/// `type`, `part` and twisted-pair number are its wire's fields.
#[derive(Debug, Clone, Copy)]
pub struct Conductor<'d> {
    /// The wire the conductor is a segment of. Two conductors in one
    /// cavity (a double crimp) are two wires, so two rows.
    pub wire: &'d Wire,
    /// Which segment of the wire it is, with its ends as the owner's body
    /// names them.
    pub segment: WireSegment<'d>,
    /// The segment's two ends, resolved, in the order the wire lists them.
    pub ends: [End<'d>; 2],
}

/// One cable screen: what it screens and where its drain lands.
#[derive(Debug, Clone, Copy)]
pub struct ShieldRun<'d> {
    /// The cable's overall screen, or one twisted pair's own.
    pub screened: Screened,
    pub shield: &'d Shield,
    pub drain: Drain<'d>,
}

/// The pins a screen's drain is terminated on, resolved: the shape of
/// [`ShieldTermination`].
#[derive(Debug, Clone, Copy)]
pub enum Drain<'d> {
    /// On no pin: cut back, or bonded at a gland or housing.
    NoPin,
    Single(End<'d>),
    Both([End<'d>; 2]),
}

impl<'d> Drain<'d> {
    fn of(design: &'d Design, owner: &'d Instance, termination: &'d ShieldTermination) -> Self {
        let end = |end| End::of(design, owner, end);
        match termination {
            ShieldTermination::NoPin => Self::NoPin,
            ShieldTermination::Single(only) => Self::Single(end(only)),
            ShieldTermination::Both(ends) => Self::Both(ends.each_ref().map(end)),
        }
    }

    /// The drain's ends, in source order.
    pub fn ends(&self) -> &[End<'d>] {
        match self {
            Self::NoPin => &[],
            Self::Single(end) => std::slice::from_ref(end),
            Self::Both(ends) => ends,
        }
    }
}

/// Where one end of a conductor, or of a drain, lands.
#[derive(Debug, Clone, Copy)]
pub enum End<'d> {
    /// A port of a component placed in the owner — a child instance, or an
    /// inline connector (whose instance carries `inline`; the IR does not
    /// say which housing half a conductor is crimped into).
    Port(PortEnd<'d>),
    /// The owner's own port: the end where the conductor leaves the owning
    /// component, carried on by a wire of the component it is placed in.
    Boundary(PortEnd<'d>),
    /// One of the owner's splices.
    Splice(SpliceEnd<'d>),
    /// An end the design does not resolve, as written. Only a design with
    /// errors has one; a checked design never does.
    Unresolved(&'d WireEnd),
}

impl<'d> End<'d> {
    /// `end`, written in `owner`'s body, resolved against the design.
    pub(super) fn of(design: &'d Design, owner: &'d Instance, end: &'d WireEnd) -> Self {
        let resolved = match end {
            WireEnd::Own(port) => owner.ports.get(port).map(|port| {
                Self::Boundary(PortEnd {
                    instance: owner,
                    port,
                })
            }),
            WireEnd::Child { instance, port } => owner
                .children
                .get(instance)
                .and_then(|path| design.get(path))
                .and_then(|child| {
                    let port = child.ports.get(port)?;
                    Some(Self::Port(PortEnd {
                        instance: child,
                        port,
                    }))
                }),
            WireEnd::Splice(name) => owner
                .splices
                .get(name)
                .map(|splice| Self::Splice(SpliceEnd { owner, splice })),
        };
        resolved.unwrap_or(Self::Unresolved(end))
    }

    /// The port the end lands on, for an end on a port (a placed
    /// component's or the owner's own).
    pub fn port(&self) -> Option<&PortEnd<'d>> {
        match self {
            Self::Port(port) | Self::Boundary(port) => Some(port),
            Self::Splice(_) | Self::Unresolved(_) => None,
        }
    }
}

/// The full path of what the end lands on: `Vehicle.ecu.pwr` for a port,
/// `Vehicle.sp_gnd` for a splice; an unresolved end as written.
impl fmt::Display for End<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Port(end) | Self::Boundary(end) => {
                write!(f, "{}.{}", end.instance.path, end.port.name)
            }
            Self::Splice(end) => write!(f, "{}.{}", end.owner.path, end.splice.name),
            Self::Unresolved(end) => write!(f, "{end}"),
        }
    }
}

/// An end on a port: the instance it is on (whose `path` is the full one)
/// and the port, with its name and label.
#[derive(Debug, Clone, Copy)]
pub struct PortEnd<'d> {
    pub instance: &'d Instance,
    pub port: &'d Port,
}

impl<'d> PortEnd<'d> {
    /// The connector cavity the end is crimped into, if the port is on a
    /// connector.
    pub fn contact(&self) -> Contact<'d> {
        match &self.port.connector {
            Some(connector) => Contact::Connector {
                designator: &connector.name,
                pins: &self.port.pins,
            },
            None => Contact::Connectorless,
        }
    }

    /// The inline connector's halves, when the end is on one.
    pub fn inline(&self) -> Option<&'d InlineMeta> {
        self.instance.inline.as_ref()
    }
}

/// Where on its instance a [`PortEnd`] is terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact<'d> {
    /// A pin of a connector: its designator, and the pins the port
    /// occupies — several for ganged cavities (`pins [2, 3]`), none when
    /// the connector assigns it no pin. An inline connector's designator is
    /// its name.
    Connector {
        designator: &'d ConnectorName,
        pins: &'d [Pin],
    },
    /// The port is on no connector: the row is kept, with no connector or
    /// pin to name.
    Connectorless,
}

/// An end on a splice: the instance that declares it, and the splice.
#[derive(Debug, Clone, Copy)]
pub struct SpliceEnd<'d> {
    pub owner: &'d Instance,
    pub splice: &'d Splice,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{InstanceName, InstancePath, LengthUnit, PortName};

    /// A checked design of `main` (its one top-level component is the root)
    /// beside `lib.wb`, which holds the parts it `use`s. Panics on an error.
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
component Ecu {
    connector x1 "ECU 6p" {
        pub port pwr   "PWR"   pin 1;
        pub port gnd   "GND"   pins [2, 3];
        pub port can_h "CAN H" pin 4;
        pub port can_l "CAN L" pin 5;
        pub port shld  "SHLD"  pin 6;
    }
    pub port sense "SENSE";
}

component Loom {
    pub port feed "FEED";
    connector x9 "Loom 1p" {
        pub port out "OUT" pin 1;
    }
    lamp: Ecu "Lamp";
    wire red   0.5 "FEED" [feed, lamp.pwr];
    wire black 0.5 "OUT"  [out, lamp.gnd];
}
"#;

    const USES: &str = "use Ecu from \"lib.wb\";\nuse Loom from \"lib.wb\";\n";

    fn path(dotted: &str) -> InstancePath {
        let mut segments = dotted.split('.').map(InstanceName::from);
        let root = InstancePath::root(segments.next().expect("a root"));
        segments.fold(root, |path, segment| path.child(segment))
    }

    fn length(value: f64, unit: LengthUnit) -> Option<Length> {
        Length::new(value, unit)
    }

    /// A port end's instance path, port name and contact.
    fn on_port<'e>(end: &'e End<'e>) -> (&'e InstancePath, &'e str, Contact<'e>) {
        let port = end.port().expect("an end on a port");
        (&port.instance.path, port.port.name.as_str(), port.contact())
    }

    fn pins(designator: &ConnectorName, pins: &[u32]) -> (String, Vec<Pin>) {
        (
            designator.to_string(),
            pins.iter().copied().map(Pin).collect(),
        )
    }

    fn contact_pins(contact: Contact<'_>) -> Option<(String, Vec<Pin>)> {
        match contact {
            Contact::Connector { designator, pins } => {
                Some((designator.to_string(), pins.to_vec()))
            }
            Contact::Connectorless => None,
        }
    }

    fn conductors<'l, 'd>(list: &'l WireList<'d>) -> Vec<&'l Conductor<'d>> {
        list.conductors().map(|(_, conductor)| conductor).collect()
    }

    #[test]
    fn a_rail_of_four_endpoints_is_three_conductors_in_written_order() {
        let design = design(&format!(
            "{USES}component Car {{ a: Ecu; b: Ecu; c: Ecu; d: Ecu;\n\
             wire white 0.5 \"WAKE\" [a.sense, b.sense, c.sense, d.sense]; }}\n"
        ));
        let list = WireList::from(&design);
        let rows: Vec<(usize, &InstancePath, &InstancePath)> = conductors(&list)
            .iter()
            .map(|c| {
                (
                    c.segment.index,
                    on_port(&c.ends[0]).0,
                    on_port(&c.ends[1]).0,
                )
            })
            .collect();
        let [a, b, c, d] = ["a", "b", "c", "d"].map(|name| path(&format!("Car.{name}")));
        assert_eq!(rows, [(0, &a, &b), (1, &b, &c), (2, &c, &d)]);
        assert!(
            conductors(&list)
                .iter()
                .all(|c| c.wire.color.code() == "WH" && c.wire.gauge == 0.5)
        );
    }

    #[test]
    fn a_cable_conductor_takes_the_cable_length_unless_it_sets_its_own() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; node: Ecu;\n\
             cable can \"CAN\" {{ type: \"STP\"; length: 2m;\n\
               twisted {{\n\
                 wire yellow 0.5 \"CAN H\" [ecu.can_h, node.can_h] {{ length: 150mm; part: \"TP-YE\"; }}\n\
                 wire green  0.5 \"CAN L\" [ecu.can_l, node.can_l];\n\
               }}\n\
               wire black 1 \"PWR\" [ecu.pwr, node.pwr] {{ length: 210cm; type: \"FLRY-B\"; }}\n\
             }}\n\
             wire red 1 [ecu.sense, node.sense]; }}\n"
        ));
        let list = WireList::from(&design);
        let rows: Vec<_> = list
            .conductors()
            .map(|(row, c)| {
                (
                    row.cable.map(|cable| cable.name.as_str()),
                    c.wire.label.as_deref(),
                    c.wire.twisted_group,
                    row.length,
                    c.wire.r#type.as_deref(),
                    c.wire.part.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    Some("can"),
                    Some("CAN H"),
                    Some(0),
                    length(150.0, LengthUnit::Millimetre),
                    None,
                    Some("TP-YE"),
                ),
                (
                    Some("can"),
                    Some("CAN L"),
                    Some(0),
                    length(2.0, LengthUnit::Metre),
                    None,
                    None
                ),
                (
                    Some("can"),
                    Some("PWR"),
                    None,
                    length(2.1, LengthUnit::Metre),
                    Some("FLRY-B"),
                    None,
                ),
                (None, None, None, None, None, None),
            ]
        );
        let (row, _) = list.conductors().next().expect("a conductor");
        assert_eq!(
            row.cable.and_then(|c| c.meta.r#type.as_deref()),
            Some("STP")
        );
    }

    #[test]
    fn two_conductors_in_one_cavity_are_two_ordinary_rows() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; a: Ecu; b: Ecu;\n\
             wire black 1   [ecu.pwr, a.pwr];\n\
             wire brown 0.5 [ecu.pwr, b.pwr]; }}\n"
        ));
        let list = WireList::from(&design);
        let at_ecu: Vec<_> = conductors(&list)
            .iter()
            .map(|c| {
                let (path, port, contact) = on_port(&c.ends[0]);
                (
                    path.clone(),
                    port.to_string(),
                    contact_pins(contact),
                    c.wire.color.code(),
                )
            })
            .collect();
        let x1 = ConnectorName::from("x1");
        assert_eq!(
            at_ecu,
            [
                (
                    path("Car.ecu"),
                    "pwr".to_string(),
                    Some(pins(&x1, &[1])),
                    "BK".to_string()
                ),
                (
                    path("Car.ecu"),
                    "pwr".to_string(),
                    Some(pins(&x1, &[1])),
                    "BN".to_string()
                ),
            ]
        );
    }

    #[test]
    fn an_end_on_a_connectorless_port_is_kept_without_a_connector() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; bcm: Ecu; wire red 0.5 [ecu.sense, bcm.gnd]; }}\n"
        ));
        let list = WireList::from(&design);
        let [conductor] = conductors(&list)[..] else {
            panic!("one conductor");
        };
        let (path_from, port, contact) = on_port(&conductor.ends[0]);
        assert_eq!((path_from, port), (&path("Car.ecu"), "sense"));
        assert_eq!(contact, Contact::Connectorless);
        // A ganged port lands on every cavity it occupies.
        assert_eq!(
            contact_pins(on_port(&conductor.ends[1]).2),
            Some(pins(&ConnectorName::from("x1"), &[2, 3]))
        );
    }

    #[test]
    fn an_end_on_an_inline_connector_is_a_pin_of_its_designator() {
        let design = design(&format!(
            "{USES}connector_type M1 \"M 1p\" {{ }}\nconnector_type F1 \"F 1p\" {{ }}\n\
             component Car {{ ecu: Ecu; pedal: Ecu;\n\
               inline ic \"Branch\" {{ male: M1; female: F1; port sig \"SIG\" pin 1; }}\n\
               wire green 0.5 [ecu.pwr, ic.sig];\n\
               wire green 0.5 [ic.sig, pedal.pwr]; }}\n"
        ));
        let list = WireList::from(&design);
        let conductors = conductors(&list);
        for end in [&conductors[0].ends[1], &conductors[1].ends[0]] {
            let (at, port, contact) = on_port(end);
            assert_eq!((at, port), (&path("Car.ic"), "sig"));
            assert_eq!(
                contact_pins(contact),
                Some(pins(&ConnectorName::from("ic"), &[1]))
            );
            let inline = end.port().and_then(PortEnd::inline).expect("an inline end");
            assert!(inline.male.is_some() && inline.female.is_some());
        }
    }

    #[test]
    fn each_conductor_to_a_splice_has_the_splice_as_an_end() {
        let design = design(&format!(
            "{USES}component Car {{ a: Ecu; b: Ecu; bat: Ecu; splice sp \"GND splice\";\n\
             wire black 0.5 [a.gnd, sp];\n\
             wire black 0.5 [b.gnd, sp];\n\
             wire black 2.5 [sp, bat.gnd]; }}\n"
        ));
        let list = WireList::from(&design);
        let rows: Vec<(usize, &str, f64)> = conductors(&list)
            .iter()
            .map(|c| {
                let (i, splice) = c
                    .ends
                    .iter()
                    .enumerate()
                    .find_map(|(i, end)| match end {
                        End::Splice(splice) => Some((i, splice)),
                        _ => None,
                    })
                    .expect("a splice end");
                assert_eq!(splice.owner.path, path("Car"));
                assert_eq!(splice.splice.label.as_deref(), Some("GND splice"));
                (i, splice.splice.name.as_str(), c.wire.gauge)
            })
            .collect();
        assert_eq!(rows, [(1, "sp", 0.5), (1, "sp", 0.5), (0, "sp", 2.5)]);
    }

    #[test]
    fn cable_screens_follow_the_conductors_with_their_drains() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; node: Ecu;\n\
             cable can \"CAN\" {{ length: 3;\n\
               twisted {{\n\
                 wire yellow 0.5 [ecu.can_h, node.can_h];\n\
                 wire green  0.5 [ecu.can_l, node.can_l];\n\
                 shield;\n\
               }}\n\
               shield \"SHLD\" [ecu.shld];\n\
             }}\n\
             wire red 1 [ecu.pwr, node.pwr]; }}\n"
        ));
        let list = WireList::from(&design);
        let kinds: Vec<bool> = list
            .rows()
            .iter()
            .map(|row| matches!(row.kind, RowKind::Shield(_)))
            .collect();
        assert_eq!(kinds, [false, false, false, true, true]);

        let shields: Vec<_> = list.shields().collect();
        let [(overall_row, overall), (pair_row, pair)] = shields[..] else {
            panic!("two screens");
        };
        assert_eq!(overall.screened, Screened::Cable);
        assert_eq!(overall.shield.label.as_deref(), Some("SHLD"));
        assert_eq!(overall_row.cable.map(|c| c.name.as_str()), Some("can"));
        assert_eq!(overall_row.length, length(3.0, LengthUnit::Metre));
        let Drain::Single(drain) = overall.drain else {
            panic!("grounded at one end");
        };
        let (at, port, contact) = on_port(&drain);
        assert_eq!((at, port), (&path("Car.ecu"), "shld"));
        assert_eq!(
            contact_pins(contact),
            Some(pins(&ConnectorName::from("x1"), &[6]))
        );
        assert_eq!(overall_row.ends().len(), 1);

        assert_eq!(pair.screened, Screened::Pair(0));
        assert!(matches!(pair.drain, Drain::NoPin));
        assert!(pair_row.ends().is_empty());
        assert_eq!(pair_row.length, length(3.0, LengthUnit::Metre));
    }

    #[test]
    fn a_type_placed_twice_lists_its_wires_once_per_placement() {
        let design = design(&format!(
            "{USES}component Car {{ front: Loom; rear: Loom; }}\n"
        ));
        let list = WireList::from(&design);
        let rows: Vec<_> = list
            .conductors()
            .map(|(row, c)| {
                let (from, from_port, _) = on_port(&c.ends[0]);
                let (to, to_port, _) = on_port(&c.ends[1]);
                (&row.owner.path, from, from_port, to, to_port)
            })
            .collect();
        let (front, rear) = (path("Car.front"), path("Car.rear"));
        let (front_lamp, rear_lamp) = (path("Car.front.lamp"), path("Car.rear.lamp"));
        assert_eq!(
            rows,
            [
                (&front, &front, "feed", &front_lamp, "pwr"),
                (&front, &front, "out", &front_lamp, "gnd"),
                (&rear, &rear, "feed", &rear_lamp, "pwr"),
                (&rear, &rear, "out", &rear_lamp, "gnd"),
            ]
        );
    }

    #[test]
    fn a_nested_components_own_wires_end_on_its_boundary() {
        let design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; loom: Loom;\n\
             wire red 1 [ecu.pwr, loom.feed]; }}\n"
        ));
        let list = WireList::from(&design);
        let rows: Vec<_> = list.conductors().collect();
        let owners: Vec<&InstancePath> = rows.iter().map(|(row, _)| &row.owner.path).collect();
        assert_eq!(owners, [&path("Car"), &path("Car.loom"), &path("Car.loom")]);

        // The car's wire lands on the loom's port from outside…
        let (_, wire_in) = rows[0];
        assert!(matches!(wire_in.ends[1], End::Port(_)));
        assert_eq!(on_port(&wire_in.ends[1]).0, &path("Car.loom"));
        // …and the loom's own wires leave from it, named as its body names
        // them: the connectorless `feed`, and `out` on its own connector.
        let (_, feed) = rows[1];
        assert!(matches!(feed.ends[0], End::Boundary(_)));
        assert_eq!(feed.segment.ends[0], &WireEnd::Own(PortName::from("feed")));
        assert_eq!(on_port(&feed.ends[0]).2, Contact::Connectorless);
        let (_, out) = rows[2];
        assert!(matches!(out.ends[0], End::Boundary(_)));
        assert_eq!(
            contact_pins(on_port(&out.ends[0]).2),
            Some(pins(&ConnectorName::from("x9"), &[1]))
        );
        assert_eq!(on_port(&out.ends[1]).0, &path("Car.loom.lamp"));
    }

    #[test]
    fn an_end_the_design_does_not_resolve_keeps_its_row() {
        let mut design = design(&format!(
            "{USES}component Car {{ ecu: Ecu; node: Ecu; wire red 1 [ecu.pwr, node.pwr]; }}\n"
        ));
        let root = design.root.clone();
        let car = design.instances.get_mut(&root).expect("the root");
        car.wires[0].endpoints[1] = WireEnd::Child {
            instance: InstanceName::from("ghost"),
            port: PortName::from("pwr"),
        };
        let list = WireList::from(&design);
        let [conductor] = conductors(&list)[..] else {
            panic!("one conductor");
        };
        assert!(matches!(conductor.ends[1], End::Unresolved(_)));
        assert_eq!(conductor.ends[1].to_string(), "ghost.pwr");
    }

    #[test]
    fn an_end_displays_as_the_full_path_of_what_it_lands_on() {
        let design = design(&format!(
            "{USES}component Car {{ loom: Loom; splice sp; wire red 1 [loom.feed, sp]; wire red 1 [sp, loom.out]; }}\n"
        ));
        let list = WireList::from(&design);
        let ends: Vec<String> = list
            .rows()
            .iter()
            .flat_map(Row::ends)
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            ends,
            [
                "Car.loom.feed",
                "Car.sp",
                "Car.sp",
                "Car.loom.out",
                "Car.loom.feed",
                "Car.loom.lamp.pwr",
                "Car.loom.out",
                "Car.loom.lamp.gnd",
            ]
        );
    }
}
