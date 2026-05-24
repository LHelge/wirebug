//! End-to-end: parse the worked example, validate, render, and check
//! the SVG carries the structural fragments we expect.
//!
//! We deliberately do *not* snapshot the raw SVG string — layout pixels
//! change too easily. Fragment asserts plus a snapshot of the parsed
//! model lock the contract down without locking down the pixels.

use wirebug::{Model, View, render};

const MODEL: &str = "examples/model.yaml";
const VIEW: &str = "examples/views/hv_overview.yaml";

#[test]
fn example_renders_with_expected_fragments() {
    let model = Model::load(MODEL).expect("model parses");
    let view = View::load(VIEW).expect("view parses");

    let report = model.validate().expect("model validates");
    // Two warnings: contactor.coil.{pos,neg} are intentionally dangling.
    assert_eq!(report.warnings.len(), 2);

    let view_report = view.validate(&model).expect("view validates");
    assert!(view_report.is_empty());

    let svg = render::render(&model, &view).expect("renders");

    assert!(svg.contains("<svg"));
    assert!(svg.contains("viewBox="));
    assert!(svg.contains("HV Power Path"));

    // Component labels
    for label in [
        "400 V Battery",
        "Main Contactor",
        "EM57 Inverter",
        "EM57 Motor",
    ] {
        assert!(svg.contains(label), "expected SVG to mention {label:?}");
    }

    assert_eq!(svg.matches("class=\"component\"").count(), 4);
    assert_eq!(svg.matches("class=\"wire\"").count(), 12);
    assert_eq!(svg.matches("class=\"port\"").count(), 26);

    // Every placed port carries a pin label (every port in the example
    // has a pin number).
    assert_eq!(svg.matches("class=\"port-pin\"").count(), 26);

    // The SVG emitter wraps text content across lines, so collapse
    // inter-tag whitespace before asserting on specific pin labels.
    let normalised: String = svg.lines().map(str::trim).collect();
    for pin in [">B+<", ">B-<", ">A1<", ">A2<", ">U<", ">V<", ">W<"] {
        assert!(normalised.contains(pin), "expected pin marker {pin:?}");
    }
}

/// Every routed wire is rectilinear: consecutive points share an x or a
/// y, so the polyline contains only right-angle bends. This holds
/// end-to-end through the renderer, across all 12 example connections.
#[test]
fn example_wires_are_orthogonal() {
    let svg = wirebug::render_paths(MODEL, VIEW).expect("renders").svg;

    let mut wires = 0;
    for points in polyline_points(&svg) {
        assert!(points.len() >= 2, "wire with too few points: {points:?}");
        for seg in points.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let orthogonal = (a.0 - b.0).abs() < 1e-6 || (a.1 - b.1).abs() < 1e-6;
            assert!(orthogonal, "diagonal wire segment {a:?} -> {b:?}");
        }
        wires += 1;
    }
    assert_eq!(wires, 12, "expected one polyline per connection");
}

/// Nudging (paper §6): wires that would share a channel are pulled apart.
/// The 6-wire resolver bundle between inverter and motor is the stress
/// case — before nudging all six collapsed onto one line. No two wires
/// from different connectors may share an overlapping collinear segment.
#[test]
fn example_wires_do_not_overlap_after_nudging() {
    let svg = wirebug::render_paths(MODEL, VIEW).expect("renders").svg;
    let wires = polyline_points(&svg);

    // (orientation, perp, lo, hi) for every wire segment, tagged by wire.
    let mut segs: Vec<(usize, bool, f64, f64, f64)> = Vec::new();
    for (wi, pts) in wires.iter().enumerate() {
        for s in pts.windows(2) {
            let (a, b) = (s[0], s[1]);
            if (a.1 - b.1).abs() < 1e-6 {
                segs.push((wi, true, a.1, a.0.min(b.0), a.0.max(b.0))); // horizontal
            } else {
                segs.push((wi, false, a.0, a.1.min(b.1), a.1.max(b.1))); // vertical
            }
        }
    }

    for (i, &(wi, hi, pi, loi, hii)) in segs.iter().enumerate() {
        for &(wj, hj, pj, loj, hij) in &segs[i + 1..] {
            if wi == wj || hi != hj || (pi - pj).abs() > 1e-6 {
                continue;
            }
            let overlap = loi.max(loj) < hii.min(hij) - 1e-6;
            assert!(
                !overlap,
                "wires {wi} and {wj} overlap on a collinear segment at {pi}"
            );
        }
    }
}

/// Nudging must not push a wire into a component box. No wire segment may
/// pass through any component rectangle's interior.
#[test]
fn example_wires_clear_component_boxes() {
    let svg = wirebug::render_paths(MODEL, VIEW).expect("renders").svg;
    let rects = component_rects(&svg);

    for pts in polyline_points(&svg) {
        for s in pts.windows(2) {
            let (a, b) = (s[0], s[1]);
            for &(x, y, w, h) in &rects {
                assert!(
                    !segment_enters_rect(a, b, x, y, w, h),
                    "wire segment {a:?}->{b:?} crosses box at ({x},{y},{w},{h})"
                );
            }
        }
    }
}

