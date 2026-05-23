//! Black-box CLI tests.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn render_example_writes_svg_and_warns_on_unconnected_ports() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("hv.svg");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            "--model",
            "examples/model.yaml",
            "--view",
            "examples/views/hv_overview.yaml",
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("contactor.coil.pos"))
        .stderr(predicate::str::contains("contactor.coil.neg"));

    let svg = fs::read_to_string(&out).expect("output written");
    assert!(svg.contains("<svg"));
    assert!(svg.contains("HV Power Path"));
}

#[test]
fn missing_model_reports_error() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            "--model",
            "examples/does-not-exist.yaml",
            "--view",
            "examples/views/hv_overview.yaml",
            "--out",
            "/tmp/wirebug-should-not-exist.svg",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
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
