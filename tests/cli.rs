//! Black-box CLI tests.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

const FIXTURE_MAIN: &str = "tests/fixtures/basic_project/main.wb";

#[test]
fn render_writes_an_svg_per_view() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("svg");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", FIXTURE_MAIN, "--out", out.to_str().unwrap()])
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
            FIXTURE_MAIN,
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
fn render_no_stamp_omits_the_corner_stamp() {
    let tmp = tempdir().expect("tempdir");
    let out_with = tmp.path().join("with");
    let out_without = tmp.path().join("without");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            "examples/main.wb",
            "--out",
            out_with.to_str().unwrap(),
        ])
        .assert()
        .success();

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            "examples/main.wb",
            "--out",
            out_without.to_str().unwrap(),
            "--no-stamp",
        ])
        .assert()
        .success();

    let with =
        std::fs::read_to_string(out_with.join("hv_system_overview.svg")).expect("hv view rendered");
    let without = std::fs::read_to_string(out_without.join("hv_system_overview.svg"))
        .expect("hv view rendered");
    assert!(with.contains("class=\"stamp\""));
    assert!(with.contains("aphid-evpack v0.1.0"));
    assert!(!without.contains("class=\"stamp\""));
    assert!(!without.contains("aphid-evpack v0.1.0"));
}

#[test]
fn render_rejects_a_project_that_does_not_check() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    std::fs::write(&main, "use missing from \"nope.wb\"\ncomponent c { }\n")
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
        .success();

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
fn check_accepts_the_fixture_project() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", FIXTURE_MAIN])
        .assert()
        .success();
}

#[test]
fn check_rejects_a_dangling_use() {
    let tmp = tempdir().expect("tempdir");
    let main = tmp.path().join("main.wb");
    std::fs::write(
        &main,
        "use missing from \"nope.wb\"\ncomponent c { pub port a \"A\"; }\n",
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
component m { leaf l; }
use leaf from "leaf.wb"
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
