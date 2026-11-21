//! Checks that need a view laid out: problems only visible once the
//! renderers have sized and placed every include's box.
//!
//! The DSL pipeline ends at `ir::Design` and never draws anything, so it
//! cannot tell how big a box is: that follows from ports, labels and pin
//! rows, in each renderer's layout code. These checks ask that same layout
//! code, so a check and the drawing cannot disagree, and they report
//! `Problem` diagnostics pointing at the includes in the source.
//! `pipeline::check_project` runs them after the DSL stages.

use std::cell::OnceCell;
use std::collections::HashSet;
use std::fmt;

use indexmap::IndexMap;

use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{
    ConnectorName, Design, Half, InlineMeta, Instance, InstanceName, PortName, SpliceName,
    TypeName, View, ViewKind, Wire, WireEnd,
};
use crate::dsl::project::Project;
use crate::export::PinTables;
use crate::render::geometry::Bounds;
use crate::render::harness::HarnessRenderer;
use crate::render::pinout::PinoutRenderer;
use crate::render::schematic::SchematicRenderer;
use crate::render::{Canvas, GridFault, MAX_CANVAS};

/// One include's drawn box.
pub(super) struct IncludeBox {
    /// The include's index in `view.includes`.
    pub(super) include: usize,
    pub(super) bounds: Bounds,
}

/// A wired port whose wire the renderer cannot route, because another
/// include's box sits within the routing clearance in front of it.
pub(super) struct CrowdedPort {
    /// Index in `view.includes` of the include that owns the port.
    pub(super) include: usize,
    pub(super) port: PortName,
    /// Index in `view.includes` of the include in the way.
    pub(super) by: usize,
}

/// One end of a wire, located for a diagnostic.
pub(super) struct WireEndAt {
    /// Index in `view.includes` of the include that owns the port; `None`
    /// for a port on the enclosure.
    pub(super) include: Option<usize>,
    pub(super) port: PortName,
}

/// A wire the router finds no path for although both its ports have room
/// to leave: keepouts that meet wall one end in.
pub(super) struct UnroutableWire {
    pub(super) ends: [WireEndAt; 2],
}

/// One chain segment of a subject wire: the pair `endpoints[index]`,
/// `endpoints[index + 1]` of `subject.wires[wire]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Segment {
    pub(super) wire: usize,
    pub(super) index: usize,
}

/// A conductor a harness view of its component can draw: a segment between
/// connectors of two different children of the subject, or a wire with an
/// end on one of the subject's splices.
pub(super) struct Conductor<'d> {
    pub(super) segment: Segment,
    pub(super) wire: &'d Wire,
    /// The segment's two endpoints, in wire order.
    pub(super) ends: [&'d WireEnd; 2],
    /// How a harness view can include each end's connector or splice.
    pub(super) targets: [EndTarget<'d>; 2],
}

/// How a harness view can include what one end of a conductor lands on.
pub(super) enum EndTarget<'d> {
    /// An include that draws it.
    Include(IncludeTarget<'d>),
    /// An inline connector that declares no housing half: no harness
    /// include can name it until it declares one.
    HalflessInline(&'d InstanceName),
}

/// A harness include target that resolves: `inst.connector`, an inline
/// connector through one of the halves it declares, or a splice by its
/// name.
pub(super) enum IncludeTarget<'d> {
    Connector {
        instance: &'d InstanceName,
        connector: &'d ConnectorName,
    },
    Inline {
        instance: &'d InstanceName,
        halves: DeclaredHalves,
    },
    Splice(&'d SpliceName),
}

/// The housing halves an inline connector declares, when it declares any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeclaredHalves {
    Male,
    Female,
    Both,
}

impl DeclaredHalves {
    /// The halves `inline` declares; `None` when it declares neither.
    pub(super) fn of(inline: &InlineMeta) -> Option<Self> {
        match (inline.male.is_some(), inline.female.is_some()) {
            (true, false) => Some(Self::Male),
            (false, true) => Some(Self::Female),
            (true, true) => Some(Self::Both),
            (false, false) => None,
        }
    }
}

/// Backticked, as the help names it: `` `a.j` ``, `` `ic.male` ``,
/// `` `ic.male` (or `ic.female`) `` when either half draws it, or a
/// splice's `` `sp` ``.
impl fmt::Display for IncludeTarget<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (instance, halves) = match self {
            Self::Connector {
                instance,
                connector,
            } => return write!(f, "`{instance}.{connector}`"),
            Self::Splice(splice) => return write!(f, "`{splice}`"),
            Self::Inline { instance, halves } => (instance, halves),
        };
        let half = |h: Half| format!("`{instance}.{}`", h.as_str());
        match halves {
            DeclaredHalves::Male => f.write_str(&half(Half::Male)),
            DeclaredHalves::Female => f.write_str(&half(Half::Female)),
            DeclaredHalves::Both => {
                write!(f, "{} (or {})", half(Half::Male), half(Half::Female))
            }
        }
    }
}