/// True iff an axis-aligned segment passes through the rectangle's
/// interior (touching an edge does not count).
fn segment_enters_rect(a: (f64, f64), b: (f64, f64), x: f64, y: f64, w: f64, h: f64) -> bool {
    let (right, bottom) = (x + w, y + h);
    if (a.1 - b.1).abs() < 1e-6 {
        let yy = a.1;
        yy > y && yy < bottom && a.0.min(b.0) < right && a.0.max(b.0) > x
    } else {
        let xx = a.0;
        xx > x && xx < right && a.1.min(b.1) < bottom && a.1.max(b.1) > y
    }
}

/// With a grid set, every port centre lands on a grid line, and ports on
/// facing components at the same row line up — so the wire between them is
/// a single straight segment. One grid step is the port pitch.
#[test]
fn example_ports_align_on_the_grid() {
    let step = View::load(VIEW).expect("view parses").grid_step();
    let svg = wirebug::render_paths(MODEL, VIEW).expect("renders").svg;

    for (cx, cy) in port_centres(&svg) {
        assert_eq!(cx % step, 0.0, "port x {cx} is off the {step} grid");
        assert_eq!(cy % step, 0.0, "port y {cy} is off the {step} grid");
    }

    // pack.hv.pos and contactor.power.in sit on the same grid row, so at
    // least one wire is a straight two-point horizontal run.
    let straight = polyline_points(&svg)
        .iter()
        .any(|p| p.len() == 2 && (p[0].1 - p[1].1).abs() < 1e-6);
    assert!(
        straight,
        "expected a straight horizontal wire between aligned ports"
    );
}

/// The canvas must enclose every wire. The resolver bundle joins two
/// south-facing ports, so it dives below the boxes and gets nudged into a
/// fan — regression against sizing the viewBox from component bounds only,
/// which clipped all but the top wire.
#[test]
fn example_wires_stay_inside_the_viewbox() {
    let svg = wirebug::render_paths(MODEL, VIEW).expect("renders").svg;

    let vb = svg
        .split_once("viewBox=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(v, _)| v)
        .expect("viewBox present");
    let n: Vec<f64> = vb.split_whitespace().map(|s| s.parse().unwrap()).collect();
    let (minx, miny, maxx, maxy) = (n[0], n[1], n[0] + n[2], n[1] + n[3]);

    for pts in polyline_points(&svg) {
        for (x, y) in pts {
            assert!(
                x >= minx && x <= maxx && y >= miny && y <= maxy,
                "wire point ({x},{y}) is outside viewBox {n:?}"
            );
        }
    }
}

/// Pull every port `<circle>` centre as `(cx, cy)`.
fn port_centres(svg: &str) -> Vec<(f64, f64)> {
    svg.match_indices("<circle")
        .filter_map(|(start, _)| {
            let tag = &svg[start..];
            let end = tag.find("/>").or_else(|| tag.find('>'))?;
            let tag = &tag[..end];
            Some((attr(tag, "cx")?, attr(tag, "cy")?))
        })
        .collect()
}

/// Pull every component `<rect>` as `(x, y, width, height)`.
fn component_rects(svg: &str) -> Vec<(f64, f64, f64, f64)> {
    svg.match_indices("<rect")
        .filter_map(|(start, _)| {
            let tag = &svg[start..];
            let end = tag.find("/>").or_else(|| tag.find('>'))?;
            let tag = &tag[..end];
            Some((
                attr(tag, "x")?,
                attr(tag, "y")?,
                attr(tag, "width")?,
                attr(tag, "height")?,
            ))
        })
        .collect()
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')? + i;
    tag[i..j].parse().ok()
}

/// Pull the `points` of every `<polyline>` out of an SVG string.
fn polyline_points(svg: &str) -> Vec<Vec<(f64, f64)>> {
    svg.match_indices("<polyline")
        .filter_map(|(start, _)| {
            let tag = &svg[start..];
            let attr = tag.find("points=\"")? + "points=\"".len();
            let end = tag[attr..].find('"')? + attr;
            let pts = tag[attr..end]
                .split_whitespace()
                .filter_map(|p| {
                    let (x, y) = p.split_once(',')?;
                    Some((x.parse().ok()?, y.parse().ok()?))
                })
                .collect();
            Some(pts)
        })
        .collect()
}

#[test]
fn example_warns_about_unconnected_contactor_coil() {
    let model = Model::load(MODEL).expect("model parses");
    let report = model.validate().expect("validates");

    let warnings: Vec<String> = report.warnings.iter().map(ToString::to_string).collect();
    assert!(
        warnings.iter().any(|w| w.contains("contactor.coil.pos")),
        "expected warning for contactor.coil.pos in {warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("contactor.coil.neg")),
        "expected warning for contactor.coil.neg in {warnings:?}"
    );
}

#[test]
fn parsed_example_model_snapshot() {
    let model = Model::load(MODEL).expect("model parses");
    insta::assert_debug_snapshot!(model);
}

/// End-to-end through the library entry point used by the CLI binary.
/// Future e2e tests should look like this — no filesystem write, no
/// `assert_cmd` spin-up.
#[test]
fn render_paths_returns_svg_and_warnings() {
    let result = wirebug::render_paths(MODEL, VIEW).expect("renders");
    assert!(result.svg.contains("<svg"));
    assert!(result.svg.contains("HV Power Path"));
    assert_eq!(result.warnings.len(), 2);
}
