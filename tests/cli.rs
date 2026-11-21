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
fn check_rejects_non_positive_gauges_and_lengths_and_unknown_units() {
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

    // Each problem is an error labelled on the offending number itself (its
    // unit included), or on the unit that is no length unit.
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
            ("wirebug::invalid_length", "-500mm"),
            ("wirebug::invalid_length", "-2"),
            ("wirebug::unknown_length_unit", "in"),
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

const SHIELD_ERRORS_ROOT: &str = "tests/fixtures/shield_errors_project";

#[test]
fn check_reports_each_wrong_shield_where_it_is_written() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", SHIELD_ERRORS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main =
        std::fs::read_to_string(format!("{SHIELD_ERRORS_ROOT}/main.wb")).expect("fixture main.wb");

    /// A problem as (code, severity, the text of every label: the repeat
    /// before the first).
    type Found<'a> = (&'a str, &'a str, Vec<&'a str>);
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
                    &main[offset..offset + length]
                })
                .collect();
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                labels,
            )
        })
        .collect();

    assert_eq!(
        found,
        [
            ("wirebug::unknown_port", "error", vec!["ghost"]),
            (
                "wirebug::duplicate_shield",
                "error",
                vec![r#"shield "SECOND";"#, "shield [ecu.gnd];"],
            ),
            (
                "wirebug::duplicate_shield",
                "error",
                vec![r#"shield "AGAIN" [pump.gnd];"#, "shield;"],
            ),
            (
                "wirebug::shield_arity",
                "error",
                vec!["shield [ecu.gnd, pump.gnd, ecu.pwr];"],
            ),
            (
                "wirebug::duplicate_endpoint",
                "error",
                vec!["pump.gnd", "pump.gnd"],
            ),
            ("wirebug::empty_cable", "warning", vec!["bare"]),
        ]
    );
    let messages: Vec<&str> = problems
        .iter()
        .map(|p| p["message"].as_str().expect("message"))
        .collect();
    assert!(messages.contains(&"cable `doubled` has more than one shield"));
    assert!(messages.contains(&"a twisted pair of cable `paired` has more than one shield"));
    assert!(messages.contains(&"shield lists endpoint `pump.gnd` twice"));
}

const SPLICE_ERRORS_ROOT: &str = "tests/fixtures/splice_errors_project";

#[test]
fn check_reports_each_wrong_splice_where_it_is_written() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", SPLICE_ERRORS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main =
        std::fs::read_to_string(format!("{SPLICE_ERRORS_ROOT}/main.wb")).expect("fixture main.wb");

    /// A problem as (code, severity, the text of every label).
    type Found<'a> = (&'a str, &'a str, Vec<&'a str>);
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
                    &main[offset..offset + length]
                })
                .collect();
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                labels,
            )
        })
        .collect();

    assert_eq!(
        found,
        [
            (
                "wirebug::duplicate_splice_name",
                "error",
                vec!["out", "out"]
            ),
            ("wirebug::unknown_port", "error", vec!["spp"]),
            ("wirebug::shield_on_splice", "error", vec!["sp"]),
            ("wirebug::wrong_include_form", "error", vec!["sp"]),
            (
                "wirebug::splice_wire_arity",
                "error",
                vec!["wire black 1 [a.y, sp, c.y];"],
            ),
            ("wirebug::unknown_cable_property", "error", vec!["colour"]),
            ("wirebug::splice_joins_nothing", "warning", vec!["once"]),
        ]
    );
    let messages: Vec<&str> = problems
        .iter()
        .map(|p| p["message"].as_str().expect("message"))
        .collect();
    assert!(messages.contains(&"splice `out` has the same name as a port of `Vehicle`"));
    assert!(messages.contains(&"a wire to a splice has exactly two ends, found 3"));
    assert!(messages.contains(&"splice `once` joins nothing: only one conductor ends on it"));
    assert!(messages.contains(&"unknown splice property `colour`"));
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

const PART_DATA_ERRORS_ROOT: &str = "tests/fixtures/part_data_errors_project";

