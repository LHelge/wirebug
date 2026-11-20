//! Black-box CLI tests.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

const FIXTURE_ROOT: &str = "tests/fixtures/basic_project";
const FIXTURE_MANIFEST: &str = "tests/fixtures/basic_project/wirebug.toml";

#[test]
fn render_writes_an_svg_per_view() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("svg");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", FIXTURE_ROOT, "--out", out.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("rendered"));

    // The top-level view lands as a slug of its title and is a real SVG
    // with its title and routed wires.
    let svg = std::fs::read_to_string(out.join("system_overview.svg")).expect("view rendered");
    assert!(svg.contains("<svg"));
    assert!(svg.contains("System Overview"));
    assert!(svg.contains("class=\"wire\""));

    // The battery detail wraps its children in the subject's enclosure,
    // drawing the pack's own external ports on the boundary.
    let detail =
        std::fs::read_to_string(out.join("battery_detail.svg")).expect("battery view rendered");
    assert!(detail.contains("class=\"enclosure\""));
    assert!(detail.contains("class=\"enclosure-label\""));

    // The harness view renders connectors as pin tables with labelled,
    // gauged cable bundles between them.
    let harness =
        std::fs::read_to_string(out.join("main_harness.svg")).expect("harness view rendered");
    assert!(harness.contains("class=\"connector\""));
    assert!(harness.contains("class=\"cable-wire\""));
    assert!(harness.contains("HV+ · 50mm²"));

    // The pinout view draws the typed connector's cavity face.
    assert!(out.join("inverter_pinout.svg").is_file());

    // The index groups the two view kinds into tabs.
    let index = std::fs::read_to_string(out.join("index.html")).expect("index rendered");
    assert!(index.contains("id=\"tab-schematic\""));
    assert!(index.contains("id=\"tab-harness\""));
}

#[test]
fn render_png_writes_a_png_per_view_and_index_references_png() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("png");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            FIXTURE_MANIFEST,
            "--out",
            out.to_str().unwrap(),
            "--png",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("rendered"));

    // Each view lands as a real PNG (right magic bytes) under the slug
    // it would have got as an SVG, with `.png` swapped in.
    let png = std::fs::read(out.join("system_overview.png")).expect("view rasterised");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(out.join("battery_detail.png").is_file());
    assert!(out.join("main_harness.png").is_file());
    assert!(out.join("inverter_pinout.png").is_file());

    // The matching SVGs are not written in PNG mode.
    assert!(!out.join("system_overview.svg").exists());
    assert!(!out.join("battery_detail.svg").exists());

    // The index references the PNGs, never the SVGs.
    let index = std::fs::read_to_string(out.join("index.html")).expect("index rendered");
    assert!(index.contains("src=\"system_overview.png\""));
    assert!(index.contains("src=\"main_harness.png\""));
    assert!(!index.contains(".svg"));
}

#[test]
fn render_pdf_writes_a_single_pdf_and_no_svgs_or_index() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("pdf");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            FIXTURE_MANIFEST,
            "--out",
            out.to_str().unwrap(),
            "--pdf",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("rendered").and(predicate::str::contains("pdf")));

    // One PDF named after the project slug, with the magic header and one
    // page per view (the fixture has four).
    let pdf = std::fs::read(out.join("basic_project.pdf")).expect("pdf written");
    assert_eq!(&pdf[..5], b"%PDF-");
    assert!(
        pdf.windows(8).any(|w| w == b"/Count 4"),
        "expected a four-page tree"
    );
    assert!(
        pdf.windows(7).any(|w| w == b"(1 / 4)"),
        "expected a page-number footer"
    );

    // The PDF replaces the normal output: no per-view files, no index.
    let entries: Vec<String> = std::fs::read_dir(&out)
        .expect("out dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, ["basic_project.pdf"]);
}

