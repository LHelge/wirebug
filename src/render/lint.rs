//! Checks that need a view laid out: problems only visible once the
//! renderers have sized and placed every include's box.
//!
//! The DSL pipeline ends at `ir::Design` and never draws anything, so it
//! cannot tell how big a box is: that follows from ports, labels and pin
//! rows, in each renderer's layout code. These checks ask that same layout
//! code, so a check and the drawing cannot disagree, and they report
//! `Problem` diagnostics pointing at the includes in the source.
//! `pipeline::check_project` runs them after the DSL stages.

use indexmap::IndexMap;

use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::{Design, PortName, View, ViewKind};
use crate::dsl::project::Project;
use crate::render::geometry::Bounds;
use crate::render::harness::HarnessRenderer;
use crate::render::pinout::PinoutRenderer;
use crate::render::schematic::SchematicRenderer;

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

/// What a renderer's layout reports about one view, for checking.
#[derive(Default)]
pub(super) struct ViewLayout {
    pub(super) boxes: Vec<IncludeBox>,
    /// Always empty for kinds whose wires are not routed around boxes, as
    /// is `unroutable`.
    pub(super) crowded: Vec<CrowdedPort>,
    pub(super) unroutable: Vec<UnroutableWire>,
}

/// Lay out every view of `design` as the renderers would and report what
/// the layout shows to be wrong: includes whose boxes overlap
/// (`overlapping_includes`), schematic includes too close for a wire to
/// leave a port (`include_too_close`), and schematic wires walled in by
/// boxes all around (`unroutable_wire`). `project` supplies the sources the
/// include spans point into.
///
/// A view the renderers cannot lay out at all (no subject instance, a bad
/// grid, an unknown kind) is skipped: rendering reports those itself.
#[must_use]
pub fn layout_problems(design: &Design, project: &Project) -> Vec<Problem> {
    let mut problems = Vec::new();
    for view in &design.views {
        let Ok(subject) = super::subject_instance(design, view) else {
            continue;
        };
        let layout = match &view.kind {
            ViewKind::Schematic => SchematicRenderer.layout(design, subject, view),
            ViewKind::Harness => HarnessRenderer.layout(design, subject, view),
            ViewKind::Pinout => PinoutRenderer.layout(subject, view),
            ViewKind::Other(_) => continue,
        };
        problems.extend(layout.problems(view, project));
    }
    problems
}

impl ViewLayout {
    /// Overlapping pairs first, then crowded pairs, each in include order,
    /// then unroutable wires in wire order. A pair that overlaps is not
    /// also reported as too close, and unroutable wires are reported only
    /// in a view with neither: colliding boxes are the cause to fix first,
    /// and a port without room to leave never routes anyway.
    fn problems(&self, view: &View, project: &Project) -> Vec<Problem> {
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
        problems
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
        let (design, problems) = crate::dsl::check_loaded(&project);
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
}