#[test]
fn check_rejects_bad_part_data_and_a_key_an_extend_fragment_sets_again() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", PART_DATA_ERRORS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");

    /// A diagnostic as (code, severity, file name, the text of its labels).
    fn summary(p: &serde_json::Value) -> (String, String, String, Vec<String>) {
        let file = p["filename"].as_str().expect("filename");
        let src = std::fs::read_to_string(file).expect("the file a diagnostic names");
        let labels = p["labels"]
            .as_array()
            .expect("labels")
            .iter()
            .map(|l| {
                let offset = l["span"]["offset"].as_u64().expect("offset") as usize;
                let length = l["span"]["length"].as_u64().expect("length") as usize;
                src[offset..offset + length].to_string()
            })
            .collect();
        (
            p["code"].as_str().unwrap_or("").to_string(),
            p["severity"].as_str().expect("severity").to_string(),
            file.rsplit('/').next().expect("file name").to_string(),
            labels,
        )
    }
    let found: Vec<_> = problems.iter().map(summary).collect();
    let row = |code: &str, file: &str, label: &str| {
        (
            code.to_string(),
            "error".to_string(),
            file.to_string(),
            vec![label.to_string()],
        )
    };
    assert_eq!(
        found,
        [
            row("wirebug::cable_property_type", "relay.wb", "40"),
            row("wirebug::unknown_cable_property", "relay.wb", "colour"),
            row("wirebug::duplicate_cable_property", "harness.wb", "part"),
        ]
    );
    // The root's `part`, in main.wb, rides along as the repeat's related
    // note.
    let related = problems[2]["related"].as_array().expect("related");
    let [first] = related.as_slice() else {
        panic!("expected one related note: {}", problems[2]);
    };
    assert_eq!(
        summary(first),
        (
            String::new(),
            "advice".to_string(),
            "main.wb".to_string(),
            vec!["part".to_string()]
        )
    );
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

    // The rail's undrawn segment, the conductor from the splice to a
    // connector no view includes, and the cable conductor in the other
    // file, each on its own wire; the drawn segment, the drawn loose wire,
    // the drawn conductor to the splice and the wire of `Dash` (no harness
    // view) are not reported.
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
                "main.wb",
                "wire \"AUX\" from `sp` to `horn.aux` is in no harness view of `Car`",
                vec!["wire white 0.5 \"AUX\" [sp, horn.aux];"]
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

/// Views whose `grid:` their renderer cannot draw: too fine for a
/// schematic's port labels, zero for a harness, negative for a pinout. A
/// project that passes `check` renders every view, so how `render` writes
/// around a view that fails is tested on hand-built designs
/// (`render::output`, `serve::build`).
const INVALID_GRIDS_ROOT: &str = "tests/fixtures/invalid_grids_project";

#[test]
fn check_rejects_a_grid_its_renderer_cannot_draw() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", INVALID_GRIDS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main =
        std::fs::read_to_string(format!("{INVALID_GRIDS_ROOT}/main.wb")).expect("fixture main.wb");

    // Each is an error labelled on the grid's number; the valid view
    // reports nothing.
    let found: Vec<(&str, &str, &str)> = problems
        .iter()
        .map(|p| {
            let span = &p["labels"][0]["span"];
            let offset = span["offset"].as_u64().expect("offset") as usize;
            let length = span["length"].as_u64().expect("length") as usize;
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                &main[offset..offset + length],
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("wirebug::grid_too_fine", "error", "5"),
            ("wirebug::invalid_grid", "error", "0"),
            ("wirebug::invalid_grid", "error", "-10"),
        ]
    );
    assert_eq!(
        problems[0]["message"],
        "grid step 5 is too fine for a schematic view; ports need at least 7.5 per step"
    );
}

#[test]
fn render_stops_at_check_instead_of_failing_on_a_grid() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("out");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", INVALID_GRIDS_ROOT, "--out", out.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("grid_too_fine"))
        .stderr(predicate::str::contains("not rendering"))
        .stderr(predicate::str::contains("error: view").not());
    assert!(!out.exists(), "nothing is written");
}

/// Views whose includes sit a thousand grid steps apart, past the canvas
/// `--png` will rasterise: a schematic too wide, a harness too tall, a
/// pinout both (a pixmap of 6.5 GB), beside one that fits. How `--png` refuses such a view while writing the
/// others is tested on a hand-built design (`render::output`).
const OVERSIZED_VIEWS_ROOT: &str = "tests/fixtures/oversized_views_project";

