//! End-to-end check of the `off_view_project` fixture: an ECU whose listed
//! ports are wired to devices its views leave out, one case per port and a
//! port on every side, drawn at the default and at the minimum grid. Each
//! port whose wire the view does not draw names where it goes.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::render::{RenderedView, SvgMode, embed_stylesheet, render_views};

fn views(mode: SvgMode) -> Vec<RenderedView> {
    views_of("off_view_project", mode)
}

/// Every view of fixture `project`, which must check clean: with all its
/// references drawn, not one layout problem.
fn views_of(project: &str, mode: SvgMode) -> Vec<RenderedView> {
    let main = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(project)
        .join("main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    render_views(&report.design.expect("a design"), mode)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders")
}

fn view(views: &[RenderedView], title: &str) -> String {
    views
        .iter()
        .find(|v| v.title == title)
        .unwrap_or_else(|| panic!("view {title:?}"))
        .svg
        .clone()
}

/// One off-view reference as drawn: the port it stands beside
/// (`instance.port`), its text (empty for a stub alone) and its tooltip.
#[derive(Debug, PartialEq)]
struct Reference {
    port: String,
    text: String,
    title: String,
}

/// Every off-view reference of `svg`, in draw order.
fn references(svg: &str) -> Vec<Reference> {
    let attr = |line: &str, name: &str| {
        line.split(&format!(" {name}=\""))
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .map(str::to_string)
    };
    let lines: Vec<&str> = svg.lines().collect();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("<g class=\"off-view\""))
        .map(|(i, line)| {
            let port = attr(line, "data-port").expect("a port");
            let port = match attr(line, "data-component") {
                Some(owner) => format!("{owner}.{port}"),
                None => port,
            };
            let title = lines[i + 1]
                .strip_prefix("<title>")
                .and_then(|t| t.strip_suffix("</title>"))
                .expect("a tooltip")
                .to_string();
            // A stub with no room for text has its tooltip alone.
            let text = if lines[i + 2].starts_with("<text class=\"off-view-label\"") {
                lines[i + 3].to_string()
            } else {
                String::new()
            };
            Reference { port, text, title }
        })
        .collect()
}

fn reference(port: &str, text: &str, title: &str) -> Reference {
    Reference {
        port: port.to_string(),
        text: text.to_string(),
        title: title.to_string(),
    }
}

/// What the ECU's ports show, on either grid: west, east, north, then
/// south.
fn expected() -> Vec<Reference> {
    vec![
        // A rail on to four ends: what fits, then how many more.
        reference(
            "ecu.gnd",
            "lamp.gnd, horn.gnd, +2 more ←",
            "lamp.gnd, horn.gnd, pump.gnd, gauge.gnd",
        ),
        // One end, on an instance the view does not include.
        reference("ecu.lamp", "→ lamp.pwr", "lamp.pwr"),
        // One wire drawn to the pump, the other named.
        reference("ecu.can_h", "→ gauge.can_h", "gauge.can_h"),
        // The subject's own port, not placed on an enclosure.
        reference("ecu.wake", "→ out (leaves Vehicle)", "out (leaves Vehicle)"),
        // Through a splice to two devices left out.
        reference(
            "ecu.sense",
            "→ sp_sense (lamp.sense, +1 more)",
            "sp_sense (lamp.sense, horn.sense)",
        ),
    ]
}

#[test]
fn every_listed_port_with_an_undrawn_wire_names_where_it_goes() {
    let views = views(SvgMode::Standalone);
    for title in ["Partial", "Partial, minimum grid"] {
        // `ecu.spare` has no wire and `pump.can_h` only a drawn one: both
        // stay bare.
        assert_eq!(references(&view(&views, title)), expected(), "{title}");
    }
}

#[test]
fn a_port_keeps_its_drawn_wire_beside_its_reference() {
    let views = views(SvgMode::Standalone);
    for title in ["Partial", "Partial, minimum grid"] {
        let svg = view(&views, title);
        // The CAN wire to the pump is drawn; the five references each draw
        // a stub, in a wire's layers.
        assert_eq!(
            svg.matches("<polyline class=\"wire\" data-color=\"white/blue\"")
                .count(),
            1,
            "{title}"
        );
        assert_eq!(
            svg.matches("class=\"wire off-view-stub\"").count(),
            5,
            "{title}"
        );
        assert_eq!(
            svg.matches("class=\"wire-casing off-view-stub\"").count(),
            5,
            "{title}"
        );
    }
}

