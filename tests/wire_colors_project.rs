//! End-to-end check of the `wire_colors_project` fixture: light, two-tone
//! and dark wires between two devices, stroked in their colour over a dark
//! casing in a schematic exactly as the harness strokes them.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::collections::BTreeMap;
use std::path::Path;

use wirebug::check_project;
use wirebug::render::{SvgMode, embed_stylesheet, render_views};

fn view(mode: SvgMode, title: &str) -> String {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wire_colors_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    let design = report.design.expect("a design");
    render_views(&design, mode)
        .into_iter()
        .map(|view| view.expect("view renders"))
        .find(|view| view.title == title)
        .unwrap_or_else(|| panic!("view {title:?}"))
        .svg
}

/// The value of attribute `name` on the one-line SVG `element`.
fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    element
        .split(&format!(" {name}=\""))
        .nth(1)
        .and_then(|rest| rest.split('"').next())
}

/// Every element of `svg` whose class is exactly `class`, in draw order.
fn elements<'a>(svg: &'a str, class: &str) -> Vec<&'a str> {
    svg.lines()
        .filter(|line| attr(line, "class") == Some(class))
        .collect()
}

/// Each core's authored colour (`data-color`) and the paint it is stroked
/// with.
fn paints(svg: &str, core_class: &str) -> BTreeMap<String, String> {
    elements(svg, core_class)
        .into_iter()
        .map(|core| {
            (
                attr(core, "data-color").expect("data-color").to_string(),
                attr(core, "stroke").expect("stroke").to_string(),
            )
        })
        .collect()
}

#[test]
fn a_schematic_paints_each_wire_as_the_harness_does() {
    let schematic = paints(&view(SvgMode::Standalone, "Signals"), "wire");
    let harness = paints(&view(SvgMode::Standalone, "Loom"), "cable-wire");
    assert_eq!(schematic, harness);
    assert_eq!(
        schematic.into_iter().collect::<Vec<_>>(),
        [
            ("black".to_string(), "black".to_string()),
            ("green/yellow".to_string(), "green".to_string()),
            ("white".to_string(), "white".to_string()),
            ("yellow".to_string(), "yellow".to_string()),
        ]
    );
}

#[test]
fn every_schematic_wire_lies_on_a_casing_and_keeps_its_code() {
    let svg = view(SvgMode::Standalone, "Signals");
    let lines: Vec<&str> = svg.lines().collect();
    let cores: Vec<usize> = (0..lines.len())
        .filter(|&i| attr(lines[i], "class") == Some("wire"))
        .collect();
    assert_eq!(cores.len(), 4);
    for i in cores {
        // The casing comes straight before its core, along the same points.
        let casing = lines[i - 1];
        assert_eq!(attr(casing, "class"), Some("wire-casing"), "{casing}");
        assert_eq!(attr(casing, "points"), attr(lines[i], "points"));
    }

    let codes: Vec<&str> = svg
        .split("class=\"wire-code\"")
        .skip(1)
        .filter_map(|rest| rest.split(">\n").nth(1)?.split('\n').next())
        .collect();
    assert_eq!(codes, ["WH", "YE", "GN/YE", "BK"]);
}

#[test]
fn a_two_tone_schematic_wire_shows_its_tracer() {
    let svg = view(SvgMode::Standalone, "Signals");
    let tracers = elements(&svg, "wire-tracer");
    assert_eq!(tracers.len(), 1);
    assert_eq!(attr(tracers[0], "stroke"), Some("yellow"));
    assert_eq!(attr(tracers[0], "data-color"), Some("green/yellow"));
}

#[test]
fn an_embedded_schematic_carries_the_colour_on_each_wire() {
    let svg = view(SvgMode::Embed, "Signals");
    assert!(!svg.contains("<style") && !svg.contains("style="));
    for core in elements(&svg, "wire") {
        assert!(attr(core, "stroke").is_some(), "{core}");
        assert!(attr(core, "data-color").is_some(), "{core}");
    }
    // Unfilled by themselves, so a host stylesheet from before they existed
    // draws nothing for them rather than a filled shape.
    for layer in elements(&svg, "wire-casing")
        .into_iter()
        .chain(elements(&svg, "wire-tracer"))
    {
        assert_eq!(attr(layer, "fill"), Some("none"), "{layer}");
    }

    // The starter stylesheet draws the casing and the tracer, and leaves
    // the core's paint to the wire.
    let css = embed_stylesheet();
    for class in ["wire-casing", "wire-tracer"] {
        assert!(
            css.lines()
                .any(|rule| rule.starts_with(&format!(".wirebug-schematic .{class} {{"))),
            "{class}"
        );
    }
    let core = css
        .lines()
        .find(|rule| rule.starts_with(".wirebug-schematic .wire {"))
        .expect("a .wire rule");
    assert!(!core.contains("stroke:"), "{core}");
}