#[test]
fn check_rejects_a_view_too_large_to_rasterise() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", OVERSIZED_VIEWS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main = std::fs::read_to_string(format!("{OVERSIZED_VIEWS_ROOT}/main.wb"))
        .expect("fixture main.wb");

    // Each is an error labelled on the view's title, then on the includes
    // farthest apart along its longer side; the view that fits reports
    // nothing.
    let found: Vec<(&str, &str, Vec<&str>)> = problems
        .iter()
        .map(|p| {
            let labels = p["labels"]
                .as_array()
                .expect("labels")
                .iter()
                .map(|l| {
                    let offset = l["span"]["offset"].as_u64().expect("offset") as usize;
                    let length = l["span"]["length"].as_u64().expect("length") as usize;
                    &main[offset..offset + length]
                })
                .collect();
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
                labels,
            )
        })
        .collect();
    let too_large = |labels: &[&'static str]| ("wirebug::view_too_large", "error", labels.to_vec());
    assert_eq!(
        found,
        [
            too_large(&["\"Wide\"", "left", "right"]),
            too_large(&["\"Tall\"", "left.x1", "right.x1"]),
            too_large(&["\"Service\"", "x8", "x9"]),
        ]
    );
    assert_eq!(
        problems[0]["message"],
        "view \"Wide\" is 20256 × 218 units, more than the 8192 a view may span on a side"
    );
}

#[test]
fn render_png_stops_at_check_instead_of_running_out_of_memory() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("out");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            OVERSIZED_VIEWS_ROOT,
            "--out",
            out.to_str().unwrap(),
            "--png",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("view_too_large"))
        .stderr(predicate::str::contains("not rendering"))
        .stderr(predicate::str::contains("error: view").not());
    assert!(!out.exists(), "nothing is written");
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

/// `main.wb` merges `traction.wb` in with the older spelling,
/// `use Vehicle from "traction.wb";`.
const USE_AS_MERGE_TRIGGER_ROOT: &str = "tests/fixtures/split_project";

#[test]
fn check_strict_accepts_use_as_a_merge_trigger() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", USE_AS_MERGE_TRIGGER_ROOT])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
}

/// `main.wb` pulls its fragments in with `fragment "<path>";`.
const FRAGMENT_ROOT: &str = "tests/fixtures/fragment_project";

#[test]
fn check_strict_accepts_fragment_lines() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", FRAGMENT_ROOT])
        .assert()
        .success();
}

#[test]
fn check_reports_a_missing_unused_or_repeated_fragment() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write wirebug.toml");
    std::fs::write(tmp.path().join("lamp.wb"), "component Lamp { }\n").expect("write lamp.wb");
    std::fs::write(
        tmp.path().join("hv.wb"),
        "extend Root { pub port hv \"HV\"; }\n",
    )
    .expect("write hv.wb");
    std::fs::write(
        tmp.path().join("main.wb"),
        "fragment \"nope.wb\";\nfragment \"lamp.wb\";\nfragment \"hv.wb\";\nfragment \"hv.wb\";\ncomponent Root { }\n",
    )
    .expect("write main.wb");

    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", tmp.path().to_str().unwrap()])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let found: Vec<(&str, &str)> = problems
        .iter()
        .map(|p| {
            (
                p["code"].as_str().expect("code"),
                p["severity"].as_str().expect("severity"),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("wirebug::use_not_found", "error"),
            ("wirebug::unused_fragment", "warning"),
            ("wirebug::duplicate_fragment", "warning"),
        ]
    );
}

/// A project whose `main.wb` names `systems/all.wb` with `fragment`, and
/// `systems/all.wb` holds `all` and nothing else.
fn project_with_an_aggregating_file(all: &str) -> tempfile::TempDir {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write wirebug.toml");
    std::fs::write(
        tmp.path().join("main.wb"),
        "fragment \"systems/all.wb\";\ncomponent Vehicle { }\nview schematic \"V\" { }\n",
    )
    .expect("write main.wb");
    std::fs::create_dir(tmp.path().join("systems")).expect("mkdir systems");
    std::fs::write(tmp.path().join("systems/all.wb"), all).expect("write all.wb");
    std::fs::write(
        tmp.path().join("systems/a.wb"),
        "extend Vehicle { pub port x \"X\"; }\n",
    )
    .expect("write a.wb");
    tmp
}

