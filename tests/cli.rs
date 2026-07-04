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

#[test]
fn check_accepts_a_manifest_target() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", FIXTURE_MANIFEST])
        .assert()
        .success();
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
