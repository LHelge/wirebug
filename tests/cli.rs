//! Black-box CLI tests.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn render_writes_an_svg_per_view() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("svg");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", "examples/main.wb", "--out", out.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("rendered"));

    // The top-level view lands as a slug of its title and is a real SVG
    // with its title and routed wires.
    let svg =
        std::fs::read_to_string(out.join("hv_system_overview.svg")).expect("hv view rendered");
    assert!(svg.contains("<svg"));
    assert!(svg.contains("HV System Overview"));
    assert!(svg.contains("class=\"wire\""));
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