/// What a renderer's layout reports about one view, for checking.
#[derive(Default)]
pub(super) struct ViewLayout {
    pub(super) boxes: Vec<IncludeBox>,
    /// Always empty for kinds whose wires are not routed around boxes, as
    /// is `unroutable`.
    pub(super) crowded: Vec<CrowdedPort>,
    pub(super) unroutable: Vec<UnroutableWire>,
    /// The conductors a harness view draws; empty for other kinds.
    pub(super) drawn: Vec<Segment>,
    /// Why the renderer rejects the view's `grid:`, when it does; nothing
    /// is laid out on such a grid, so no box or wire is reported.
    pub(super) grid: Option<GridFault>,
    /// The canvas of the view's standalone SVG, the largest of the output
    /// modes and the one `--png` rasterises; `None` when the view cannot
    /// be drawn, for a reason reported on its own.
    pub(super) canvas: Option<Canvas>,
}

/// The conductors each component's harness views draw between them, keyed
/// by component in the order its first harness view appears. A conductor
/// is covered when any one of them draws it.
#[derive(Default)]
struct HarnessCoverage<'d> {
    components: IndexMap<&'d TypeName, (&'d Instance, HashSet<Segment>)>,
}

impl<'d> HarnessCoverage<'d> {
    /// Count `drawn` as covered for `view`'s component, whose views all
    /// render against `subject`.
    fn add(&mut self, view: &'d View, subject: &'d Instance, drawn: Vec<Segment>) {
        self.components
            .entry(&view.subject)
            .or_insert_with(|| (subject, HashSet::new()))
            .1
            .extend(drawn);
    }

    /// One `undrawn_conductor` warning per conductor of a component with
    /// harness views that none of them draws, in component then wire order.
    /// A component with no harness view is not built from drawings yet, so
    /// is never reported.
    fn problems(&self, design: &'d Design, project: &Project) -> Vec<Problem> {
        self.components
            .iter()
            .flat_map(|(component, (subject, drawn))| {
                HarnessRenderer
                    .conductors(design, subject)
                    .into_iter()
                    .filter(|c| !drawn.contains(&c.segment))
                    .map(|c| c.undrawn(component, project))
            })
            .collect()
    }
}

impl Conductor<'_> {
    /// What to change so a harness view of `component` draws this
    /// conductor: the two includes, or first a half for each inline
    /// connector that declares none (an include of an undeclared half is
    /// itself an error).
    fn advice(&self, component: &TypeName) -> String {
        let spliced = self
            .targets
            .iter()
            .any(|t| matches!(t, EndTarget::Include(IncludeTarget::Splice(_))));
        let draws = if spliced {
            "a harness view draws a conductor only when it includes what both its ends land on, connector or splice"
        } else {
            "a harness view draws a conductor only when it includes the connectors at both ends"
        };
        if let [EndTarget::Include(a), EndTarget::Include(b)] = &self.targets {
            return format!("{draws}; include {a} and {b} in one harness view of `{component}`");
        }
        let halfless: Vec<String> = self
            .targets
            .iter()
            .filter_map(|t| match t {
                EndTarget::HalflessInline(instance) => Some(format!("`{instance}`")),
                EndTarget::Include(_) => None,
            })
            .collect();
        match halfless.as_slice() {
            [one] => format!(
                "{draws}, and inline connector {one} must declare a `male` or `female` half before a harness view can include it"
            ),
            many => format!(
                "{draws}, and inline connectors {} must each declare a `male` or `female` half before a harness view can include them",
                many.join(" and ")
            ),
        }
    }

    /// The warning that no harness view of `component` draws this conductor.
    fn undrawn(&self, component: &TypeName, project: &Project) -> Problem {
        let wire = self.wire;
        let mut conductor = match &wire.cable {
            Some(cable) => format!("cable `{cable}` conductor"),
            None => "wire".to_string(),
        };
        if let Some(label) = &wire.label {
            conductor.push_str(&format!(" \"{label}\""));
        }
        let [from, to] = self.ends;
        Problem::UndrawnConductor {
            conductor,
            from: from.to_string(),
            to: to.to_string(),
            component: component.to_string(),
            advice: self.advice(component),
            src: project.source(wire.span.file),
            at: wire.span.into(),
        }
    }
}

/// Lay out every view of `design` as the renderers would and report what
/// the layout shows to be wrong: includes whose boxes overlap
/// (`overlapping_includes`), schematic includes too close for a wire to
/// leave a port (`include_too_close`), schematic wires walled in by boxes
/// all around (`unroutable_wire`), and, after every view's problems,
/// conductors that no harness view of their component draws
/// (`undrawn_conductor`). `project` supplies the sources the spans point
/// into.
///
/// Every view kind is also held to [`MAX_CANVAS`] (`view_too_large`,
/// labelled on the view's title and the includes farthest apart), the
/// limit `--png` refuses to rasterise past.
///
/// A view whose `grid:` its renderer rejects reports that alone: a step
/// that is not positive (`invalid_grid`), or one finer than the renderer
/// can draw ports on (`grid_too_fine`), labelled on the number.
///
/// A view the renderers cannot lay out at all (no subject instance, an
/// unknown kind) is skipped: earlier phases report those
/// (`uninstantiated_view_subject`, `unknown_view_kind`), and these checks
/// run only on a design without errors.
#[must_use]
pub(crate) fn layout_problems(design: &Design, project: &Project) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut coverage = HarnessCoverage::default();
    let pin_tables = OnceCell::new();
    for view in &design.views {
        let Ok(subject) = super::subject_instance(design, view) else {
            continue;
        };
        let layout = match &view.kind {
            ViewKind::Schematic => SchematicRenderer.layout(design, subject, view),
            ViewKind::Harness => HarnessRenderer.layout(design, subject, view),
            ViewKind::Pinout => {
                let tables = pin_tables.get_or_init(|| PinTables::from(design));
                PinoutRenderer.layout(design, tables, subject, view)
            }
            ViewKind::Other(_) => continue,
        };
        problems.extend(layout.problems(view, project));
        if view.kind == ViewKind::Harness {
            coverage.add(view, subject, layout.drawn);
        }
    }
    problems.extend(coverage.problems(design, project));
    problems
}

