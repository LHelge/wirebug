//! What a harness view draws, from the IR alone: which conductors and
//! screens it scopes in, and through which half it includes an inline
//! connector. The harness renderer draws exactly this, and the build tables
//! (`export`) read the same answer to tell which housing half of an inline
//! connector a conductor is crimped into, so the two cannot disagree.

use super::{
    CableName, ConnectorName, Design, Half, Include, Instance, InstanceName, Screened, SpliceName,
    View, WireEnd,
};

/// What a conductor end lands on in a harness view of a subject, and what
/// an include places there: a child instance's connector, or one of the
/// subject's splices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HarnessNode<'a> {
    Connector(&'a InstanceName, &'a ConnectorName),
    Splice(&'a SpliceName),
}

impl<'a> HarnessNode<'a> {
    /// What `include` places in a harness view of `subject`: an instance's
    /// connector, a splice (an include naming one alone), or nothing a
    /// harness draws.
    pub(crate) fn of_include(subject: &'a Instance, include: &'a Include) -> Option<Self> {
        match &include.connector {
            Some(connector) => Some(Self::Connector(&include.instance, connector)),
            None => subject
                .splices
                .get_key_value(&SpliceName::from(include.instance.as_str()))
                .map(|(name, _)| Self::Splice(name)),
        }
    }

    /// Where `end`, written in `subject`'s body, lands in a harness view of
    /// it: the connector of a child's port, or a splice. `None` for an end
    /// no harness view can draw: the subject's own port, or a port on no
    /// connector.
    pub(crate) fn of_end(
        design: &'a Design,
        subject: &'a Instance,
        end: &'a WireEnd,
    ) -> Option<Self> {
        match end {
            WireEnd::Own(_) => None,
            WireEnd::Child { instance, port } => {
                let child = subject.children.get(instance).and_then(|p| design.get(p))?;
                let connector = &child.ports.get(port)?.connector.as_ref()?.name;
                Some(Self::Connector(instance, connector))
            }
            WireEnd::Splice(splice) => Some(Self::Splice(splice)),
        }
    }

    /// The two nodes a conductor with `ends` runs between in a harness view
    /// of `subject`. `None` when an end lands on no node, or both on the
    /// same one: a jumper inside one housing is not a harness conductor.
    pub(crate) fn between(
        design: &'a Design,
        subject: &'a Instance,
        ends: [&'a WireEnd; 2],
    ) -> Option<[Self; 2]> {
        let [from, to] = ends;
        let from = Self::of_end(design, subject, from)?;
        let to = Self::of_end(design, subject, to)?;
        (from != to).then_some([from, to])
    }
}

/// One harness view of `subject`, read for what it draws: a conductor when
/// it includes the nodes at both its ends, a cable's screen when it draws
/// a conductor of what the screen covers, and a drain end when it draws the
/// screen and includes the end's connector.
#[derive(Debug, Clone)]
pub(crate) struct HarnessScope<'a> {
    design: &'a Design,
    subject: &'a Instance,
    view: &'a View,
    included: Vec<HarnessNode<'a>>,
}

impl<'a> HarnessScope<'a> {
    /// The scope of `view`, a harness view of `subject`'s type, drawn
    /// against `subject`.
    pub(crate) fn new(design: &'a Design, subject: &'a Instance, view: &'a View) -> Self {
        Self {
            design,
            subject,
            view,
            included: view
                .includes
                .iter()
                .filter_map(|include| HarnessNode::of_include(subject, include))
                .collect(),
        }
    }

    /// Whether the view includes `node`.
    pub(crate) fn includes(&self, node: HarnessNode<'_>) -> bool {
        self.included.contains(&node)
    }