#[test]
fn render_rejects_pdf_combined_with_png_or_embed() {
    for conflicting in ["--png", "--embed"] {
        Command::cargo_bin("wirebug")
            .expect("binary present")
            .args(["render", FIXTURE_MANIFEST, "--out", "unused", "--pdf"])
            .arg(conflicting)
            .assert()
            .failure()
            .stderr(predicate::str::contains("cannot be used with"));
    }
}

#[test]
fn render_embed_writes_manifest_and_no_index() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("embed");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            FIXTURE_ROOT,
            "--out",
            out.to_str().unwrap(),
            "--embed",
        ])
        .assert()
        .success();

    // SVGs still land on disk under their slugged file names.
    let svg = std::fs::read_to_string(out.join("system_overview.svg")).expect("view rendered");

    // Embed-mode SVGs drop the built-in <style>, suppress the corner
    // stamp, and class-tag the root so a host stylesheet can scope
    // rules under `.wirebug`.
    assert!(!svg.contains("<style>"));
    assert!(!svg.contains("class=\"stamp\""));
    assert!(!svg.contains("basic-project v0.1.0"));
    assert!(svg.contains("class=\"wirebug wirebug-schematic\""));

    let harness =
        std::fs::read_to_string(out.join("main_harness.svg")).expect("harness view rendered");
    assert!(harness.contains("class=\"wirebug wirebug-harness\""));

    let pinout =
        std::fs::read_to_string(out.join("inverter_pinout.svg")).expect("pinout view rendered");
    assert!(pinout.contains("class=\"wirebug wirebug-pinout\""));

    // The HTML index is replaced by a JSON sidecar listing the views.
    assert!(!out.join("index.html").exists());
    let manifest_src =
        std::fs::read_to_string(out.join("manifest.json")).expect("embed manifest written");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_src).expect("manifest is valid JSON");
    assert_eq!(manifest["project"]["name"], "basic-project");

    // A companion stylesheet reproduces the standalone look, scoped under
    // the classes the embed roots carry, and the manifest points at it.
    assert_eq!(manifest["stylesheet"], "wirebug.css");
    let css = std::fs::read_to_string(out.join("wirebug.css")).expect("stylesheet written");
    assert!(css.contains(".wirebug-schematic .component rect {"));
    assert!(css.contains(".wirebug-harness .connector rect {"));
    let views = manifest["views"].as_array().expect("views array");
    let first = &views[0];
    assert_eq!(first["title"], "System Overview");
    assert_eq!(first["filename"], "system_overview.svg");
    assert_eq!(first["kind"], "schematic");
    assert!(
        views
            .iter()
            .any(|v| v["kind"] == "harness" && v["filename"] == "main_harness.svg"),
        "harness view listed in manifest"
    );
}

#[test]
fn render_rejects_a_project_that_does_not_check() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    std::fs::write(&main, "use missing from \"nope.wb\";\ncomponent c { }\n")
        .expect("write main.wb");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            main.to_str().unwrap(),
            "--out",
            tmp.path().join("svg").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not rendering"));
}

#[test]
fn render_disambiguates_duplicate_view_titles() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    let out = tmp.path().join("svg");
    std::fs::write(
        tmp.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write wirebug.toml");
    std::fs::write(
        &main,
        r#"
component c { }
view schematic "Overview" { }
view schematic "Overview" { }
"#,
    )
    .expect("write main.wb");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            main.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        // `check` warns on the repeat, which doesn't block a render.
        .success()
        .stderr(predicate::str::contains(
            "duplicate view title \"Overview\"",
        ));

    assert!(out.join("overview.svg").is_file());
    assert!(out.join("overview_2.svg").is_file());

    let index = std::fs::read_to_string(out.join("index.html")).expect("index rendered");
    assert!(index.contains("src=\"overview.svg\""));
    assert!(index.contains("src=\"overview_2.svg\""));
}

#[test]
fn help_text_mentions_render() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("render"));
}

#[test]
fn help_text_mentions_check() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("check"));
}

#[test]
fn help_text_mentions_serve() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("serve"));
}