impl ViewLayout {
    /// The layout of a view whose grid the renderer rejects: only the fault.
    pub(super) fn bad_grid(fault: GridFault) -> Self {
        Self {
            grid: Some(fault),
            ..Self::default()
        }
    }

    /// A rejected grid alone, when there is one. Otherwise overlapping
    /// pairs first, then crowded pairs, each in include order, then
    /// unroutable wires in wire order, then a canvas over the limit. A pair
    /// that overlaps is not also reported as too close, and unroutable
    /// wires are reported only in a view with neither: colliding boxes are
    /// the cause to fix first, and a port without room to leave never
    /// routes anyway.
    fn problems(&self, view: &View, project: &Project) -> Vec<Problem> {
        if let Some(fault) = self.grid {
            // A renderer's default grid is always valid, so a rejected grid
            // is an authored one, with a number to point at.
            return view
                .grid
                .map(|grid| {
                    let src = project.source(grid.span.file);
                    let at = grid.span.into();
                    match fault {
                        GridFault::NonPositive { grid } => Problem::InvalidGrid {
                            value: grid,
                            src,
                            at,
                        },
                        GridFault::TooFine { grid, minimum } => Problem::GridTooFine {
                            value: grid,
                            minimum,
                            kind: view.kind.to_string(),
                            src,
                            at,
                        },
                    }
                })
                .into_iter()
                .collect();
        }

        let mut overlapping = Vec::new();
        for (k, later) in self.boxes.iter().enumerate() {
            for earlier in &self.boxes[..k] {
                if earlier.bounds.overlaps(&later.bounds) {
                    overlapping.push(pair(earlier.include, later.include));
                }
            }
        }

        let mut crowded: IndexMap<(usize, usize), Vec<String>> = IndexMap::new();
        for port in &self.crowded {
            let key = pair(port.include, port.by);
            if !overlapping.contains(&key) {
                let owner = &view.includes[port.include].instance;
                crowded
                    .entry(key)
                    .or_default()
                    .push(format!("`{owner}.{}`", port.port));
            }
        }

        // Both includes of a pair belong to one view, and a view sits in
        // one file, so the later include's source carries both labels.
        let includes = |(first, at): (usize, usize)| (&view.includes[first], &view.includes[at]);
        let overlaps = overlapping.into_iter().map(|key| {
            let (first, at) = includes(key);
            Problem::OverlappingIncludes {
                include: at.to_string(),
                other: first.to_string(),
                view: view.title.clone(),
                src: project.source(at.span.file),
                at: at.span.into(),
                first: first.span.into(),
            }
        });
        let too_close = crowded.into_iter().map(|(key, ports)| {
            let (first, at) = includes(key);
            Problem::IncludeTooClose {
                include: at.to_string(),
                other: first.to_string(),
                view: view.title.clone(),
                ports: ports.join(", "),
                src: project.source(at.span.file),
                at: at.span.into(),
                first: first.span.into(),
            }
        });
        let mut problems: Vec<Problem> = overlaps.chain(too_close).collect();
        if problems.is_empty() {
            problems.extend(self.unroutable.iter().filter_map(|wire| {
                let [from, to] = &wire.ends;
                let named = |end: &WireEndAt| match end.include {
                    Some(i) => format!("{}.{}", view.includes[i].instance, end.port),
                    None => end.port.to_string(),
                };
                // Label the includes the wire ends on. A wire between two
                // enclosure ports has none to point at, but never needs
                // one: the enclosure stands four steps off every box, so
                // the ring inside it is always open.
                let mut spans = wire
                    .ends
                    .iter()
                    .filter_map(|e| Some(view.includes[e.include?].span));
                let at = spans.next()?;
                Some(Problem::UnroutableWire {
                    from: named(from),
                    to: named(to),
                    view: view.title.clone(),
                    src: project.source(at.file),
                    at: at.into(),
                    other: spans.next().map(Into::into),
                })
            }));
        }
        problems.extend(self.too_large(view, project));
        problems
    }

