//! Black-box CLI tests.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

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
fn check_accepts_the_example_project() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "examples/main.wb"])
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