#[test]
fn serve_help_documents_the_port_flag() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--port"));
}

#[test]
fn serve_help_documents_the_host_flag() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--host"));
}

#[test]
fn check_accepts_the_fixture_project() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", FIXTURE_ROOT])
        .assert()
        .success();
}

const INVALID_VALUES_ROOT: &str = "tests/fixtures/invalid_values_project";

#[test]
fn check_rejects_non_positive_gauges_and_cable_length() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", INVALID_VALUES_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main =
        std::fs::read_to_string(format!("{INVALID_VALUES_ROOT}/main.wb")).expect("fixture main.wb");

    // Each problem is an error labelled on the offending number itself.
    let found: Vec<(&str, &str)> = problems
        .iter()
        .map(|p| {
            assert_eq!(p["severity"], "error", "{p}");
            let span = &p["labels"][0]["span"];
            let offset = span["offset"].as_u64().expect("offset") as usize;
            let length = span["length"].as_u64().expect("length") as usize;
            (
                p["code"].as_str().expect("code"),
                &main[offset..offset + length],
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("wirebug::invalid_gauge", "-1"),
            ("wirebug::invalid_gauge", "0"),
            ("wirebug::invalid_length", "-2"),
        ]
    );
}

#[test]
fn render_refuses_a_non_positive_cable_length() {
    let tmp = tempdir().expect("tempdir");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            INVALID_VALUES_ROOT,
            "--out",
            tmp.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("length must be positive, got -2"))
        .stderr(predicate::str::contains("not rendering"));
}

const DEGENERATE_WIRES_ROOT: &str = "tests/fixtures/degenerate_wires_project";

#[test]
fn check_rejects_a_repeated_endpoint_and_warns_on_an_empty_cable() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", DEGENERATE_WIRES_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main = std::fs::read_to_string(format!("{DEGENERATE_WIRES_ROOT}/main.wb"))
        .expect("fixture main.wb");

    /// A problem as (code, severity, the offset and text of every label).
    type Found<'a> = (&'a str, &'a str, Vec<(usize, &'a str)>);
    let found: Vec<Found> = problems
        .iter()
        .map(|p| {
            let labels = p["labels"]
                .as_array()
                .expect("labels")
                .iter()
                .map(|l| {
                    let offset = l["span"]["offset"].as_u64().expect("offset") as usize;
                    let length = l["span"]["length"].as_u64().expect("length") as usize;
                    (offset, &main[offset..offset + length])
                })
                .collect();
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                labels,
            )
        })
        .collect();

    // Both occurrences in the looping wire are named; the double crimp on
    // `pump.gnd` is not reported.
    let looping = main
        .find("[ecu.gnd, ecu.gnd]")
        .expect("looping wire in fixture");
    let first = looping + 1;
    let again = looping + "[ecu.gnd, ".len();
    let spare = main.find("spare").expect("empty cable in fixture");
    assert_eq!(
        found,
        [
            (
                "wirebug::duplicate_endpoint",
                "error",
                vec![(again, "ecu.gnd"), (first, "ecu.gnd")],
            ),
            ("wirebug::empty_cable", "warning", vec![(spare, "spare")]),
        ]
    );
}

const DUPLICATE_TITLES_ROOT: &str = "tests/fixtures/duplicate_titles_project";