    /// `view_too_large` when the canvas is over [`MAX_CANVAS`], labelled on
    /// the view's title and on the includes whose boxes lie farthest apart
    /// along the longer side.
    fn too_large(&self, view: &View, project: &Project) -> Option<Problem> {
        let canvas = self.canvas.filter(|canvas| !canvas.fits())?;
        let across = canvas.width() >= canvas.height();
        let span = |b: &IncludeBox| {
            let Bounds {
                origin,
                width,
                height,
            } = b.bounds;
            if across {
                (origin.x, origin.x + width)
            } else {
                (origin.y, origin.y + height)
            }
        };
        let first = self
            .boxes
            .iter()
            .min_by(|a, b| span(a).0.total_cmp(&span(b).0))
            .map(|b| b.include);
        let last = self
            .boxes
            .iter()
            .max_by(|a, b| span(a).1.total_cmp(&span(b).1))
            .map(|b| b.include);
        let label = |include: Option<usize>| include.map(|i| view.includes[i].span.into());
        let (width, height) = canvas.reported();
        Some(Problem::ViewTooLarge {
            view: view.title.clone(),
            width,
            height,
            limit: MAX_CANVAS,
            src: project.source(view.span.file),
            at: view.span.into(),
            first: label(first),
            last: label(last.filter(|&last| Some(last) != first)),
        })
    }
}

/// An unordered pair of include indices, earlier first.
fn pair(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::Diagnostic;

    use crate::render::{SvgMode, render_views};

    /// Whether every view of `design` renders.
    fn renders(design: &Design) -> bool {
        render_views(design, SvgMode::Standalone)
            .iter()
            .all(Result::is_ok)
    }

    /// A loaded single-file project and its elaborated design. The temp
    /// directory is dropped once loaded: sources live in the `Project`.
    fn load(src: &str) -> (Project, Design) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"lint\"\nversion = \"0.1.0\"\n",
        )
        .expect("write wirebug.toml");
        let main = dir.path().join("main.wb");
        std::fs::write(&main, src).expect("write main.wb");
        let (project, problems) = crate::dsl::project::load(&main);
        assert!(problems.is_empty(), "load problems: {problems:?}");
        let project = project.expect("project loads");
        let crate::dsl::Checked {
            design, problems, ..
        } = crate::dsl::check_loaded(&project);
        assert!(
            !problems.iter().any(Problem::is_error),
            "check errors: {problems:?}"
        );
        (project, design.expect("a design"))
    }

    /// Each problem as its code and the source text of its labels.
    fn summary(src: &str) -> Vec<(String, Vec<String>)> {
        let (project, design) = load(src);
        layout_problems(&design, &project)
            .iter()
            .map(|p| {
                let code = p.code().expect("a code").to_string();
                let labels = p
                    .labels()
                    .into_iter()
                    .flatten()
                    .map(|l| src[l.offset()..l.offset() + l.len()].to_string())
                    .collect();
                (code, labels)
            })
            .collect()
    }

    /// Two `Node`s wired `a.sig`–`b.sig`, on a 20-unit grid where a node
    /// with one port is 8 steps wide and 4 high; `a` sits at the origin.
    fn two_nodes(b_at: (i32, i32), a_side: &str, b_side: &str) -> String {
        let (x, y) = b_at;
        format!(
            r#"
component Sys {{
    a: Node;
    b: Node;
    wire red 1 [a.sig, b.sig];

    component Node {{
        pub port sig "SIG";
    }}
}}

view schematic "V" {{
    grid: 20;
    include a at (0, 0) ports {{ {a_side}: sig; }}
    include b at ({x}, {y}) ports {{ {b_side}: sig; }}
}}
"#
        )
    }

    fn code(code: &str, labels: &[&str]) -> (String, Vec<String>) {
        (
            format!("wirebug::{code}"),
            labels.iter().map(ToString::to_string).collect(),
        )
    }

    #[test]
    fn overlapping_boxes_are_reported_with_both_includes() {
        assert_eq!(
            summary(&two_nodes((4, 1), "east", "west")),
            [code("overlapping_includes", &["b", "a"])]
        );
    }

    #[test]
    fn boxes_touching_edge_to_edge_are_fine() {
        // `b` sits on `a`'s south edge; neither port faces the other.
        assert!(summary(&two_nodes((0, 4), "east", "east")).is_empty());
    }

    #[test]
    fn facing_ports_one_step_apart_are_too_close() {
        let src = two_nodes((9, 0), "east", "west");
        assert_eq!(summary(&src), [code("include_too_close", &["b", "a"])]);
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert_eq!(
            problem.to_string(),
            "no room to route `a.sig`, `b.sig`: `b` is too close to `a` in view \"V\""
        );
    }

    #[test]
    fn an_overlapping_pair_is_not_also_too_close() {
        assert_eq!(
            summary(&two_nodes((7, 0), "east", "west")),
            [code("overlapping_includes", &["b", "a"])]
        );
    }

    /// The lint asks the router's own precondition, so over every relative
    /// placement it fires exactly when rendering fails.
    #[test]
    fn crowding_is_reported_exactly_when_routing_fails() {
        let sides = [
            ("east", "west"),
            ("east", "north"),
            ("south", "west"),
            ("north", "south"),
        ];
        for x in 0..=11 {
            for y in 0..=6 {
                for (a_side, b_side) in sides {
                    let (project, design) = load(&two_nodes((x, y), a_side, b_side));
                    let problems = layout_problems(&design, &project);
                    if problems
                        .iter()
                        .any(|p| matches!(p, Problem::OverlappingIncludes { .. }))
                    {
                        continue;
                    }
                    assert_eq!(
                        problems.is_empty(),
                        renders(&design),
                        "b at ({x}, {y}), {a_side}/{b_side}: {problems:?}"
                    );
                }
            }
        }
    }

    /// `a`'s port has room to leave, but `n`, `s` and `e` stand within two
    /// steps of each other (and touch `a`), closing every way out.
    const WALLED_IN: &str = r#"
