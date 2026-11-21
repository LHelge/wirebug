//! Rectangle-based SVG schematic renderer.
//!
//! Each included instance becomes a box with its ports on the sides the
//! view authors for them, and every wire segment becomes an orthogonal
//! polyline between two ports.
//!
//! - `layout` turns a design's view into positioned boxes and ports.
//! - `off_view` lays out the reference beside a port whose wires the view
//!   does not draw, and settles the routing around those references.
//! - `draw` emits the SVG for those boxes, ports, and wires.
//! - `route` finds object-avoiding orthogonal paths for the wires.

mod draw;
pub(super) mod layout;
mod off_view;
mod pin;
mod route;

use svg::Document;
use svg::node::element::{Group, Style, Text};

use crate::dsl::ir::{Design, Instance, InstanceName, View};
use crate::error::{ViewErrorKind, ViewResult};
use crate::render::geometry::Point;
use crate::render::lint::{CrowdedPort, IncludeBox, UnroutableWire, ViewLayout, WireEndAt};
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};
use crate::render::{Canvas, Drawing, SvgMode};

use layout::{Connection, Grid, PlacedPort, Placement, ViewBox};
use off_view::{Drawn, OffView};
use route::Router;

/// Grid step (world units) used when a view doesn't specify one. Ports
/// sit two steps apart, so the default port pitch is twice this.
pub(super) const DEFAULT_GRID: f64 = 15.0;

pub(super) const MIN_WIDTH: f64 = 160.0;
pub(super) const MIN_HEIGHT: f64 = 100.0;
/// Tightest spacing between adjacent ports at which their labels and pin
/// numbers still clear each other (labels are 11px, pins 10px). Ports sit
/// two grid steps apart, so the grid step must be at least half this; a
/// finer grid errors rather than overlapping labels. It holds for a pin
/// annotation of any length (`hv:2,3`), because only an annotation's
/// height ever faces the next port: one that would reach the next port on
/// a north or south side turns to run along its stub (`pin::PinAnnotation`).
pub(super) const MIN_PORT_PITCH: f64 = 15.0;
pub(super) const LABEL_INSET: f64 = 10.0;
/// How far a pin annotation keeps from its own port along the stub, and
/// from its own stub when it reads across a north or south side.
pub(super) const PIN_INSET: f64 = 6.0;
/// The least room between a level north or south pin annotation and the
/// next port's stub: half the 4-unit `wire-casing` stroke, plus one.
pub(super) const PIN_NEIGHBOUR_GAP: f64 = 3.0;
/// How far a pin annotation turned along a north or south stub sits beside
/// it, centre of the text to the stub — the band a level west or east
/// annotation keeps above its stub.
pub(super) const PIN_ACROSS: f64 = 8.0;
/// Estimated advance of one character of the 10px pin font.
pub(super) const PIN_CHAR_WIDTH: f64 = 6.0;
/// The room between a north pin annotation moved past the component title
/// and the title.
pub(super) const PIN_TITLE_GAP: f64 = 2.0;
/// The least room between a pin annotation and the canvas edge.
pub(super) const PIN_CANVAS_PAD: f64 = 12.0;
pub(super) const PORT_RADIUS: f64 = 4.0;
pub(super) const CHAR_WIDTH: f64 = 7.5;
pub(super) const COMPONENT_TITLE_GAP: f64 = 8.0;
pub(super) const SVG_MARGIN: f64 = 48.0;
pub(super) const TITLE_GAP: f64 = 12.0;
pub(super) const TEXT_BOX_MIN_WIDTH: f64 = 80.0;
pub(super) const TEXT_BOX_HEIGHT: f64 = 34.0;
pub(super) const TEXT_BOX_PAD_X: f64 = 12.0;