#[test]
fn check_warns_on_a_view_title_repeated_in_another_file() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", DUPLICATE_TITLES_ROOT])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let read = |name: &str| {
        std::fs::read_to_string(format!("{DUPLICATE_TITLES_ROOT}/{name}")).expect("fixture file")
    };
    let (main, body) = (read("main.wb"), read("body.wb"));

    /// A diagnostic as (code, severity, file name, the text of its labels).
    fn summary<'a>(
        p: &'a serde_json::Value,
        src: &'a str,
    ) -> (&'a str, &'a str, &'a str, Vec<&'a str>) {
        let file = p["filename"].as_str().expect("filename");
        let labels = p["labels"]
            .as_array()
            .expect("labels")
            .iter()
            .map(|l| {
                let offset = l["span"]["offset"].as_u64().expect("offset") as usize;
                let length = l["span"]["length"].as_u64().expect("length") as usize;
                &src[offset..offset + length]
            })
            .collect();
        (
            p["code"].as_str().unwrap_or(""),
            p["severity"].as_str().expect("severity"),
            file.rsplit('/').next().expect("file name"),
            labels,
        )
    }

    // The repeat in body.wb is the warning; the first view, in main.wb, is
    // its related note.
    let [problem] = problems.as_slice() else {
        panic!("expected one problem, got {problems:?}");
    };
    assert_eq!(
        summary(problem, &body),
        (
            "wirebug::duplicate_view_title",
            "warning",
            "body.wb",
            vec!["\"Overview\""]
        )
    );
    let [first] = problem["related"].as_array().expect("related").as_slice() else {
        panic!("expected one related note: {problem}");
    };
    assert_eq!(
        summary(first, &main),
        ("", "advice", "main.wb", vec!["\"Overview\""])
    );
}

#[test]
fn check_strict_fails_on_a_duplicate_view_title() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", DUPLICATE_TITLES_ROOT])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "duplicate view title \"Overview\"",
        ));
}

const UNDRAWN_CONDUCTORS_ROOT: &str = "tests/fixtures/undrawn_conductors_project";

#[test]
fn check_warns_on_conductors_no_harness_view_draws() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", UNDRAWN_CONDUCTORS_ROOT])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let read = |name: &str| {
        std::fs::read_to_string(format!("{UNDRAWN_CONDUCTORS_ROOT}/{name}")).expect("fixture file")
    };
    let (main, horn) = (read("main.wb"), read("horn.wb"));

    // A problem as (code, severity, file name, message, the text of every
    // label).
    let found: Vec<(&str, &str, &str, &str, Vec<&str>)> = problems
        .iter()
        .map(|p| {
            let file = p["filename"].as_str().expect("filename");
            let src = if file.ends_with("horn.wb") {
                &horn
            } else {
                &main
            };
            let labels = p["labels"]
                .as_array()
                .expect("labels")
                .iter()
                .map(|l| {
                    let offset = l["span"]["offset"].as_u64().expect("offset") as usize;
                    let length = l["span"]["length"].as_u64().expect("length") as usize;
                    &src[offset..offset + length]
                })
                .collect();
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                file.rsplit('/').next().expect("file name"),
                p["message"].as_str().expect("message"),
                labels,
            )
        })
        .collect();

    // The rail's undrawn segment and the cable conductor in the other file,
    // each on its own wire; the drawn segment, the drawn loose wire and the
    // wire of `Dash` (no harness view) are not reported.
    assert_eq!(
        found,
        vec![
            (
                "wirebug::undrawn_conductor",
                "warning",
                "main.wb",
                "wire from `lamp.gnd` to `horn.gnd` is in no harness view of `Car`",
                vec!["wire black 1 [ecu.gnd, lamp.gnd, horn.gnd];"]
            ),
            (
                "wirebug::undrawn_conductor",
                "warning",
                "horn.wb",
                "cable `horn_feed` conductor \"HORN\" from `ecu.horn` to `horn.sig` is in no harness view of `Car`",
                vec!["wire red 1 \"HORN\" [ecu.horn, horn.sig];"]
            ),
        ]
    );
}

#[test]
fn check_strict_fails_on_an_undrawn_conductor() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", UNDRAWN_CONDUCTORS_ROOT])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "include `lamp.x1` and `horn.x1` in one harness view of `Car`",
        ));
}

const CROWDED_VIEWS_ROOT: &str = "tests/fixtures/crowded_views_project";