component Sys {
    a: Node;
    n: Node;
    s: Node;
    e: Node;
    far: Node;
    wire red 1 [a.sig, far.sig];

    component Node {
        pub port sig "SIG";
    }
}

view schematic "V" {
    grid: 20;
    include a at (0, 0) ports { east: sig; }
    include n at (8, -3);
    include s at (8, 3);
    include e at (16, 0);
    include far at (30, 0) ports { west: sig; }
}
"#;

    #[test]
    fn a_walled_in_wire_is_unroutable() {
        assert_eq!(summary(WALLED_IN), [code("unroutable_wire", &["a", "far"])]);
        // Rendering anyway fails the view, naming it and both wire ends.
        let (_, design) = load(WALLED_IN);
        let views = render_views(&design, SvgMode::Standalone);
        let [Err(failure)] = views.as_slice() else {
            panic!("the one view fails: {views:?}");
        };
        assert_eq!(
            failure.to_string(),
            "view \"V\": failed to route schematic wire between `a.sig` and `far.sig`"
        );
    }

    /// The router and the lint see a splice's net as the drawing does: the
    /// net's one segment is walled in like the wire it replaces.
    #[test]
    fn a_walled_in_splice_net_is_unroutable() {
        let spliced = WALLED_IN.replace(
            "    wire red 1 [a.sig, far.sig];",
            "    splice sp;\n    wire red 1 [a.sig, sp];\n    wire red 1 [sp, far.sig];",
        );
        assert_eq!(summary(&spliced), [code("unroutable_wire", &["a", "far"])]);
        let (_, design) = load(&spliced);
        assert!(!renders(&design));

        let open = spliced.replace("include e at (16, 0)", "include e at (18, 0)");
        assert!(summary(&open).is_empty());
        let (_, design) = load(&open);
        assert!(renders(&design));
    }

    #[test]
    fn opening_one_wall_makes_the_wire_routable() {
        let open = WALLED_IN.replace("include e at (16, 0)", "include e at (18, 0)");
        assert!(summary(&open).is_empty());
        let (_, design) = load(&open);
        assert!(renders(&design));
    }

    #[test]
    fn the_enclosure_is_never_an_obstacle() {
        // The enclosure wraps both boxes (so it overlaps them) and carries
        // a wired port of its own; none of that is reported.
        let src = r#"
component Sys {
    a: Node;
    b: Node;
    pub port out "OUT";
    wire red 1 [a.sig, b.sig];
    wire red 1 [b.sig, out];

    component Node {
        pub port sig "SIG";
    }
}

view schematic "V" {
    grid: 20;
    enclosure {
        out at (east, 0);
    }
    include a at (0, 0) ports { east: sig; }
    include b at (12, 0) ports { west: sig; }
}
"#;
        assert!(summary(src).is_empty());
        let (_, design) = load(src);
        assert!(renders(&design));
    }

    #[test]
    fn harness_connectors_placed_on_one_spot_overlap() {
        let src = r#"
component Sys {
    l: Node;
    r: Node;
    far: Node;
    wire red 1 [l.sig, r.sig];

    component Node {
        connector j "J 1p" {
            pub port sig "SIG" pin 1;
        }
    }
}

view harness "H" {
    grid: 20;
    include l.j at (0, 0);
    include r.j at (0, 0);
    include far.j at (20, 0);
}
"#;
        assert_eq!(
            summary(src),
            [code("overlapping_includes", &["r.j", "l.j"])]
        );
    }

    /// `a`, `b` and `c` each carry connector `j` (pins `sig`, `ret`) and a
    /// connectorless `bare` port; `body` holds the wires and `views` the
    /// views.
    fn harnessed(body: &str, views: &str) -> String {
        format!(
            r#"
component Sys {{
    a: Node;
    b: Node;
    c: Node;
    pub port out "OUT";
{body}
    component Node {{
        connector j "J 2p" {{
            pub port sig "SIG" pin 1;
            pub port ret "RET" pin 2;
        }}
        pub port bare "BARE";
    }}
}}
{views}
"#
        )
    }

    /// A harness view of `Sys` titled `title`, including `targets` in a row.
    fn harness(title: &str, targets: &[&str]) -> String {
        let includes: String = targets
            .iter()
            .enumerate()
            .map(|(k, t)| format!("    include {t} at ({}, 0);\n", k * 20))
            .collect();
        format!("view harness \"{title}\" {{\n    grid: 20;\n{includes}}}\n")
    }

    #[test]
    fn a_conductor_no_harness_view_draws_is_reported_on_its_wire() {
        let src = harnessed(
            r#"    wire red 1 "SIG" [a.sig, b.sig];
    wire black 1 [a.ret, c.ret];"#,
            &harness("AB", &["a.j", "b.j"]),
        );
        assert_eq!(
            summary(&src),
            [code("undrawn_conductor", &["wire black 1 [a.ret, c.ret];"])]
        );
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert_eq!(
            problem.to_string(),
            "wire from `a.ret` to `c.ret` is in no harness view of `Sys`"
        );
        assert_eq!(problem.severity(), Some(miette::Severity::Warning));
        assert_eq!(
            problem.help().map(|h| h.to_string()).as_deref(),
            Some(
                "a harness view draws a conductor only when it includes the connectors at both ends; include `a.j` and `c.j` in one harness view of `Sys`"
            )
        );
    }

    #[test]
    fn a_cable_conductor_is_named_by_its_cable_and_label() {
        let src = harnessed(
            r#"    cable w1 "Loom" {
        wire red 1 "SIG" [a.sig, c.sig];
    }"#,
            &harness("AB", &["a.j", "b.j"]),
        );
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert_eq!(
            problem.to_string(),
            "cable `w1` conductor \"SIG\" from `a.sig` to `c.sig` is in no harness view of `Sys`"
        );
        assert_eq!(
            summary(&src),
            [code(
                "undrawn_conductor",
                &[r#"wire red 1 "SIG" [a.sig, c.sig];"#]
            )]
        );
    }

    #[test]
    fn a_conductor_drawn_by_any_harness_view_of_its_component_is_covered() {
        let src = harnessed(
            r#"    wire red 1 [a.sig, b.sig];
    wire black 1 [a.ret, c.ret];"#,
            &format!(
                "{}{}",
                harness("AB", &["a.j", "b.j"]),
                harness("AC", &["a.j", "c.j"])
            ),
        );
        assert!(summary(&src).is_empty());
    }

    #[test]
    fn components_without_a_harness_view_are_not_reported() {
        let src = harnessed(
            "    wire red 1 [a.sig, b.sig];",
            "view schematic \"S\" {\n    grid: 20;\n    include a at (0, 0);\n}\n",
        );
        assert!(summary(&src).is_empty());
    }

    #[test]
    fn a_rail_is_judged_per_chain_segment() {
        // `a–b` is drawn, `b–c` is not.
        let src = harnessed(
            "    wire black 1 [a.ret, b.ret, c.ret];",
            &harness("AB", &["a.j", "b.j"]),
        );
        let (project, design) = load(&src);
        let messages: Vec<String> = layout_problems(&design, &project)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            messages,
            ["wire from `b.ret` to `c.ret` is in no harness view of `Sys`"]
        );
    }

    #[test]
    fn segments_off_a_connector_or_within_one_are_not_conductors() {
        // An own port, a connectorless port, and a jumper inside `c.j`:
        // no harness view can draw any of them.
        let src = harnessed(
            r#"    wire red 1 [out, c.sig];
    wire red 1 [a.bare, c.sig];
    wire red 1 [c.sig, c.ret];"#,
            &harness("AB", &["a.j", "b.j"]),
        );
        assert!(summary(&src).is_empty());
    }

    #[test]
    fn two_drawn_conductors_on_one_pin_are_fine() {
        // A double crimp on `a.sig`: both conductors are drawn.
        let src = harnessed(
            r#"    wire red 1 [a.sig, b.sig];
    wire red 1 [a.sig, c.sig];"#,
            &harness("ABC", &["a.j", "b.j", "c.j"]),
        );
        assert!(summary(&src).is_empty());
    }

    #[test]
    fn a_harness_view_with_a_bad_grid_still_covers_its_conductors() {
        // The grid is reported; its conductors are not piled on.
        let src = harnessed(
            "    wire red 1 [a.sig, b.sig];",
            &harness("AB", &["a.j", "b.j"]).replace("grid: 20;", "grid: 0;"),
        );
        assert_eq!(summary(&src), [code("invalid_grid", &["0"])]);
    }

    #[test]
    fn a_non_positive_grid_is_an_error_on_its_number() {
        let src = two_nodes((16, 0), "east", "west").replace("grid: 20;", "grid: -5;");
        assert_eq!(summary(&src), [code("invalid_grid", &["-5"])]);
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert_eq!(problem.to_string(), "grid step must be positive, got -5");
        assert!(problem.is_error());
    }

    #[test]
    fn a_schematic_grid_too_fine_for_a_port_label_names_the_minimum() {
        // Ports sit two steps apart and need `MIN_PORT_PITCH` (15) each.
        let src = two_nodes((16, 0), "east", "west").replace("grid: 20;", "grid: 5;");
        assert_eq!(summary(&src), [code("grid_too_fine", &["5"])]);
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert_eq!(
            problem.to_string(),
            "grid step 5 is too fine for a schematic view; ports need at least 7.5 per step"
        );
        assert!(problem.is_error());
        assert!(
            problem
                .help()
                .expect("help")
                .to_string()
                .contains("use `grid: 7.5;` or coarser")
        );
    }

    /// The lint asks each renderer's own grid validation, so for every view
    /// kind a grid is reported exactly when rendering rejects it — and only
    /// a schematic has a minimum.
    #[test]
    fn a_grid_is_reported_exactly_when_its_renderer_rejects_it() {
        let pinout = r#"
component Sys {
    connector x1 "X1 1p" {
        pub port a "A" pin 1;
    }
}

view pinout "P" {
    grid: 20;
    include x1 at (0, 0);
}
"#;
        let sources = [
            ("schematic", two_nodes((30, 0), "east", "west")),
            (
                "harness",
                harnessed(
                    "    wire red 1 [a.sig, b.sig];",
                    &harness("AB", &["a.j", "b.j"]),
                ),
            ),
            ("pinout", pinout.to_string()),
        ];
        for (kind, src) in &sources {
            for grid in ["-5", "0", "0.5", "5", "7", "7.5", "20"] {
                let src = src.replace("grid: 20;", &format!("grid: {grid};"));
                let (project, design) = load(&src);
                let grid_problems: Vec<_> = layout_problems(&design, &project)
                    .into_iter()
                    .filter(|p| {
                        matches!(p, Problem::InvalidGrid { .. } | Problem::GridTooFine { .. })
                    })
                    .collect();
                assert_eq!(
                    grid_problems.is_empty(),
                    renders(&design),
                    "{kind} view on grid {grid}: {grid_problems:?}"
                );
                let step: f64 = grid.parse().expect("a number");
                let minimum = crate::render::schematic::MIN_PORT_PITCH / 2.0;
                let rejected = step <= 0.0 || (*kind == "schematic" && step < minimum);
                assert_eq!(!grid_problems.is_empty(), rejected, "{kind} on {grid}");
            }
        }
    }

    /// `ic` is mated mid-harness: `a` plugs into its female half, `b` into
    /// its male half.
    const INLINE: &str = r#"