    /// Whether the view draws the conductor running between `ends`, two
    /// ends written in the subject's body (a segment of one of its wires).
    pub(crate) fn draws(&self, ends: [&'a WireEnd; 2]) -> bool {
        HarnessNode::between(self.design, self.subject, ends)
            .is_some_and(|nodes| nodes.iter().all(|&node| self.includes(node)))
    }

    /// Whether the view draws the screen of `cable`, one of the subject's,
    /// that covers `screened`: the cable's overall screen when the view
    /// draws any of its conductors, a pair's when it draws one of the
    /// pair's.
    pub(crate) fn draws_screen(&self, cable: &CableName, screened: Screened) -> bool {
        self.subject
            .wires
            .iter()
            .filter(|wire| wire.cable.as_ref() == Some(cable))
            .filter(|wire| match screened {
                Screened::Cable => true,
                Screened::Pair(group) => wire.twisted_group == Some(group),
            })
            .flat_map(|wire| wire.segments())
            .any(|segment| self.draws(segment.ends))
    }

    /// Whether the view draws the drain of that screen to `end`, one of
    /// the ends it is terminated on.
    pub(crate) fn draws_drain(
        &self,
        cable: &CableName,
        screened: Screened,
        end: &'a WireEnd,
    ) -> bool {
        self.draws_screen(cable, screened)
            && HarnessNode::of_end(self.design, self.subject, end)
                .is_some_and(|node| self.includes(node))
    }

    /// The housing half the view includes the inline connector `inline`
    /// through, a child of the subject; `None` when it does not include it.
    pub(crate) fn half(&self, inline: &InstanceName) -> Option<Half> {
        self.view
            .includes
            .iter()
            .filter(|include| &include.instance == inline)
            .find_map(|include| include.half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checked design of `main`, its one top-level component the root.
    /// Panics on an error.
    fn design(main: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in [
            (
                "wirebug.toml",
                "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
            ),
            ("main.wb", main),
        ] {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        let report = crate::check_project(Some(dir.path()));
        let errors: Vec<_> = report.problems.iter().filter(|p| p.is_error()).collect();
        assert!(errors.is_empty(), "the source checks: {errors:?}");
        report.design.expect("a design")
    }

    const CAR: &str = r#"
connector_type M2 "M 2p" { }
connector_type F2 "F 2p" { }

component Car {
    ecu: Dev; lamp: Dev;
    inline ic "Branch" { male: M2; female: F2; port a "A" pin 1; port b "B" pin 2; }
    splice sp;
    cable c "C" {
        twisted {
            wire red   0.5 [ecu.a, ic.a];
            wire black 0.5 [ecu.b, ic.b];
            shield [ecu.s];
        }
        shield [ecu.s, ic.b];
    }
    wire white 0.5 [ic.a, lamp.a];
    wire blue  0.5 [ecu.a, ecu.b];
    wire green 0.5 [lamp.b, sp];
    wire brown 0.5 [ecu.n, lamp.n];

    component Dev {
        connector x1 "X 3p" { pub port a "A" pin 1; pub port b "B" pin 2; pub port s "S" pin 3; }
        pub port n "N";
    }
}

view harness "ECU side" {
    include ecu.x1 at (0, 0);
    include ic.female at (10, 0);
}
"#;

    fn scope(design: &Design) -> HarnessScope<'_> {
        let subject = design.get(&design.root).expect("the root");
        HarnessScope::new(design, subject, &design.views[0])
    }

    fn ends(design: &Design, wire: usize) -> [&WireEnd; 2] {
        let car = design.get(&design.root).expect("the root");
        let segment = car.wires[wire].segments().next().expect("a segment");
        segment.ends
    }

    #[test]
    fn a_view_draws_a_conductor_whose_two_ends_it_includes() {
        let design = design(CAR);
        let scope = scope(&design);
        // The cable's pair runs ECU → inline; the white wire leaves the
        // inline for the lamp, which the view leaves out.
        assert!(scope.draws(ends(&design, 0)));
        assert!(scope.draws(ends(&design, 1)));
        assert!(!scope.draws(ends(&design, 2)));
    }

    #[test]
    fn a_jumper_or_an_end_on_no_connector_is_never_drawn() {
        let design = design(CAR);
        let scope = scope(&design);
        assert!(!scope.draws(ends(&design, 3)), "a jumper within ecu.x1");
        assert!(!scope.draws(ends(&design, 5)), "connectorless ports");
        let car = design.get(&design.root).expect("the root");
        assert_eq!(
            HarnessNode::of_end(&design, car, &WireEnd::Splice(SpliceName::from("sp"))),
            Some(HarnessNode::Splice(&SpliceName::from("sp")))
        );
    }

    #[test]
    fn a_screen_is_drawn_with_a_conductor_it_covers_and_its_drain_to_an_included_pin() {
        let design = design(CAR);
        let scope = scope(&design);
        let cable = CableName::from("c");
        assert!(scope.draws_screen(&cable, Screened::Cable));
        assert!(scope.draws_screen(&cable, Screened::Pair(0)));
        assert!(!scope.draws_screen(&cable, Screened::Pair(1)));
        let at = |instance: &str, port: &str| WireEnd::Child {
            instance: InstanceName::from(instance),
            port: crate::dsl::ir::PortName::from(port),
        };
        let (ecu_s, lamp_s) = (at("ecu", "s"), at("lamp", "s"));
        assert!(scope.draws_drain(&cable, Screened::Cable, &ecu_s));
        assert!(!scope.draws_drain(&cable, Screened::Cable, &lamp_s));
        assert!(!scope.draws_drain(&CableName::from("none"), Screened::Cable, &ecu_s));
    }

    #[test]
    fn a_view_names_the_half_it_includes_an_inline_through() {
        let design = design(CAR);
        let scope = scope(&design);
        assert_eq!(scope.half(&InstanceName::from("ic")), Some(Half::Female));
        assert_eq!(scope.half(&InstanceName::from("ecu")), None);
        assert_eq!(scope.half(&InstanceName::from("lamp")), None);
    }
}