#[test]
fn check_rejects_includes_that_overlap_or_leave_a_wire_no_route() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", CROWDED_VIEWS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let read = |name: &str| {
        std::fs::read_to_string(format!("{CROWDED_VIEWS_ROOT}/{name}")).expect("fixture file")
    };
    let (main, module) = (read("main.wb"), read("components/module.wb"));

    // A problem as (code, severity, file name, the text of every label).
    let found: Vec<(&str, &str, &str, Vec<&str>)> = problems
        .iter()
        .map(|p| {
            let file = p["filename"].as_str().expect("filename");
            let src = if file.ends_with("module.wb") {
                &module
            } else {
                &main
            };
            let labels = p["labels"]
                .as_array()
                .expect("labels")
                .iter()
                .map(|l| {
                    let offset = l["span"]["offset"].as_u64().expect("offset") as usize;
                    let length = l["span"]["length"].as_u64().expect("length") as usize;
                    &src[offset..offset + length]
                })
                .collect();
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                file.rsplit('/').next().expect("file name"),
                labels,
            )
        })
        .collect();

    // Each colliding pair is named by both includes, the later one first,
    // and the walled-in wire by the includes it ends on; the clean
    // "Touching" view (boxes edge to edge inside an enclosure) reports
    // nothing.
    assert_eq!(
        found,
        vec![
            (
                "wirebug::include_too_close",
                "error",
                "main.wb",
                vec!["right", "left"]
            ),
            (
                "wirebug::overlapping_includes",
                "error",
                "main.wb",
                vec!["rear", "left"]
            ),
            (
                "wirebug::unroutable_wire",
                "error",
                "main.wb",
                vec!["left", "rear"]
            ),
            (
                "wirebug::overlapping_includes",
                "error",
                "main.wb",
                vec!["right.x1", "left.x1"]
            ),
            (
                "wirebug::overlapping_includes",
                "error",
                "module.wb",
                vec!["x2", "x1"]
            ),
        ]
    );
    assert!(
        problems[0]["message"]
            .as_str()
            .expect("message")
            .contains("`left.sig`, `right.sig`"),
        "the crowded ports are named: {}",
        problems[0]
    );
}

#[test]
fn render_stops_at_check_instead_of_failing_to_route() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("svg");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", CROWDED_VIEWS_ROOT, "--out", out.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("include_too_close"))
        .stderr(predicate::str::contains("not rendering"))
        .stderr(predicate::str::contains("failed to route").not());
    assert!(!out.exists(), "nothing is written");
}

/// A project that passes `check` but has one view `render` cannot draw
/// ("Fine Grid"), between a schematic and a harness view that render. Its
/// failure is a too-fine `grid:`, which only the renderer rejects today;
/// when `check` learns to, the fixture swaps in another failure `check`
/// does not catch, and these tests keep their meaning.
const RENDER_FAILURE_ROOT: &str = "tests/fixtures/render_failure_project";

/// `render` the render-failure fixture with `flags`, asserting it fails
/// naming the failed view, and return the output directory's file names.
fn render_with_a_failing_view(flags: &[&str]) -> (tempfile::TempDir, Vec<String>) {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("out");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            RENDER_FAILURE_ROOT,
            "--out",
            out.to_str().unwrap(),
        ])
        .args(flags)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "error: view \"Fine Grid\": grid step 5 is too small",
        ))
        .stderr(predicate::str::contains("rendered 2 of 3 view(s)"));
    let mut entries: Vec<String> = std::fs::read_dir(&out)
        .expect("out dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    (tmp, entries)
}

#[test]
fn render_writes_the_views_that_render_and_names_the_one_that_fails() {
    let (tmp, entries) = render_with_a_failing_view(&[]);
    assert_eq!(entries, ["harness.svg", "index.html", "overview.svg"]);

    // The index shows the failure where the drawing would be.
    let index = std::fs::read_to_string(tmp.path().join("out/index.html")).expect("index");
    assert!(index.contains("<img src=\"overview.svg\""));
    assert!(index.contains("<img src=\"harness.svg\""));
    assert!(index.contains("<section class=\"failed\">\n        <h2>Fine Grid</h2>"));
    assert!(index.contains("grid step 5 is too small"));
}