#[test]
fn check_strict_accepts_an_aggregating_file_that_uses_the_component() {
    let tmp =
        project_with_an_aggregating_file("use Vehicle from \"../main.wb\";\nfragment \"a.wb\";\n");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", tmp.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
}

#[test]
fn check_strict_rejects_two_files_that_only_fragment_each_other() {
    let tmp = project_with_an_aggregating_file("fragment \"../main.wb\";\n");
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "check",
            "--strict",
            "--format",
            "json",
            tmp.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let codes: Vec<&str> = problems
        .iter()
        .map(|p| p["code"].as_str().expect("code"))
        .collect();
    assert_eq!(
        codes,
        ["wirebug::unused_fragment", "wirebug::unused_fragment"]
    );
}

/// A project whose `systems/all.wb` pulls in two loom files with `fragment`
/// but lacks `use Vehicle from "../main.wb";`, so their `extend`s merge into
/// nothing; each loom file has a harness view when `views` is set. The
/// problem codes `check --format json` reports, and the help of its first
/// `orphan_fragment`.
fn check_aggregator_without_the_component(views: bool) -> (Vec<String>, String) {
    let tmp = tempdir().expect("tempdir");
    let write = |path: &str, src: &str| {
        let path = tmp.path().join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, src).expect("write");
    };
    write(
        "wirebug.toml",
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    );
    write(
        "components/leaf.wb",
        "component Leaf { connector c \"C 1p\" { pub port a \"A\" pin 1; } }\n",
    );
    write(
        "main.wb",
        "use Leaf from \"components/leaf.wb\";\nfragment \"systems/all.wb\";\n\
         component Vehicle { pack: Leaf \"Pack\"; }\nview schematic \"V\" { }\n",
    );
    write("systems/all.wb", "fragment \"a.wb\";\nfragment \"b.wb\";\n");
    for (file, device) in [("a", "inv"), ("b", "dcdc")] {
        let view = if views {
            format!(
                "view harness \"{file} loom\" {{ include pack.c at (2, 2); include {device}.c at (10, 2); }}\n"
            )
        } else {
            String::new()
        };
        write(
            &format!("systems/{file}.wb"),
            &format!(
                "use Leaf from \"../components/leaf.wb\";\n\
                 extend Vehicle {{ {device}: Leaf \"D\"; wire red 1 [pack.a, {device}.a]; }}\n{view}"
            ),
        );
    }
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", tmp.path().to_str().unwrap()])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let codes = problems
        .iter()
        .map(|p| p["code"].as_str().expect("code").to_string())
        .collect();
    let help = problems
        .iter()
        .find(|p| p["code"] == "wirebug::orphan_fragment")
        .and_then(|p| p["help"].as_str())
        .expect("an orphan_fragment with a help")
        .to_string();
    (codes, help)
}

#[test]
fn check_says_where_to_use_the_component_an_aggregator_lacks() {
    for views in [false, true] {
        let (codes, help) = check_aggregator_without_the_component(views);
        assert_eq!(
            codes,
            [
                "wirebug::orphan_fragment",
                "wirebug::orphan_fragment",
                "wirebug::unused_fragment",
                "wirebug::unused_fragment",
                "wirebug::unused_fragment",
            ],
            "views: {views}"
        );
        assert!(
            help.contains("add `use Vehicle from \"../main.wb\";` to `systems/all.wb`"),
            "{help}"
        );
    }
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

/// A view in `parts.wb` of a component the design never instantiates; only
/// the file's connector type is used, so nothing else flags it.
const UNINSTANTIATED_SUBJECT_ROOT: &str = "tests/fixtures/uninstantiated_subject_project";

#[test]
fn check_rejects_a_view_of_a_component_the_design_never_instantiates() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "check",
            "--strict",
            "--format",
            "json",
            UNINSTANTIATED_SUBJECT_ROOT,
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let parts = std::fs::read_to_string(format!("{UNINSTANTIATED_SUBJECT_ROOT}/parts.wb"))
        .expect("fixture parts.wb");
    let [problem] = problems.as_slice() else {
        panic!("expected one problem, got {problems:?}");
    };
    let span = &problem["labels"][0]["span"];
    let offset = span["offset"].as_u64().expect("offset") as usize;
    let length = span["length"].as_u64().expect("length") as usize;
    assert_eq!(problem["code"], "wirebug::uninstantiated_view_subject");
    assert_eq!(problem["severity"], "error");
    assert!(
        problem["filename"]
            .as_str()
            .expect("filename")
            .ends_with("parts.wb")
    );
    assert_eq!(&parts[offset..offset + length], "\"Spare Detail\"");
}

