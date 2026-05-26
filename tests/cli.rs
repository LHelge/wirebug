//! Black-box CLI tests.

use assert_cmd::Command;
use predicates::prelude::*;

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
fn check_reports_not_yet_implemented() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "examples/main.wb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet implemented"));
}