#[test]
fn embed_output_carries_the_references_and_the_starter_styles_them() {
    let views = views(SvgMode::Embed);
    assert_eq!(references(&view(&views, "Partial")), expected());
    assert!(embed_stylesheet().contains(".wirebug-schematic .off-view-label {"));
}

/// `off_view_limits_project`: references where room runs out.
fn limits() -> Vec<RenderedView> {
    views_of("off_view_limits_project", SvgMode::Standalone)
}

/// The width of `svg`'s root viewBox, in world units.
fn viewbox_width(svg: &str) -> f64 {
    let viewbox = &svg[svg.find("viewBox=\"").expect("a viewBox") + "viewBox=\"".len()..];
    let viewbox = &viewbox[..viewbox.find('"').expect("a closing quote")];
    viewbox
        .split(' ')
        .nth(2)
        .and_then(|width| width.parse().ok())
        .expect("a numeric width")
}

#[test]
fn a_reference_never_takes_a_view_past_the_canvas_limit() {
    // The view is nearly as wide as a canvas may be: the reference gives
    // way to its arrow alone, which the margin holds, and `check` (in
    // `views_of`) stays clean, as it was before references were drawn.
    let views = limits();
    let near = views
        .iter()
        .find(|v| v.title == "Near the canvas limit")
        .expect("the view");
    let width = viewbox_width(&near.svg);
    assert!(width <= 8192.0, "{width}");
    assert_eq!(
        references(&near.svg),
        [reference("right.out", "→", "far.some_long_port_name")]
    );
}

#[test]
fn a_view_crowded_with_references_has_no_layout_problem() {
    // Sixteen ports with an undrawn wire among eight crossing wires:
    // `check` reports nothing, and every reference that is drawn gives way
    // rather than cover a wire, so none reads as more than its far end.
    let views = limits();
    let crossed = view(&views, "Crossed");
    let drawn = references(&crossed);
    assert!(!drawn.is_empty());
    for reference in &drawn {
        assert!(
            reference.text.is_empty()
                || ["→", "←"].contains(&reference.text.as_str())
                || reference.text.contains(&reference.title),
            "{reference:?}"
        );
    }
}

#[test]
fn an_own_port_off_the_enclosure_keeps_its_mark_past_the_outline() {
    let views = limits();
    assert_eq!(
        references(&view(&views, "Enclosed")),
        [reference(
            "dev.q",
            "→ feed (leaves Vehicle)",
            "feed (leaves Vehicle)"
        )]
    );
}

#[test]
fn a_reference_beside_its_wire_stays_with_its_own_port() {
    // `c.p0` and `c.p2` run straight across to `d` and on to `gone`;
    // `c.p1` and `c.p3` have no wire. Their texts go beside the drawn
    // wire, at the end of a leader, only while that keeps them nearer
    // their own port than the unwired one below: in full at the default
    // grid, an arrow at grid 10, and not at all at the finest grid.
    let views = limits();
    let at = |grid: &str| references(&view(&views, &format!("Neighbours, grid {grid}")));
    assert_eq!(
        at("15"),
        [
            reference("c.p0", "→ gone.p0", "gone.p0"),
            reference("c.p2", "→ gone.p2", "gone.p2")
        ]
    );
    assert_eq!(
        at("10"),
        [
            reference("c.p0", "→", "gone.p0"),
            reference("c.p2", "→", "gone.p2")
        ]
    );
    assert!(at("7.5").is_empty());
}

#[test]
fn a_short_drawn_wire_keeps_its_port_s_reference() {
    // The BMS's CAN pair runs straight to the lamp close by, and on to a
    // lamp the views leave out; its long pin numbers (`signals:11`) and
    // the drawn wires' own codes leave each reference its text, beside
    // the drawn wire, across and under a long title alike.
    let views = limits();
    for title in ["Short wire", "Under a long title"] {
        assert_eq!(
            references(&view(&views, title)),
            [
                reference("bms.can_h", "→ farl.pwr", "farl.pwr"),
                reference("bms.can_l", "→ farl.gnd", "farl.gnd")
            ],
            "{title}"
        );
    }
}