#[test]
fn render_stops_at_check_instead_of_failing_on_an_uninstantiated_subject() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("out");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            UNINSTANTIATED_SUBJECT_ROOT,
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("uninstantiated_view_subject"))
        .stderr(predicate::str::contains("not rendering"))
        .stderr(predicate::str::contains("has no instance in the design").not());
    assert!(!out.exists(), "nothing is written");
}

/// Ports, instances, a connector and a cable named with contextual keywords
/// (`at`, `from`, `fragment`, `size`, `grid`, `layout`, `text`, `shield`,
/// `splice`, `face`).
const CONTEXTUAL_NAMES_ROOT: &str = "tests/fixtures/contextual_names_project";

#[test]
fn check_accepts_contextual_keywords_as_names() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", CONTEXTUAL_NAMES_ROOT])
        .assert()
        .success();
}

#[test]
fn render_draws_a_project_named_with_contextual_keywords() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("out");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            CONTEXTUAL_NAMES_ROOT,
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
    let overview = std::fs::read_to_string(out.join("overview.svg")).expect("overview.svg");
    assert!(overview.contains("FROM"), "port label drawn");
    assert!(
        overview.contains("Two sensors on one loom"),
        "text box drawn"
    );
    assert!(out.join("loom.svg").exists(), "harness view drawn");
}

/// A schematic view with one text box in the current form and one in the
/// deprecated named form.
const DEPRECATED_TEXT_ROOT: &str = "tests/fixtures/deprecated_text_project";

#[test]
fn check_warns_on_a_text_box_in_the_deprecated_named_form() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", DEPRECATED_TEXT_ROOT])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main =
        std::fs::read_to_string(format!("{DEPRECATED_TEXT_ROOT}/main.wb")).expect("fixture file");

    // One warning, for the named box only, labelled on its statement, its
    // help the statement rewritten with the author's label and position.
    let [problem] = problems.as_slice() else {
        panic!("expected one problem, got {problems:?}");
    };
    assert_eq!(problem["code"], "wirebug::deprecated_text_syntax");
    assert_eq!(problem["severity"], "warning");
    let [label] = problem["labels"].as_array().expect("labels").as_slice() else {
        panic!("expected one label: {problem}");
    };
    let offset = label["span"]["offset"].as_u64().expect("offset") as usize;
    let length = label["span"]["length"].as_u64().expect("length") as usize;
    assert_eq!(
        &main[offset..offset + length],
        "text old_note at (26, 14) \"Deprecated form\";"
    );
    assert!(
        problem["help"]
            .as_str()
            .expect("help")
            .contains("`text \"Deprecated form\" at (26, 14);`"),
        "{problem}"
    );
}

#[test]
fn check_strict_fails_on_a_text_box_in_the_deprecated_named_form() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict", DEPRECATED_TEXT_ROOT])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "text box `old_note` is written in the deprecated named form",
        ));
}

#[test]
fn render_draws_both_text_box_forms_and_keeps_the_named_box_data_text() {
    for mode in [None, Some("--embed")] {
        let tmp = tempdir().expect("tempdir");
        let out = tmp.path().join("out");
        let mut args = vec![
            "render",
            DEPRECATED_TEXT_ROOT,
            "--out",
            out.to_str().unwrap(),
        ];
        args.extend(mode);

        Command::cargo_bin("wirebug")
            .expect("binary present")
            .args(&args)
            .assert()
            .success();
        let svg = std::fs::read_to_string(out.join("bench.svg")).expect("bench.svg");
        let boxes: Vec<&str> = svg
            .match_indices("<g class=\"text-box\"")
            .map(|(at, _)| &svg[at..at + svg[at..].find('>').expect("tag ends")])
            .collect();
        // Source order: the current form has no name to carry; the named
        // box keeps the `data-text` a host may style against.
        assert_eq!(
            boxes,
            [
                "<g class=\"text-box\"",
                "<g class=\"text-box\" data-text=\"old_note\""
            ],
            "{mode:?}"
        );
        assert!(svg.contains("Current form"), "{mode:?}");
        assert!(svg.contains("Deprecated form"), "{mode:?}");
    }
}