#[test]
fn render_png_rasterises_the_views_that_render() {
    let (_tmp, entries) = render_with_a_failing_view(&["--png"]);
    assert_eq!(entries, ["harness.png", "index.html", "overview.png"]);
}

#[test]
fn render_pdf_pages_only_the_views_that_render() {
    let (tmp, entries) = render_with_a_failing_view(&["--pdf"]);
    assert_eq!(entries, ["render_failure_project.pdf"]);
    let pdf = std::fs::read(tmp.path().join("out/render_failure_project.pdf")).expect("pdf");
    assert!(pdf.windows(8).any(|w| w == b"/Count 2"), "two pages");
    assert!(pdf.windows(7).any(|w| w == b"(2 / 2)"), "numbered of two");
}

#[test]
fn render_embed_lists_only_the_views_it_wrote() {
    let (tmp, entries) = render_with_a_failing_view(&["--embed"]);
    assert_eq!(
        entries,
        [
            "harness.svg",
            "manifest.json",
            "overview.svg",
            "wirebug.css"
        ]
    );
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join("out/manifest.json")).expect("manifest"),
    )
    .expect("json");
    let titles: Vec<&str> = manifest["views"]
        .as_array()
        .expect("views")
        .iter()
        .map(|v| v["title"].as_str().expect("title"))
        .collect();
    assert_eq!(titles, ["Overview", "Harness"]);
}

#[test]
fn check_accepts_a_manifest_target() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", FIXTURE_MANIFEST])
        .assert()
        .success();
}

#[test]
fn manifest_version_prints_the_v_prefixed_project_version() {
    // The fixture manifest declares version 0.1.0.
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["manifest", "version", FIXTURE_ROOT])
        .assert()
        .success()
        .stdout(predicate::str::diff("v0.1.0\n"));
}

#[test]
fn manifest_version_accepts_a_manifest_target() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["manifest", "version", FIXTURE_MANIFEST])
        .assert()
        .success()
        .stdout(predicate::str::diff("v0.1.0\n"));
}

#[test]
fn manifest_version_works_even_when_the_wb_sources_do_not_compile() {
    // A project whose main.wb is broken still has a readable manifest, so
    // CI can check the tag before the design necessarily checks clean.
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("wirebug.toml"),
        "[project]\nname = \"broken\"\nversion = \"9.9.9\"\n",
    )
    .expect("write wirebug.toml");
    std::fs::write(tmp.path().join("main.wb"), "component c { @ }\n").expect("write main.wb");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["manifest", "version", tmp.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::diff("v9.9.9\n"));
}

#[test]
fn manifest_version_fails_when_there_is_no_project() {
    let tmp = tempdir().expect("tempdir");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["manifest", "version", tmp.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn check_rejects_a_dangling_use() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    std::fs::write(
        &main,
        "use missing from \"nope.wb\";\ncomponent c { pub port a \"A\"; }\n",
    )
    .expect("write main.wb");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", main.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot find imported file"));
}

#[test]
fn check_accepts_use_after_a_component() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    std::fs::write(
        tmp.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write wirebug.toml");
    std::fs::write(
        &main,
        r#"
component m { l: leaf; }
use leaf from "leaf.wb";
view schematic "M" { include l at (0, 0); }
"#,
    )
    .expect("write main.wb");
    std::fs::write(tmp.path().join("leaf.wb"), "component leaf { }\n").expect("write leaf.wb");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", main.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn check_rejects_an_undeclared_inline_half() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    std::fs::write(
        tmp.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write wirebug.toml");
    std::fs::write(
        &main,
        r#"
connector_type M2 "M 2p" { }
component m {
    pub port a "A";
    inline ic {
        male: M2;
        port sig "SIG" pin 1;
    }
    wire red 1 [a, ic.sig];
}
view harness "H" { include ic.female at (0, 0); }
"#,
    )
    .expect("write main.wb");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", main.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("undeclared_inline_half"));
}