pub(super) const STYLE: &str = "\
.component rect { fill: white; stroke: black; stroke-width: 1.5; }
.component-label { font: bold 13px sans-serif; text-anchor: middle; }
.port circle { fill: black; }
.port-label { font: 11px sans-serif; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.port-pin { font: italic 10px sans-serif; fill: #555; }
.wire-casing { fill: none; stroke: black; stroke-width: 4; stroke-linecap: butt; }
.wire { fill: none; stroke-width: 2; }
.wire-tracer { fill: none; stroke-width: 2; stroke-dasharray: 4 4; }
.wire-code { font: italic 9px sans-serif; fill: #555; text-anchor: middle; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.off-view-label { font: 10px sans-serif; fill: #333; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.enclosure rect { fill: none; stroke: #888; stroke-width: 1.5; stroke-dasharray: 6 4; }
.enclosure-label { font: bold 13px sans-serif; text-anchor: middle; fill: #555; }
.text-box rect { fill: #fff9d6; stroke: #8a7a2f; stroke-width: 1.25; }
.text-box text { font: 12px sans-serif; fill: #2f2a13; text-anchor: middle; dominant-baseline: central; }
.title { font: bold 14px sans-serif; }
.stamp { font: 10px sans-serif; fill: #666; text-anchor: end; }\
";

/// SVG renderer for `kind: schematic` views.
#[derive(Default)]
pub struct SchematicRenderer;

impl SchematicRenderer {
    /// Render `view` (documenting `subject`) against `design` to an SVG
    /// drawing. Wire segments are routed against the placed boxes.
    ///
    /// `mode` picks the presentation: [`SvgMode::Embed`] drops the
    /// built-in `<style>` (the host owns the look), suppresses the
    /// bottom-right project-identity stamp, and class-tags the root
    /// `<svg>` `wirebug wirebug-schematic`; [`SvgMode::Plain`] keeps the
    /// styles but omits the view title and the stamp (the PDF page
    /// header/footer carry them instead).
    pub(super) fn render(
        &self,
        design: &Design,
        subject: &Instance,
        view: &View,
        mode: SvgMode,
    ) -> ViewResult<Drawing> {
        let grid = Grid::of_view(view)?;
        let placement = Placement::compute(design, subject, view, grid)?;

        // Route before sizing the canvas: wires can detour outside the
        // component bounds, so the viewBox has to enclose them too. The
        // off-view references are fitted round what is drawn, after.
        let router = Router::build(&placement, grid.step());
        let connections = placement.connections();
        let wires = route(&router, &connections, grid)?;
        let codes = draw::code_anchors(&wires);
        let references = off_views(design, view, &placement, &connections, &wires, &codes, grid);

        let mut doc = Document::new().set("xmlns", "http://www.w3.org/2000/svg");
        if mode.is_embed() {
            doc = doc.set("class", "wirebug wirebug-schematic");
        } else {
            doc = doc.add(Style::new(STYLE));
        }

        let has_title = mode.shows_title(view);
        let viewbox = viewbox(design, view, &placement, &wires, &references, mode);
        let manifest = mode.stamp(design);
        let canvas = Canvas::new(viewbox.width, viewbox.height);
        doc = mode.frame(doc, Point::new(viewbox.x, viewbox.y), canvas);

        if has_title {
            doc = doc.add(
                Text::new(view.title.clone())
                    .set("class", "title")
                    .set("x", viewbox.x + SVG_MARGIN)
                    .set("y", viewbox.y + SVG_MARGIN - TITLE_GAP),
            );
        }

        if let Some(manifest) = manifest {
            doc = doc.add(stamp_element(
                manifest,
                viewbox.x + viewbox.width - STAMP_INSET,
                viewbox.y + viewbox.height - STAMP_INSET,
            ));
        }

        // The enclosure is the subject's boundary; draw it behind the
        // components so its dashed wrapper reads as a backdrop.
        if let Some(enclosure) = placement.enclosure() {
            doc = doc.add(draw::render_enclosure(enclosure));
        }

        let mut components_group = Group::new().set("class", "components");
        for (name, pc) in &placement.components {
            components_group = components_group.add(draw::render_component(name, pc));
        }
        doc = doc.add(components_group);

        // One net at a time, in source order: all its casings, then its
        // cores, its tracers and its codes. Within a net no casing lies over
        // a core, so where its wires meet they read as joined; the next
        // net's casing cuts an earlier net's core where they cross, so the
        // two read as passing over each other. An off-view stub belongs to
        // the net of the drawn wires leaving its port, under their cores,
        // so where a drawn wire shares its first run the drawn colour
        // shows; a stub on a port with no drawn wire is a net of its own,
        // drawn before the rest.
        let nets = Connection::nets(&connections);
        let net_of = |reference: &OffView| {
            let end = reference.end();
            nets.iter()
                .position(|net| net.iter().any(|&i| connections[i].ends().contains(&end)))
        };
        let stub_nets: Vec<Option<usize>> = references.iter().map(net_of).collect();
        let stubs_of = |net: Option<usize>| {
            references
                .iter()
                .zip(&stub_nets)
                .filter(move |(_, n)| **n == net)
                .map(|(reference, _)| draw::WireDrawing::off_view_stub(reference))
        };
        let mut wires_group = Group::new().set("class", "wires");
        for stub in stubs_of(None) {
            for layer in [Some(stub.casing()), Some(stub.core()), stub.tracer()]
                .into_iter()
                .flatten()
            {
                wires_group = wires_group.add(layer);
            }
        }
        for (n, net) in nets.iter().enumerate() {
            let stubs: Vec<_> = stubs_of(Some(n)).collect();
            let drawn: Vec<_> = net
                .iter()
                .map(|&i| draw::WireDrawing::new(&wires[i], connections[i].color))
                .collect();
            for wire in drawn.iter().chain(&stubs) {
                wires_group = wires_group.add(wire.casing());
            }
            for wire in stubs.iter().chain(&drawn) {
                wires_group = wires_group.add(wire.core());
            }
            for tracer in stubs
                .iter()
                .chain(&drawn)
                .filter_map(draw::WireDrawing::tracer)
            {
                wires_group = wires_group.add(tracer);
            }
            for &i in net {
                if let Some(anchor) = &codes[i] {
                    wires_group =
                        wires_group.add(draw::render_wire_code(anchor, connections[i].color));
                }
            }
        }
        doc = doc.add(wires_group);

        // The references' texts go over every wire, though each is fitted
        // clear of them.
        if !references.is_empty() {
            let mut off_views_group = Group::new().set("class", "off-views");
            for reference in &references {
                off_views_group = off_views_group.add(draw::render_off_view(reference));
            }
            doc = doc.add(off_views_group);
        }

        let mut texts_group = Group::new().set("class", "text-boxes");
        for text in &placement.texts {
            texts_group = texts_group.add(draw::render_text_box(text));
        }
        doc = doc.add(texts_group);

        Ok(Drawing::new(doc, canvas))
    }

    /// What `check` needs from `view`'s layout: each include's box, every
    /// wired port whose wire [`Self::render`] could not route because
    /// another box sits inside its clearance, every wire the router finds
    /// no path for, and, when every wire routes, the canvas of the
    /// standalone SVG. Nothing but the fault when the grid is invalid: no
    /// box can be laid out on it.
    pub(super) fn layout(&self, design: &Design, subject: &Instance, view: &View) -> ViewLayout {
        let grid = match Grid::of_view(view) {
            Ok(grid) => grid,
            Err(fault) => return ViewLayout::bad_grid(fault),
        };
        let Ok(placement) = Placement::compute(design, subject, view, grid) else {
            return ViewLayout::default();
        };
        let include_named =
            |name: &InstanceName| view.includes.iter().position(|inc| &inc.instance == name);
        let include_of = |component: usize| {
            let (name, _) = placement.components.get_index(component)?;
            include_named(name)
        };
        let boxes = placement
            .components
            .values()
            .enumerate()
            .filter_map(|(component, pc)| {
                Some(IncludeBox {
                    include: include_of(component)?,
                    bounds: pc.bounds(),
                })
            })
            .collect();
        let router = Router::build(&placement, grid.step());
        let crowded: Vec<_> = router
            .blocked_stubs(&placement)
            .into_iter()
            .filter_map(|blocked| {
                Some(CrowdedPort {
                    include: include_of(blocked.owner)?,
                    port: blocked.port.port.clone(),
                    by: include_of(blocked.blocker)?,
                })
            })
            .collect();
        let end = |owner: Option<&InstanceName>, port: &PlacedPort| WireEndAt {
            include: owner.and_then(include_named),
            port: port.port.clone(),
        };
        let connections = placement.connections();
        let unroutable: Vec<_> = connections
            .iter()
            .filter(|c| !router.connects(c.a, c.b))
            .map(|c| UnroutableWire {
                ends: [end(c.owners[0], c.a), end(c.owners[1], c.b)],
            })
            .collect();
        // The canvas encloses the routed wires, and only a view whose every
        // wire routes is drawn at all.
        let canvas = if crowded.is_empty() && unroutable.is_empty() {
            route(&router, &connections, grid).ok().map(|wires| {
                let codes = draw::code_anchors(&wires);
                let references =
                    off_views(design, view, &placement, &connections, &wires, &codes, grid);
                let vb = viewbox(
                    design,
                    view,
                    &placement,
                    &wires,
                    &references,
                    SvgMode::Standalone,
                );
                Canvas::new(vb.width, vb.height)
            })
        } else {
            None
        };
        ViewLayout {
            boxes,
            crowded,
            unroutable,
            canvas,
            ..ViewLayout::default()
        }
    }
}

/// Route every connection as [`SchematicRenderer::render`] draws it, or
/// name the wire that has no route.
fn route(
    router: &Router,
    connections: &[Connection<'_>],
    grid: Grid,
) -> ViewResult<Vec<Vec<Point>>> {
    let pairs: Vec<_> = connections.iter().map(|c| (c.a, c.b)).collect();
    // Spread parallel wires in a shared channel by the port pitch (two
    // steps), so a nudged bundle matches the spacing of the ports it fans
    // out from rather than packing twice as tight.
    router.route_all(&pairs, grid.pitch()).map_err(|unrouted| {
        let [from, to] = connections[unrouted.pair].ends();
        ViewErrorKind::RouteFailed { from, to }
    })
}

/// The off-view references of `view`, fitted round its routed `wires` and
/// their `codes` ([`Placement::off_views`]), on the standalone SVG's canvas
/// — the largest of the output modes, and the one `check` measures — so
/// every mode draws the same references.
fn off_views(
    design: &Design,
    view: &View,
    placement: &Placement,
    connections: &[Connection<'_>],
    wires: &[Vec<Point>],
    codes: &[Option<draw::CodeAnchor>],
    grid: Grid,
) -> Vec<OffView> {
    let drawn = Drawn {
        connections,
        wires,
        codes,
    };
    placement.off_views(&drawn, grid, |references| {
        let vb = viewbox(
            design,
            view,
            placement,
            wires,
            references,
            SvgMode::Standalone,
        );
        Canvas::new(vb.width, vb.height)
    })
}

/// The viewBox `view` is drawn in under `mode`: `placement`, its routed
/// `wires` and its off-view `references`, with room for the title and the
/// identity stamp when the mode draws them.
fn viewbox(
    design: &Design,
    view: &View,
    placement: &Placement,
    wires: &[Vec<Point>],
    references: &[OffView],
    mode: SvgMode,
) -> ViewBox {
    let mut viewbox = placement.viewbox(mode.shows_title(view), wires, references);
    if mode.stamp(design).is_some() {
        viewbox.height += STAMP_HEIGHT;
    }
    viewbox
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dsl::ir::{
        Include, InstanceName, PortName, Side, TextBox, TypeName, ViewGrid, ViewKind,
    };
    use crate::dsl::span::{FileId, Span};
    use crate::error::ViewErrorKind;
    use crate::render::tests::{assert_sized_by_its_viewbox, assert_unsized};

    /// Elaborate a single-file `.wb` source into a [`Design`]. The source
    /// must have a single top-level component (the views are built
    /// separately with [`view_of`]).
    pub(crate) fn design_from(src: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), src).expect("write");
        let (project, _) = crate::dsl::project::load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = crate::dsl::resolve::resolve(&project);
        let (design, problems) = crate::dsl::elaborate::elaborate(&resolved);
        assert!(problems.is_empty(), "elaboration problems: {problems:?}");
        design.expect("a design")
    }

    /// A schematic view over `subject`, including the named instances at
    /// the given grid coordinates, each with its authored port placements
    /// `(port, side)` in order.
    #[allow(clippy::type_complexity)]
    pub(crate) fn view_of(subject: &str, includes: &[(&str, f64, f64, &[(&str, Side)])]) -> View {
        View {
            kind: ViewKind::Schematic,
            title: "T".to_string(),
            grid: None,
            span: Span::new(FileId(0), 0, 0),
            subject: TypeName::from(subject),
            has_enclosure: false,
            enclosure: Vec::new(),
            includes: includes
                .iter()
                .map(|(name, x, y, ports)| Include {
                    instance: InstanceName::from(*name),
                    connector: None,
                    half: None,
                    x: *x,
                    y: *y,
                    ports: ports
                        .iter()
                        .map(|(p, side)| (PortName::from(*p), *side))
                        .collect(),
                    span: Span::new(FileId(0), 0, 0),
                })
                .collect(),
            texts: Vec::new(),
        }
    }

    fn render(design: &Design, view: &View) -> ViewResult<String> {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        SchematicRenderer
            .render(design, subject, view, SvgMode::Standalone)
            .map(|drawing| drawing.svg)
    }

    fn two_box_design() -> Design {
        design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    wire red 1 [a.p, b.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
}
"#,
        )
    }

    #[test]
    fn render_contains_expected_fragments() {
        let design = two_box_design();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");

        assert!(svg.contains("<svg"));
        assert!(svg.contains("viewBox="));
        assert!(svg.contains("alpha"));
        assert!(svg.contains("beta"));
        assert!(svg.contains("class=\"wire\""));
        assert!(svg.contains("class=\"component\""));
        assert!(svg.contains("class=\"port\""));
    }

    #[test]
    fn wire_carries_its_color_code() {
        let design = two_box_design();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");

        // The `wire red 1` net annotates as its IEC 60757 code and tags
        // its polyline with the authored color for host stylesheets.
        assert!(svg.contains("class=\"wire-code\""));
        assert!(svg.contains(">\nRD\n</text>"));
        assert!(svg.contains("data-color=\"red\""));
    }

    /// One drawn layer of a wire: a `<polyline>` in the `wires` group.
    #[derive(Debug)]
    struct Layer {
        class: String,
        stroke: Option<String>,
        data_color: Option<String>,
        fill: Option<String>,
        points: String,
    }

    /// Every wire layer (casing, core, tracer) of `svg`, in draw order.
    fn wire_layers(svg: &str) -> Vec<Layer> {
        let attr = |element: &str, name: &str| {
            element
                .split(&format!(" {name}=\""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string)
        };
        svg.lines()
            .filter(|line| line.starts_with("<polyline"))
            .map(|line| Layer {
                class: attr(line, "class").expect("a class"),
                stroke: attr(line, "stroke"),
                data_color: attr(line, "data-color"),
                fill: attr(line, "fill"),
                points: attr(line, "points").expect("points"),
            })
            .collect()
    }

    fn one_wire_design(color: &str) -> Design {
        design_from(&format!(
            r#"
component sys {{
    a: alpha;
    b: beta;
    wire {color} 1 [a.p, b.p];
    component alpha {{
        pub port p "Out" pin 1;
    }}
    component beta {{
        pub port p "In" pin 1;
    }}
}}
"#
        ))
    }

    fn one_wire_view() -> View {
        view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        )
    }

    #[test]
    fn wire_is_stroked_in_its_colour_over_a_casing() {
        let svg = render(&one_wire_design("red"), &one_wire_view()).expect("renders");
        let layers = wire_layers(&svg);
        let classes: Vec<&str> = layers.iter().map(|l| l.class.as_str()).collect();
        assert_eq!(classes, ["wire-casing", "wire"]);
        let (casing, core) = (&layers[0], &layers[1]);
        // The casing runs under the whole core and takes its paint from
        // the stylesheet; the core is painted in the wire's colour.
        assert_eq!(casing.points, core.points);
        assert_eq!(casing.stroke, None);
        assert_eq!(core.stroke.as_deref(), Some("red"));
        assert_eq!(core.data_color.as_deref(), Some("red"));
    }

    #[test]
    fn light_wire_is_cased() {
        for color in ["white", "yellow"] {
            let svg = render(&one_wire_design(color), &one_wire_view()).expect("renders");
            let layers = wire_layers(&svg);
            assert_eq!(layers[0].class, "wire-casing", "{color}");
            assert_eq!(layers[1].stroke.as_deref(), Some(color));
        }
        // The casing is drawn dark and wider than the core.
        assert!(STYLE.contains(".wire-casing { fill: none; stroke: black; stroke-width: 4;"));
        assert!(STYLE.contains(".wire { fill: none; stroke-width: 2; }"));
    }

    #[test]
    fn stylesheet_leaves_the_wire_paint_to_the_wire() {
        // A `stroke` in the `.wire` rule would override every wire's
        // colour: a presentation attribute loses to any stylesheet rule.
        let rule = STYLE
            .lines()
            .find(|rule| rule.starts_with(".wire {"))
            .expect("a .wire rule");
        assert!(!rule.contains("stroke:"), "{rule}");
    }

    #[test]
    fn two_tone_wire_draws_its_tracer_over_the_core() {
        let svg = render(&one_wire_design("green/yellow"), &one_wire_view()).expect("renders");
        let layers = wire_layers(&svg);
        let classes: Vec<&str> = layers.iter().map(|l| l.class.as_str()).collect();
        assert_eq!(classes, ["wire-casing", "wire", "wire-tracer"]);
        assert_eq!(layers[1].stroke.as_deref(), Some("green"));
        assert_eq!(layers[2].stroke.as_deref(), Some("yellow"));
        assert_eq!(layers[2].data_color.as_deref(), Some("green/yellow"));
        assert_eq!(layers[2].points, layers[1].points);
        assert!(svg.contains(">\nGN/YE\n</text>"));
    }

    #[test]
    fn crossing_nets_are_drawn_one_after_the_other() {
        // Two wires of different nets that must cross between the columns:
        // `a`–`b` runs down and `c`–`d` up. The later net's casing lies over
        // the earlier one's core where they cross, so the two read as
        // passing over each other.
        let design = design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    c: gamma;
    d: delta;
    wire red 1 [a.p, b.p];
    wire white/blue 1 [c.p, d.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
    component gamma {
        pub port p "Out" pin 1;
    }
    component delta {
        pub port p "In" pin 1;
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 12.0, &[("p", Side::West)]),
                ("c", 0.0, 8.0, &[("p", Side::East)]),
                ("d", 16.0, 4.0, &[("p", Side::West)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");
        let cores: Vec<Vec<Point>> = wire_layers(&svg)
            .iter()
            .filter(|layer| layer.class == "wire")
            .map(|layer| {
                layer
                    .points
                    .split(' ')
                    .map(|pair| {
                        let (x, y) = pair.split_once(',').expect("an x,y pair");
                        Point::new(x.parse().expect("x"), y.parse().expect("y"))
                    })
                    .collect()
            })
            .collect();
        assert!(
            !draw::wire_crossings(&cores)[0].is_empty(),
            "the wires cross: {cores:?}"
        );
        assert_eq!(
            drawn_classes(&svg),
            [
                "wire-casing",
                "wire",
                "wire-code",
                "wire-casing",
                "wire",
                "wire-tracer",
                "wire-code"
            ]
        );
    }

    /// The wire layer and code classes of `svg`, in draw order.
    fn drawn_classes(svg: &str) -> Vec<&'static str> {
        svg.lines()
            .filter_map(|line| {
                ["wire-casing", "wire", "wire-tracer", "wire-code"]
                    .into_iter()
                    .find(|class| line.contains(&format!("class=\"{class}\"")))
            })
            .collect()
    }

    #[test]
    fn wires_meeting_at_a_port_lay_every_casing_before_any_core() {
        // One net three ways: a rail `a`–`b`–`c`, and a two-tone wire of
        // another colour crimped into `b.p` beside it. Its wires all leave
        // `b.p` through the same stub; a casing drawn over a core there
        // would cut the joint like a crossing.
        let design = design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    c: alpha;
    d: alpha;
    wire red 1 [a.p, b.p, c.p];
    wire white/blue 1 [d.p, b.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 8.0, &[("p", Side::West)]),
                ("c", 0.0, 8.0, &[("p", Side::East)]),
                ("d", 0.0, 16.0, &[("p", Side::East)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");
        assert_eq!(
            drawn_classes(&svg),
            [
                "wire-casing",
                "wire-casing",
                "wire-casing",
                "wire",
                "wire",
                "wire",
                "wire-tracer",
                "wire-code",
                "wire-code",
                "wire-code"
            ]
        );
        // Cores stay in source order: the crimped wire's lies on top.
        let cores: Vec<_> = wire_layers(&svg)
            .into_iter()
            .filter(|layer| layer.class == "wire")
            .filter_map(|layer| layer.stroke)
            .collect();
        assert_eq!(cores, ["red", "red", "white"]);
    }

    #[test]
    fn embed_mode_carries_the_colour_on_the_wire() {
        let svg =
            render_embed(&one_wire_design("green/yellow"), &one_wire_view()).expect("renders");
        // The paint is a presentation attribute, not a `style`: a host
        // stylesheet rule on `.wire` still wins over it. Every layer is
        // unfilled by itself, so a host stylesheet without rules for the
        // casing and the tracer never blots the area a bend encloses.
        assert!(!svg.contains("style="));
        let layers = wire_layers(&svg);
        assert_eq!(layers.len(), 3);
        assert!(layers.iter().all(|l| l.fill.as_deref() == Some("none")));
        assert_eq!(layers[1].stroke.as_deref(), Some("green"));
        assert_eq!(layers[1].data_color.as_deref(), Some("green/yellow"));
        assert_eq!(layers[2].stroke.as_deref(), Some("yellow"));
    }

    #[test]
    fn two_tone_wire_annotates_with_combined_code() {
        let design = design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    wire green/yellow 2.5 [a.p, b.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");
        assert!(svg.contains(">\nGN/YE\n</text>"));
        assert!(svg.contains("data-color=\"green/yellow\""));
    }

    #[test]
    fn unknown_wire_color_annotates_verbatim() {
        let design = design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    wire chartreuse 1 [a.p, b.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");
        assert!(svg.contains(">\nchartreuse\n</text>"));
        // Painted verbatim, as the harness paints it.
        assert_eq!(wire_layers(&svg)[1].stroke.as_deref(), Some("chartreuse"));
    }

    #[test]
    fn grid_finer_than_min_port_pitch_errors() {
        let design = two_box_design();
        // Pitch is 2 steps, so a step of 5 gives pitch 10 < MIN_PORT_PITCH.
        let mut view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        view.grid = Some(ViewGrid {
            step: 5.0,
            span: Span::new(FileId(0), 0, 0),
        });
        assert!(matches!(
            render(&design, &view),
            Err(ViewErrorKind::GridTooSmall { .. })
        ));
    }

    #[test]
    fn wire_to_excluded_box_draws_as_an_off_view_reference() {
        let design = two_box_design();
        // Only `a` is included; the wire's `b` end isn't placed, so no wire
        // is drawn, only `a.p`'s stub in the wire's colour and where it goes.
        let view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let svg = render(&design, &view).unwrap();
        assert!(!svg.contains("class=\"wire\""));
        let layers = wire_layers(&svg);
        let classes: Vec<&str> = layers.iter().map(|l| l.class.as_str()).collect();
        assert_eq!(classes, ["wire-casing off-view-stub", "wire off-view-stub"]);
        assert_eq!(layers[1].stroke.as_deref(), Some("red"));
        assert!(svg.contains(
            "<g class=\"off-view\" data-component=\"a\" data-port=\"p\">\n<title>b.p</title>"
        ));
        assert!(svg.contains(">\n→ b.p\n</text>"));
    }

    #[test]
    fn a_view_drawing_every_wire_has_no_off_view_reference() {
        let svg = render(&two_box_design(), &one_wire_view()).expect("renders");
        assert!(!svg.contains("class=\"off-view"), "{svg}");
        assert!(!svg.contains("off-view-stub"), "{svg}");
    }

    #[test]
    fn an_off_view_stub_joins_the_net_of_the_drawn_wire_leaving_its_port() {
        // `a.p` is wired to `b.p`, drawn, and to `c.p`, left out: the stub
        // is drawn with the drawn wire's net, its casing among the net's
        // casings and its core under the drawn wire's, so their shared
        // first run shows the drawn colour.
        let design = design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    c: beta;
    wire red 1 [a.p, b.p];
    wire white/blue 1 [a.p, c.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
}
"#,
        );
        let svg = render(&design, &one_wire_view()).expect("renders");
        let classes: Vec<String> = wire_layers(&svg).into_iter().map(|l| l.class).collect();
        assert_eq!(
            classes,
            [
                "wire-casing",
                "wire-casing off-view-stub",
                "wire off-view-stub",
                "wire",
                "wire-tracer off-view-stub",
            ]
        );
        let stub = &wire_layers(&svg)[2];
        assert_eq!(stub.stroke.as_deref(), Some("white"));
        assert_eq!(stub.data_color.as_deref(), Some("white/blue"));
    }

    #[test]
    fn embed_mode_carries_the_stub_colour_and_starter_styles_its_text() {
        let design = two_box_design();
        let view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let svg = render_embed(&design, &view).expect("renders");
        let core = &wire_layers(&svg)[1];
        assert_eq!(core.stroke.as_deref(), Some("red"));
        assert_eq!(core.data_color.as_deref(), Some("red"));
        assert!(svg.contains("class=\"off-view-label\""));
        assert!(
            STYLE
                .lines()
                .any(|rule| rule.starts_with(".off-view-label {"))
        );
    }

    fn render_embed(design: &Design, view: &View) -> ViewResult<String> {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        SchematicRenderer
            .render(design, subject, view, SvgMode::Embed)
            .map(|drawing| drawing.svg)
    }

    #[test]
    fn embed_mode_omits_embedded_style_block() {
        let design = two_box_design();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render_embed(&design, &view).expect("renders");

        // The built-in <style> tag and its STYLE-block selectors are absent
        // in embed mode; the host stylesheet owns the look.
        assert!(!svg.contains("<style>"));
        assert!(!svg.contains(".component rect"));
    }

    #[test]
    fn embed_mode_class_tags_the_root_svg() {
        let design = two_box_design();
        let view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let svg = render_embed(&design, &view).expect("renders");
        assert!(svg.contains("class=\"wirebug wirebug-schematic\""));
    }

    /// Standalone and plain roots carry their canvas as their intrinsic
    /// size, so a browser shows the view one world unit to the px; an
    /// embed root leaves its size to the host.
    #[test]
    fn only_an_embed_root_has_no_intrinsic_size() {
        let design = two_box_design();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        let svg = |mode| {
            SchematicRenderer
                .render(&design, subject, &view, mode)
                .expect("renders")
                .svg
        };
        assert_sized_by_its_viewbox(&svg(SvgMode::Standalone));
        assert_sized_by_its_viewbox(&svg(SvgMode::Plain));
        assert_unsized(&svg(SvgMode::Embed));
    }

    #[test]
    fn plain_mode_keeps_styles_but_omits_title_and_stamp() {
        let mut design = two_box_design();
        design.manifest = Some(crate::dsl::manifest::Manifest {
            name: "demo".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            authors: Vec::new(),
            license: None,
            revision: None,
            date: None,
        });
        let view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");

        let standalone = SchematicRenderer
            .render(&design, subject, &view, SvgMode::Standalone)
            .expect("renders")
            .svg;
        assert!(standalone.contains("class=\"stamp\""));
        assert!(standalone.contains("class=\"title\""));

        // Plain mode strips the in-drawing furniture — the PDF page
        // header/footer carry the title and identity instead.
        let plain = SchematicRenderer
            .render(&design, subject, &view, SvgMode::Plain)
            .expect("renders")
            .svg;
        assert!(plain.contains("<style>"));
        assert!(!plain.contains("class=\"stamp\""));
        assert!(!plain.contains("class=\"title\""));
    }

    #[test]
    fn renders_text_boxes() {
        let design = design_from("component sys { }");
        let mut view = view_of("sys", &[]);
        view.texts.push(TextBox {
            name: None,
            x: 2.0,
            y: 3.0,
            label: "This is my textbox!".to_string(),
        });

        let svg = render(&design, &view).unwrap();

        assert!(
            svg.contains("<g class=\"text-box\">"),
            "no name, no data-text"
        );
        assert!(svg.contains("This is my textbox!"));
    }

    #[test]
    fn named_text_box_keeps_its_name_as_data_text() {
        let design = design_from("component sys { }");
        let mut view = view_of("sys", &[]);
        view.texts.push(TextBox {
            name: Some("note".to_string()),
            x: 2.0,
            y: 3.0,
            label: "Named".to_string(),
        });

        let svg = render(&design, &view).unwrap();

        assert!(svg.contains("<g class=\"text-box\" data-text=\"note\">"));
        assert!(svg.contains("Named"));
    }
}