/// A port named with the reserved word `wire`.
const RESERVED_WORD_ROOT: &str = "tests/fixtures/reserved_word_project";

#[test]
fn check_says_a_reserved_word_cannot_be_a_name() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", RESERVED_WORD_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");
    let main =
        std::fs::read_to_string(format!("{RESERVED_WORD_ROOT}/main.wb")).expect("fixture main.wb");
    let [problem] = problems.as_slice() else {
        panic!("expected one problem, got {problems:?}");
    };
    assert_eq!(problem["code"], "wirebug::reserved_word");
    assert_eq!(problem["severity"], "error");
    assert_eq!(problem["message"], "'wire' is a reserved word");
    assert!(
        problem["help"]
            .as_str()
            .is_some_and(|help| help.contains("rename it")),
        "{problem}"
    );
    let span = &problem["labels"][0]["span"];
    let offset = span["offset"].as_u64().expect("offset") as usize;
    let length = span["length"].as_u64().expect("length") as usize;
    assert_eq!(&main[offset..offset + length], "wire");
}

#[test]
fn check_text_output_explains_the_reserved_word() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", RESERVED_WORD_ROOT])
        .assert()
        .failure()
        .stderr(predicate::str::contains("'wire' is a reserved word"))
        .stderr(predicate::str::contains("help:"));
}

/// Two broken statements in `lamp.wb` and three in `main.wb` (one a grid
/// with a unit), plus a wire of gauge zero in `main.wb`.
const SYNTAX_ERRORS_ROOT: &str = "tests/fixtures/syntax_errors_project";

#[test]
fn check_reports_every_syntax_error_and_what_else_is_wrong() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--format", "json", SYNTAX_ERRORS_ROOT])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let problems: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("check prints a JSON array");

    // Each problem as (file, code, the text under its label, its line).
    let found: Vec<(String, String, String, usize)> = problems
        .iter()
        .map(|p| {
            let path = p["filename"].as_str().expect("filename");
            let src = std::fs::read_to_string(path).expect("fixture file");
            let span = &p["labels"][0]["span"];
            let offset = span["offset"].as_u64().expect("offset") as usize;
            let length = span["length"].as_u64().expect("length") as usize;
            let file = std::path::Path::new(path)
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned();
            (
                file,
                p["code"].as_str().expect("code").to_string(),
                src[offset..offset + length].to_string(),
                src[..offset].lines().count(),
            )
        })
        .collect();
    let at = |file: &str, code: &str, text: &str, line: usize| {
        (file.to_string(), code.to_string(), text.to_string(), line)
    };
    // No `unknown_port` for `lamp.anode`, whose declaration is the broken
    // line 4 of lamp.wb, and no `unknown_include` cascade either.
    assert_eq!(
        found,
        [
            at("main.wb", "wirebug::parse", "[", 12),
            at("main.wb", "wirebug::parse", "2", 16),
            at("main.wb", "wirebug::parse", "1cm", 17),
            at("lamp.wb", "wirebug::parse", ";", 4),
            at("lamp.wb", "wirebug::parse", ";", 6),
            at("main.wb", "wirebug::invalid_gauge", "0", 11),
        ]
    );
    let unit = problems
        .iter()
        .find(|p| p["labels"][0]["span"]["length"] == 3)
        .expect("the grid's unit");
    assert_eq!(
        unit["message"],
        "only a `length` takes a unit: write `1`, without the `cm`"
    );
}

#[test]
fn render_refuses_a_project_with_syntax_errors() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("svg");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", SYNTAX_ERRORS_ROOT, "--out", out.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not rendering"));
    let drawn = std::fs::read_dir(&out).map_or(0, |entries| entries.count());
    assert_eq!(drawn, 0, "nothing is drawn");
}

const WIRE_LIST_ROOT: &str = "tests/fixtures/wire_list_project";
const PIN_TABLE_ROOT: &str = "tests/fixtures/pin_table_project";

/// Run `wirebug export <args>`, returning (success, stdout, stderr).
fn export(args: &[&str]) -> (bool, String, String) {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("export")
        .args(args)
        .output()
        .expect("runs");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("utf-8 stdout"),
        String::from_utf8(output.stderr).expect("utf-8 stderr"),
    )
}