connector_type Plug "Plug 1p" {
    part: "P1";
}

connector_type Socket "Socket 1p" {
    part: "S1";
}

component Sys {
    a: Node;
    b: Node;

    inline ic "Branch" {
        male: Plug;
        female: Socket;
        port sig "SIG" pin 1;
    }

    cable main "Main" {
        wire red 1 "SIG" [a.sig, ic.sig];
    }
    cable branch "Branch" {
        wire red 1 "SIG" [ic.sig, b.sig];
    }

    component Node {
        connector j "J 1p" {
            pub port sig "SIG" pin 1;
        }
    }
}
"#;

    #[test]
    fn an_inline_connector_covers_through_either_half() {
        let views = format!(
            "{}{}",
            harness("Main", &["a.j", "ic.female"]),
            harness("Branch", &["ic.male", "b.j"])
        );
        assert!(summary(&format!("{INLINE}{views}")).is_empty());

        // Without the branch loom's view, its conductor is reported, and
        // the help names both halves.
        let src = format!("{INLINE}{}", harness("Main", &["a.j", "ic.female"]));
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert_eq!(
            problem.to_string(),
            "cable `branch` conductor \"SIG\" from `ic.sig` to `b.sig` is in no harness view of `Sys`"
        );
        let help = problem.help().expect("help").to_string();
        assert!(
            help.ends_with(
                "include `ic.male` (or `ic.female`) and `b.j` in one harness view of `Sys`"
            ),
            "{help}"
        );
    }

    /// The help for a conductor of `INLINE` with the inline's `halves`
    /// lines replaced by `halves`, under a view of `a.j` and `b.j` only.
    fn inline_help(halves: &str) -> String {
        let src = INLINE.replace("        male: Plug;\n        female: Socket;\n", halves);
        let src = format!("{src}{}", harness("Ends", &["a.j", "b.j"]));
        let (project, design) = load(&src);
        let problems = layout_problems(&design, &project);
        let [problem, _] = problems.as_slice() else {
            panic!("both conductors are undrawn: {problems:?}");
        };
        problem.help().expect("help").to_string()
    }

    #[test]
    fn the_help_names_only_the_halves_an_inline_declares() {
        assert!(
            inline_help("        female: Socket;\n")
                .ends_with("include `a.j` and `ic.female` in one harness view of `Sys`")
        );
        assert!(
            inline_help("        male: Plug;\n")
                .ends_with("include `a.j` and `ic.male` in one harness view of `Sys`")
        );
    }

    /// Including an undeclared half is itself an error, so the help must
    /// not suggest one.
    #[test]
    fn an_inline_without_halves_must_declare_one_before_it_is_included() {
        let help = inline_help("");
        assert_eq!(
            help,
            "a harness view draws a conductor only when it includes the connectors at both ends, and inline connector `ic` must declare a `male` or `female` half before a harness view can include it"
        );
    }

    /// `a` and `b` each run a conductor to splice `sp` in `Sys`, under
    /// `views`.
    fn spliced(views: &str) -> String {
        harnessed(
            r#"    splice sp "Joint";
    wire black 1 [a.sig, sp];
    wire black 1 [sp, b.sig];"#,
            views,
        )
    }

    #[test]
    fn a_conductor_to_a_splice_is_drawn_when_the_view_includes_both_its_ends() {
        assert!(summary(&spliced(&harness("AB", &["a.j", "sp", "b.j"]))).is_empty());

        // Without the splice, neither conductor is drawn; the help names
        // the splice as an include.
        let src = spliced(&harness("AB", &["a.j", "b.j"]));
        let (project, design) = load(&src);
        let problems = layout_problems(&design, &project);
        let messages: Vec<String> = problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            [
                "wire from `a.sig` to `sp` is in no harness view of `Sys`",
                "wire from `sp` to `b.sig` is in no harness view of `Sys`"
            ]
        );
        assert_eq!(
            problems[0].help().map(|h| h.to_string()).as_deref(),
            Some(
                "a harness view draws a conductor only when it includes what both its ends land on, connector or splice; include `a.j` and `sp` in one harness view of `Sys`"
            )
        );
    }

    #[test]
    fn a_splice_overlapping_a_connector_is_reported() {
        let views = "view harness \"H\" {\n    grid: 20;\n    include a.j at (0, 0);\n    include sp at (1, 0);\n    include b.j at (20, 0);\n}\n";
        assert_eq!(
            summary(&spliced(views)),
            [code("overlapping_includes", &["sp", "a.j"])]
        );
    }

    #[test]
    fn overlapping_pinout_tables_are_reported() {
        let src = r#"
component Sys {
    connector x1 "X1 1p" {
        pub port a "A" pin 1;
    }
    connector x2 "X2 1p" {
        pub port b "B" pin 1;
    }
    connector x3 "X3 1p" {
        pub port c "C" pin 1;
    }
}

view pinout "P" {
    grid: 20;
    include x1 at (0, 0);
    include x2 at (2, 1);
    include x3 at (20, 0);
}
"#;
        assert_eq!(summary(src), [code("overlapping_includes", &["x2", "x1"])]);
    }

    /// A view of each kind on `grid`, with its second include at `at` and
    /// its first at the origin: schematic `a`/`b`, harness `a.j`/`b.j`,
    /// pinout `x1`/`x2`.
    fn spread(kind: &str, grid: u32, at: (i64, i64)) -> String {
        let (x, y) = at;
        let src = match kind {
            "schematic" => two_nodes((0, 0), "east", "west")
                .replace("include b at (0, 0)", &format!("include b at ({x}, {y})")),
            "harness" => harnessed(
                "    wire red 1 [a.sig, b.sig];",
                &format!(
                    "view harness \"V\" {{\n    grid: 20;\n    include a.j at (0, 0);\n    include b.j at ({x}, {y});\n}}\n"
                ),
            ),
            _ => format!(
                r#"
component Sys {{
    connector x1 "X1 1p" {{
        pub port a "A" pin 1;
    }}
    connector x2 "X2 1p" {{
        pub port b "B" pin 1;
    }}
}}

view pinout "V" {{
    grid: 20;
    include x1 at (0, 0);
    include x2 at ({x}, {y});
}}
"#
            ),
        };
        src.replace("grid: 20;", &format!("grid: {grid};"))
    }

    #[test]
    fn a_view_too_large_to_rasterise_names_its_size_on_the_title_and_extremes() {
        let src = spread("schematic", 20, (100_000, 0));
        assert_eq!(
            summary(&src),
            [code("view_too_large", &["\"V\"", "a", "b"])]
        );
        let (project, design) = load(&src);
        let [problem] = layout_problems(&design, &project)
            .try_into()
            .expect("one problem");
        assert!(problem.is_error());
        assert_eq!(
            problem.to_string(),
            "view \"V\" is 2000256 × 218 units, more than the 8192 a view may span on a side"
        );
    }

    #[test]
    fn the_extremes_are_taken_along_the_longer_side() {
        // `b` is far out to the south; `c`, beside `a`, is the box that
        // reaches farthest east but plays no part in the height.
        let src = spread("pinout", 20, (0, 100_000)).replace(
            "    connector x2",
            "    connector x3 \"X3 1p\" {\n        pub port c \"C\" pin 1;\n    }\n    connector x2",
        );
        let src = src.replace(
            "    include x2",
            "    include x3 at (20, 0);\n    include x2",
        );
        assert_eq!(
            summary(&src),
            [code("view_too_large", &["\"V\"", "x1", "x2"])]
        );
    }

    /// The lint and `--png` measure one canvas and compare it with one
    /// limit, so for every view kind, along either side, and over the
    /// placements on either side of the limit (a schematic on grid 16 with
    /// `b` at 496 steps is exactly at it), the lint fires exactly when the
    /// rasteriser refuses the view. Only the refusal is asked for, so no
    /// pixmap is allocated.
    #[test]
    fn a_view_is_too_large_exactly_when_png_refuses_it() {
        let rasterizer = crate::render::png::PngRasterizer::new();
        for kind in ["schematic", "harness", "pinout"] {
            for grid in [16, 20] {
                let edge = (MAX_CANVAS / f64::from(grid)) as i64;
                for axis in ["x", "y"] {
                    let mut outcomes = Vec::new();
                    for along in edge - 25..=edge - 7 {
                        let at = if axis == "x" { (along, 0) } else { (0, along) };
                        let (project, design) = load(&spread(kind, grid, at));
                        let too_large = layout_problems(&design, &project)
                            .iter()
                            .any(|p| matches!(p, Problem::ViewTooLarge { .. }));
                        let views = render_views(&design, SvgMode::Standalone);
                        let [Ok(view)] = views.as_slice() else {
                            panic!("{kind} at {at:?} on grid {grid} renders: {views:?}");
                        };
                        let refused = rasterizer.admitted(&view.svg).is_err();
                        assert_eq!(too_large, refused, "{kind} at {at:?} on grid {grid}");
                        outcomes.push(refused);
                    }
                    assert!(
                        outcomes.contains(&false) && outcomes.contains(&true),
                        "{kind} on grid {grid} along {axis}: the sweep crosses the limit"
                    );
                }
            }
        }
    }
}