#[test]
fn export_wires_prints_the_wire_list_as_csv_on_stdout() {
    let (ok, stdout, stderr) = export(&["wires", WIRE_LIST_ROOT]);
    assert!(ok, "{stderr}");
    let mut lines = stdout.lines();
    let header = lines.next().expect("a header");
    assert!(header.starts_with("kind,owner,cable,"), "{header}");
    assert!(header.ends_with(",length_m,rail_length_m,rail_segment"));
    // 16 conductors (a rail of four ends is three), two screens, and the
    // sub-loom's wire once per placement.
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 19, "{stdout}");
    assert_eq!(rows.iter().filter(|r| r.starts_with("shield,")).count(), 2);
    // Nothing but the table on stdout; a clean project says nothing.
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn export_connectors_prints_a_row_per_cavity_entry() {
    let (ok, stdout, stderr) = export(&["connectors", PIN_TABLE_ROOT]);
    assert!(ok, "{stderr}");
    let mut lines = stdout.lines();
    let header = lines.next().expect("a header");
    assert!(header.starts_with("instance,connector,half,"), "{header}");
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 43, "{stdout}");
    // A conductor into a port with no pin is listed, its pin empty.
    assert!(
        rows.iter()
            .any(|r| r.starts_with("Vehicle.ecu,x1,,Ecu12,ECU 12p,ECU-12,,,aux,AUX,conductor,")),
        "{stdout}"
    );
    // An unused cavity is a row of its own.
    assert!(
        rows.iter()
            .any(|r| r.starts_with("Vehicle.ecu,x1,,Ecu12,ECU 12p,ECU-12,7,,,,empty,"))
    );
}

#[test]
fn export_json_carries_the_format_version_and_the_project() {
    for (table, root, data) in [
        ("wires", WIRE_LIST_ROOT, "rows"),
        ("connectors", PIN_TABLE_ROOT, "connectors"),
    ] {
        let (ok, stdout, stderr) = export(&[table, root, "--format", "json"]);
        assert!(ok, "{stderr}");
        let document: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
        assert_eq!(document["table"], table);
        assert_eq!(document["format_version"], 1);
        assert!(document["project"]["name"].is_string());
        assert_eq!(
            document["generator"],
            concat!("wirebug v", env!("CARGO_PKG_VERSION"))
        );
        assert!(document[data].is_array(), "{table}: {stdout}");
    }
}

#[test]
fn export_out_writes_the_file_and_prints_a_summary_instead() {
    let tmp = tempdir().expect("tempdir");
    let path = tmp.path().join("wires.csv");
    let (ok, stdout, stderr) = export(&["wires", WIRE_LIST_ROOT, "--out", path.to_str().unwrap()]);
    assert!(ok, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr.trim_end(),
        format!("exported 19 wire-list row(s) as csv to {}", path.display())
    );
    let written = std::fs::read_to_string(&path).expect("written");
    let (_, printed, _) = export(&["wires", WIRE_LIST_ROOT]);
    assert_eq!(written, printed);
}

#[test]
fn export_refuses_a_project_with_errors_and_prints_no_table() {
    let (ok, stdout, stderr) = export(&["wires", SYNTAX_ERRORS_ROOT]);
    assert!(!ok);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("not exporting"), "{stderr}");
}

#[test]
fn export_strict_refuses_a_project_with_warnings() {
    let (ok, stdout, stderr) = export(&["connectors", UNDRAWN_CONDUCTORS_ROOT]);
    assert!(ok, "warnings do not block: {stderr}");
    assert!(stdout.starts_with("instance,"));
    assert!(
        stderr.contains("undrawn"),
        "the warnings go to stderr: {stderr}"
    );

    let (ok, stdout, stderr) = export(&["connectors", UNDRAWN_CONDUCTORS_ROOT, "--strict"]);
    assert!(!ok);
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("— not exporting"), "{stderr}");
}

#[test]
fn export_help_lists_its_tables_and_flags() {
    let help = |args: &[&str]| {
        let output = Command::cargo_bin("wirebug")
            .expect("binary present")
            .args(args)
            .output()
            .expect("runs");
        assert!(output.status.success());
        String::from_utf8(output.stdout).expect("utf-8")
    };
    insta::assert_snapshot!("export_help", help(&["export", "--help"]));
    insta::assert_snapshot!("export_wires_help", help(&["export", "wires", "--help"]));
}
