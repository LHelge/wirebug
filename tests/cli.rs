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

/// Copy every file under `from` to the same place under `to`.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("copy target");
    for entry in std::fs::read_dir(from).expect("copy source") {
        let path = entry.expect("entry").path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("file copied");
        }
    }
}

#[test]
fn render_with_no_arguments_writes_into_out_in_the_project_root() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("basic_project");
    copy_tree(std::path::Path::new(FIXTURE_ROOT), &root);

    // Run from a subdirectory: the default is the project's `out`, not the
    // current directory's.
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("render")
        .current_dir(root.join("components"))
        .assert()
        .success()
        .stderr(predicate::str::contains("rendered"));

    assert!(root.join("out/system_overview.svg").is_file());
    assert!(root.join("out/index.html").is_file());
    assert!(!root.join("components/out").exists());
}

#[test]
fn render_of_a_named_project_defaults_to_its_root_not_the_current_directory() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("basic_project");
    copy_tree(std::path::Path::new(FIXTURE_ROOT), &root);
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("cwd created");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", root.join("wirebug.toml").to_str().unwrap()])
        .current_dir(&elsewhere)
        .assert()
        .success();

    assert!(root.join("out/system_overview.svg").is_file());
    assert!(!elsewhere.join("out").exists());
}

#[test]
fn render_with_a_relative_out_writes_relative_to_the_current_directory() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("basic_project");
    copy_tree(std::path::Path::new(FIXTURE_ROOT), &root);

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", "--out", "drawings"])
        .current_dir(root.join("components"))
        .assert()
        .success();

    assert!(
        root.join("components/drawings/system_overview.svg")
            .is_file()
    );
    assert!(!root.join("out").exists());
}

#[test]
fn render_help_says_where_out_defaults_to() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", "--help"])
        .output()
        .expect("runs");
    assert!(output.status.success());
    insta::assert_snapshot!(
        "render_help",
        String::from_utf8(output.stdout).expect("utf-8")
    );
}

/// A copy of the basic fixture in `dir` whose manifest authors its
/// revision. The stamp then never asks git, whose answer depends on the
/// environment: with `HOME` and `XDG_CONFIG_HOME` moved, git stops reading
/// the user's global excludes, files it ignored become untracked, and the
/// revision turns `-dirty`.
fn basic_fixture_with_fixed_revision(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("basic_project");
    copy_tree(std::path::Path::new(FIXTURE_ROOT), &root);
    let manifest = root.join("wirebug.toml");
    let mut text = std::fs::read_to_string(&manifest).expect("manifest read");
    text.push_str("revision = \"fixture\"\n");
    std::fs::write(&manifest, text).expect("manifest written");
    root
}

/// Every file `render <mode>` writes for the project at `root` into `out`,
/// by name. With `hide_fonts` the run sees no system font: fontconfig is
/// pointed at a config whose only font directory is empty, and `HOME` at an
/// empty directory, so a lookup of the system fonts would find none.
fn rendered_files(
    root: &std::path::Path,
    out: &std::path::Path,
    mode: &str,
    hide_fonts: bool,
) -> Vec<(String, Vec<u8>)> {
    let mut render = Command::cargo_bin("wirebug").expect("binary present");
    render.args([
        "render",
        root.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        mode,
    ]);
    let empty = out.with_extension("empty");
    if hide_fonts {
        std::fs::create_dir_all(&empty).expect("empty dir");
        let config = out.with_extension("conf");
        std::fs::write(
            &config,
            format!("<fontconfig><dir>{}</dir></fontconfig>", empty.display()),
        )
        .expect("fontconfig written");
        render
            .env("FONTCONFIG_FILE", &config)
            .env_remove("FONTCONFIG_PATH")
            .env("HOME", &empty)
            .env("XDG_DATA_HOME", &empty)
            .env("XDG_CONFIG_HOME", &empty);
    }
    render.assert().success();
    let mut files: Vec<_> = std::fs::read_dir(out)
        .expect("out dir")
        .map(|entry| {
            let path = entry.expect("entry").path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).expect("file read"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn png_and_pdf_output_is_the_same_with_or_without_system_fonts() {
    // Text is drawn with the bundled font alone, so hiding every system
    // font changes no byte. (With the system fonts loaded instead, the
    // hidden run found no font and dropped every label.)
    let tmp = tempdir().expect("tempdir");
    let root = basic_fixture_with_fixed_revision(tmp.path());
    for (mode, drawing) in [
        ("--png", "system_overview.png"),
        ("--pdf", "basic_project.pdf"),
    ] {
        let dir = mode.trim_start_matches('-');
        let seen = rendered_files(&root, &tmp.path().join(format!("{dir}-seen")), mode, false);
        let hidden = rendered_files(&root, &tmp.path().join(format!("{dir}-hidden")), mode, true);
        let names = |files: &[(String, Vec<u8>)]| -> Vec<String> {
            files.iter().map(|(name, _)| name.clone()).collect()
        };
        assert!(names(&seen).iter().any(|name| name == drawing));
        assert_eq!(names(&seen), names(&hidden));
        for ((name, seen), (_, hidden)) in seen.iter().zip(&hidden) {
            assert!(
                seen == hidden,
                "{name} differs when system fonts are hidden"
            );
        }
    }
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

const PAPER_SIZES_ROOT: &str = "tests/fixtures/paper_sizes_project";

/// Render `root` with `--pdf` plus `extra` into `out`, returning the
/// command's assertion for its exit status and stderr.
fn render_pdf(root: &str, out: &std::path::Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", root, "--out", out.to_str().unwrap(), "--pdf"])
        .args(extra)
        .assert()
}

/// Every page's `/MediaBox`, in the order the pages are written.
fn media_boxes(pdf: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(pdf);
    text.match_indices("/MediaBox [")
        .map(|(at, prefix)| {
            let rest = &text[at + prefix.len()..];
            rest[..rest.find(']').expect("closed box")].to_string()
        })
        .collect()
}

#[test]
fn render_pdf_lays_pages_on_a4_unless_told_otherwise() {
    let tmp = tempdir().expect("tempdir");
    render_pdf(FIXTURE_ROOT, &tmp.path().join("default"), &[]).success();
    render_pdf(FIXTURE_ROOT, &tmp.path().join("a4"), &["--paper", "a4"]).success();

    let default = std::fs::read(tmp.path().join("default/basic_project.pdf")).expect("pdf");
    let a4 = std::fs::read(tmp.path().join("a4/basic_project.pdf")).expect("pdf");
    assert!(default == a4, "`--paper a4` is the default");
    // A4 landscape or portrait, 210 × 297 mm in points.
    let boxes = media_boxes(&default);
    assert_eq!(boxes.len(), 4);
    for page in boxes {
        assert!(
            ["0 0 841.89 595.276", "0 0 595.276 841.89"].contains(&page.as_str()),
            "not A4: {page}"
        );
    }
}

#[test]
fn render_pdf_on_a3_lays_every_page_on_a3() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("a3");
    render_pdf(PAPER_SIZES_ROOT, &out, &["--paper", "a3"]).success();

    let pdf = std::fs::read(out.join("paper_sizes_project.pdf")).expect("pdf");
    // Both views are wider than tall: A3 landscape, 420 × 297 mm.
    assert_eq!(
        media_boxes(&pdf),
        ["0 0 1190.551 841.89", "0 0 1190.551 841.89"]
    );
}

#[test]
fn render_pdf_warns_of_a_view_too_wide_to_read_on_its_paper() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("a4");
    // The warning neither fails the run nor keeps the page out.
    render_pdf(PAPER_SIZES_ROOT, &out, &[]).success().stderr(
        predicate::str::contains(
            "warning: view \"Wide\": smallest text 3.6 pt on A4, below the 5 pt \
                 legibility minimum; legible on Tabloid (--paper tabloid)",
        )
        .and(predicate::str::contains("Compact").not())
        .and(predicate::str::contains("rendered 2 view(s) as pdf")),
    );
    let pdf = std::fs::read(out.join("paper_sizes_project.pdf")).expect("pdf");
    assert_eq!(media_boxes(&pdf).len(), 2, "the wide view keeps its page");

    render_pdf(PAPER_SIZES_ROOT, &tmp.path().join("a3"), &["--paper", "a3"])
        .success()
        .stderr(predicate::str::contains("warning").not());
}

#[test]
fn render_pdf_small_text_warning_does_not_fail_a_strict_run() {
    let tmp = tempdir().expect("tempdir");
    render_pdf(PAPER_SIZES_ROOT, &tmp.path().join("pdf"), &["--strict"])
        .success()
        .stderr(predicate::str::contains("warning: view \"Wide\""));
}

#[test]
fn render_rejects_paper_without_pdf() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            PAPER_SIZES_ROOT,
            "--out",
            "unused",
            "--paper",
            "a3",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the following required arguments were not provided",
        ))
        .stderr(predicate::str::contains("--pdf"));
}

#[test]
fn render_help_states_the_legibility_minimum_and_that_strict_ignores_it() {
    let minimum = wirebug::render::pdf::MIN_LEGIBLE_PT;
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("below {minimum} pt")))
        .stdout(predicate::str::contains("- a4:").and(predicate::str::contains("- tabloid:")))
        .stdout(predicate::str::contains("[default: a4]"))
        .stdout(predicate::str::contains("even under `--strict`"));
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

/// The value of `name` on `svg`'s root `<svg>` element, as written.
fn root_attribute<'s>(svg: &'s str, name: &str) -> Option<&'s str> {
    let start = svg.find("<svg ")?;
    let tag = &svg[start..start + svg[start..].find('>')?];
    let key = format!(" {name}=\"");
    let value = &tag[tag.find(&key)? + key.len()..];
    Some(&value[..value.find('"')?])
}

/// The width and height `svg`'s root viewBox spans, as written.
fn viewbox_size(svg: &str) -> (&str, &str) {
    let viewbox: Vec<&str> = root_attribute(svg, "viewBox")
        .expect("a viewBox")
        .split(' ')
        .collect();
    (viewbox[2], viewbox[3])
}

#[test]
fn render_sizes_every_view_one_world_unit_to_the_px() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("svg");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["render", FIXTURE_ROOT, "--out", out.to_str().unwrap()])
        .assert()
        .success();

    // Each kind of view carries its viewBox's size as its own, so a
    // browser shows every view at the same text size.
    for file in [
        "system_overview.svg",
        "main_harness.svg",
        "inverter_pinout.svg",
    ] {
        let svg = std::fs::read_to_string(out.join(file)).expect("view rendered");
        let (width, height) = viewbox_size(&svg);
        assert_eq!(root_attribute(&svg, "width"), Some(width), "{file}");
        assert_eq!(root_attribute(&svg, "height"), Some(height), "{file}");
    }

    // The index shows them as images at that natural size, a wider one
    // shrunk to the page.
    let index = std::fs::read_to_string(out.join("index.html")).expect("index written");
    let svg = std::fs::read_to_string(out.join("system_overview.svg")).expect("view rendered");
    assert!(
        index.contains(&index_image("system_overview.svg", "System Overview", &svg)),
        "{index}"
    );
    assert!(index.contains("img { max-width: 100%; height: auto; }"));
}

/// The `<img>` the HTML index shows `file` with, given the standalone
/// `svg` it was drawn as: sized to the viewBox in whole CSS px.
fn index_image(file: &str, title: &str, svg: &str) -> String {
    let (width, height) = viewbox_size(svg);
    let px = |side: &str| side.parse::<f64>().expect("a number").round();
    format!(
        "<img src=\"{file}\" alt=\"{title}\" width=\"{}\" height=\"{}\">",
        px(width),
        px(height)
    )
}

#[test]
fn render_png_index_shows_each_view_one_world_unit_to_the_px() {
    let tmp = tempdir().expect("tempdir");
    let (svg_out, png_out) = (tmp.path().join("svg"), tmp.path().join("png"));
    for (out, flags) in [(&svg_out, &[][..]), (&png_out, &["--png"][..])] {
        Command::cargo_bin("wirebug")
            .expect("binary present")
            .args(["render", FIXTURE_ROOT, "--out", out.to_str().unwrap()])
            .args(flags)
            .assert()
            .success();
    }

    // A PNG has two pixels a world unit, but the index shows it at the
    // size of its SVG, not at its pixel size.
    let svg = std::fs::read_to_string(svg_out.join("inverter_pinout.svg")).expect("svg");
    let index = std::fs::read_to_string(png_out.join("index.html")).expect("index written");
    assert!(
        index.contains(&index_image("inverter_pinout.png", "Inverter Pinout", &svg)),
        "{index}"
    );
}

#[test]
fn render_embed_leaves_the_size_to_the_host_and_gives_it_in_the_manifest() {
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

    let manifest_src =
        std::fs::read_to_string(out.join("manifest.json")).expect("manifest written");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_src).expect("manifest is valid JSON");
    let views = manifest["views"].as_array().expect("views array");
    for kind in ["schematic", "harness", "pinout"] {
        assert!(views.iter().any(|v| v["kind"] == kind), "a {kind} view");
    }
    for view in views {
        let file = view["filename"].as_str().expect("a file name");
        let svg = std::fs::read_to_string(out.join(file)).expect("view rendered");
        // No intrinsic size on the root: the host's CSS sizes the view.
        assert_eq!(root_attribute(&svg, "width"), None, "{file}");
        assert_eq!(root_attribute(&svg, "height"), None, "{file}");
        // Its natural size is in the manifest instead, the very numbers
        // of the viewBox. Read from the text: `serde_json` parses some
        // floats a last digit off.
        let (width, height) = viewbox_size(&svg);
        let json = |side: &str| {
            serde_json::to_string(&side.parse::<f64>().expect("a number")).expect("serializes")
        };
        let entry = &manifest_src[manifest_src
            .find(&format!("\"filename\": \"{file}\""))
            .expect("the view's entry")..];
        let size = format!(
            "\"width\": {},\n      \"height\": {}\n",
            json(width),
            json(height)
        );
        assert!(entry.contains(&size), "{file}: {size} in {entry}");
        assert!(
            entry.find(&size) < entry.find('}'),
            "{file}: {size} in its own entry"
        );
    }
}

const TIED_ROUTES_ROOT: &str = "tests/fixtures/tied_routes_project";

/// Each process seeds its hash maps afresh, so this catches any output
/// that follows hash order — here, the router settling a tie between
/// two equally good routes.
#[test]
fn render_writes_the_same_bytes_in_every_process() {
    let tmp = tempdir().expect("tempdir");
    let render = |run: usize| {
        let out = tmp.path().join(format!("run{run}"));
        Command::cargo_bin("wirebug")
            .expect("binary present")
            .args(["render", TIED_ROUTES_ROOT, "--out", out.to_str().unwrap()])
            .assert()
            .success();
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&out)
            .expect("output directory")
            .map(|entry| {
                let path = entry.expect("directory entry").path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(&path).expect("rendered file"))
            })
            .collect();
        files.sort();
        files
    };

    let first = render(0);
    assert!(first.iter().any(|(name, _)| name == "stacked.svg"));
    for run in 1..8 {
        let again = render(run);
        let names = |files: &[(String, Vec<u8>)]| -> Vec<String> {
            files.iter().map(|(name, _)| name.clone()).collect()
        };
        assert_eq!(names(&again), names(&first), "run {run} wrote other files");
        for ((name, bytes), (_, expected)) in again.iter().zip(&first) {
            assert!(bytes == expected, "run {run} wrote a different {name}");
        }
    }
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
    // A pinout's measured text widths are fractional: the size reads
    // rounded up to whole units, in the message and in its label.
    assert_eq!(
        problems[2]["message"],
        "view \"Service\" is 20380 × 20218 units, more than the 8192 a view may span on a side"
    );
    assert_eq!(problems[2]["labels"][0]["label"], "20380 × 20218 units");
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
const BOM_ROOT: &str = "tests/fixtures/bom_project";

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
        ("bom", BOM_ROOT, "lines"),
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
fn render_embed_writes_the_build_tables_the_manifest_names() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("embed");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            PIN_TABLE_ROOT,
            "--out",
            out.to_str().unwrap(),
            "--embed",
        ])
        .assert()
        .success();
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("manifest.json")).expect("manifest written"),
    )
    .expect("valid JSON");
    for table in ["wires", "connectors", "bom"] {
        let filename = manifest["exports"][table].as_str().expect("named");
        assert_eq!(filename, format!("{table}.json"));
        let written = std::fs::read_to_string(out.join(filename)).expect("table written");
        // The same document `export --format json` prints.
        let (_, printed, _) = export(&[table, PIN_TABLE_ROOT, "--format", "json"]);
        assert_eq!(written, printed, "{table}");
    }
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
    insta::assert_snapshot!("export_bom_help", help(&["export", "bom", "--help"]));
}

#[test]
fn export_bom_prints_a_row_per_thing_to_buy_with_its_quantity() {
    let (ok, stdout, stderr) = export(&["bom", BOM_ROOT]);
    assert!(ok, "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some(
            "kind,specified,part,type,label,description,manufacturer,rating,\
             color,color_code,gauge_mm2,quantity,total_length_m,unmeasured,boundary,where"
        )
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 22, "{stdout}");
    for expected in [
        // Seven lamps' housings and two inlines' male halves.
        "housing,true,DT04-2P,Dt04_2p,,Deutsch DT04-2P,,,,,,9,,,,",
        // An inline that declares no half is still two housings, listed
        // under its label.
        "housing,false,,,,Spare,,,,,,2,,,,Vehicle.ic_spare.male;Vehicle.ic_spare.female",
        // Two cables of one type: the one with a length in the total, the
        // other counted as unmeasured.
        "cable,true,,2x0.5 TP,,,,,,,,2,3,1,,Vehicle.can1;Vehicle.can2",
        // One of the nine conductors continues the vehicle's wire at the
        // pod's connectorless boundary port.
        "wire,false,,,,,,,red,RD,1,9,6.5,2,1,",
        "component,false,,Switch,,,,,,,,1,,,,Vehicle.sw",
        "splice,true,D-436,,,Solder sleeve,,,,,,3,,,,Vehicle.sp_gnd;Vehicle.front.sp_bar;Vehicle.rear.sp_bar",
    ] {
        assert!(
            rows.iter().any(|row| row.starts_with(expected)),
            "{expected}: {stdout}"
        );
    }
}

#[test]
fn export_bom_as_json_flags_the_unspecified_and_lists_the_placements() {
    let (ok, stdout, stderr) = export(&["bom", BOM_ROOT, "--format", "json"]);
    assert!(ok, "{stderr}");
    let document: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let lines = document["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 22);
    let lamps = lines
        .iter()
        .find(|line| line["part"] == "LMP-1")
        .expect("the lamps' line");
    assert_eq!(lamps["kind"], "component");
    assert_eq!(lamps["specified"], true);
    assert_eq!(lamps["quantity"], 7);
    assert_eq!(lamps["where"].as_array().map(Vec::len), Some(7));
    // A connector block that names no type is grouped by its description.
    let untyped = &lines[1];
    assert_eq!(untyped["kind"], "housing");
    assert_eq!(untyped["specified"], false);
    assert!(untyped["part"].is_null() && untyped["type"].is_null());
    assert_eq!(untyped["description"], "Relay 2p");
    assert_eq!(untyped["where"], serde_json::json!(["Vehicle.relay.x1"]));
    let red = lines
        .iter()
        .find(|line| line["kind"] == "wire" && line["color_code"] == "RD")
        .expect("the red wire's line");
    assert_eq!(red["boundary"], 1);
}

#[test]
fn export_bom_out_summarises_the_rows_written() {
    let tmp = tempdir().expect("tempdir");
    let path = tmp.path().join("bom.csv");
    let (ok, stdout, stderr) = export(&["bom", BOM_ROOT, "--out", path.to_str().unwrap()]);
    assert!(ok, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr.trim_end(),
        format!("exported 22 BOM row(s) as csv to {}", path.display())
    );
}

#[test]
fn render_embed_writes_the_bom_beside_the_manifest() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("embed");
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args([
            "render",
            BOM_ROOT,
            "--out",
            out.to_str().unwrap(),
            "--embed",
        ])
        .assert()
        .success();
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("manifest.json")).expect("manifest written"),
    )
    .expect("valid JSON");
    assert_eq!(manifest["exports"]["bom"], "bom.json");
    let written = std::fs::read_to_string(out.join("bom.json")).expect("bom written");
    let (_, printed, _) = export(&["bom", BOM_ROOT, "--format", "json"]);
    assert_eq!(written, printed);
}

/// Run `wirebug <args>` with its stdout a pipe whose reader is gone before
/// anything is written, as when the output is piped into `head` and `head`
/// has stopped reading; return (success, stderr).
fn run_into_closed_pipe(args: &[&str]) -> (bool, String) {
    use assert_cmd::cargo::CommandCargoExt;
    use std::io::Read;
    use std::process::Stdio;

    let mut child = std::process::Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawns");
    // Closing the read end now makes the command's first write to stdout
    // fail with a broken pipe: it checks the project before it writes.
    drop(child.stdout.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr piped")
        .read_to_string(&mut stderr)
        .expect("utf-8 stderr");
    (child.wait().expect("exits").success(), stderr)
}

#[test]
fn a_reader_that_stops_early_is_not_an_error() {
    for table in ["wires", "connectors", "bom"] {
        for format in ["csv", "json"] {
            let (ok, stderr) =
                run_into_closed_pipe(&["export", table, PIN_TABLE_ROOT, "--format", format]);
            assert!(ok, "export {table} --format {format}: {stderr}");
            assert!(!stderr.contains("error"), "{stderr}");
        }
    }
    let (ok, stderr) =
        run_into_closed_pipe(&["check", "--format", "json", UNDRAWN_CONDUCTORS_ROOT]);
    assert!(ok, "check --format json: {stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn init_scaffolds_a_project_that_checks_strictly_and_renders() {
    let tmp = tempdir().expect("tempdir");
    let dir = tmp.path().join("new").join("lamp-test");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("init")
        .arg(&dir)
        .assert()
        .success()
        .stderr(predicate::str::contains(format!(
            "created {}",
            dir.join("wirebug.toml").display()
        )))
        .stderr(predicate::str::contains(format!(
            "created {}",
            dir.join("main.wb").display()
        )))
        .stderr(predicate::str::contains(format!(
            "created {}",
            dir.join(".gitignore").display()
        )))
        .stderr(predicate::str::contains("next: `wirebug check "));

    let manifest = std::fs::read_to_string(dir.join("wirebug.toml")).expect("manifest");
    assert!(manifest.contains("name = \"lamp-test\""), "{manifest}");
    assert!(manifest.contains("version = \"0.1.0\""), "{manifest}");
    let gitignore = std::fs::read_to_string(dir.join(".gitignore")).expect(".gitignore");
    assert!(gitignore.lines().any(|line| line == "out/"), "{gitignore}");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["check", "--strict"])
        .arg(&dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("ok — 3 instances, 1 views"));

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("render")
        .arg(&dir)
        .assert()
        .success();
    let svg = std::fs::read_to_string(dir.join("out").join("overview.svg")).expect("view rendered");
    assert!(svg.contains("Sensor"));
    assert!(svg.contains("Controller"));
    assert!(svg.contains("class=\"wire\""));
}

#[test]
fn init_with_no_directory_scaffolds_the_current_one() {
    let tmp = tempdir().expect("tempdir");
    let dir = tmp.path().join("bench rig");
    std::fs::create_dir(&dir).expect("mkdir");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("init")
        .current_dir(&dir)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "next: `wirebug check`, then `wirebug render`",
        ));

    let manifest = std::fs::read_to_string(dir.join("wirebug.toml")).expect("manifest");
    assert!(manifest.contains("name = \"bench-rig\""), "{manifest}");
    assert!(dir.join("main.wb").is_file());
}

#[test]
fn init_refuses_a_directory_that_already_has_a_manifest() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("wirebug.toml"), "[project]\n").expect("write");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("init")
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "{} already exists; not overwriting it",
            tmp.path().join("wirebug.toml").display()
        )));

    assert_eq!(
        std::fs::read_to_string(tmp.path().join("wirebug.toml")).expect("read"),
        "[project]\n"
    );
    assert!(!tmp.path().join("main.wb").exists());
    assert!(!tmp.path().join(".gitignore").exists());
}

#[test]
fn init_refuses_a_directory_that_already_has_a_main_wb() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("main.wb"), "// mine\n").expect("write");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("init")
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("main.wb already exists"));

    assert!(!tmp.path().join("wirebug.toml").exists());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("main.wb")).expect("read"),
        "// mine\n"
    );
}

#[test]
fn init_keeps_an_existing_gitignore() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join(".gitignore"), "target/\n").expect("write");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("init")
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains(".gitignore").not());

    assert_eq!(
        std::fs::read_to_string(tmp.path().join(".gitignore")).expect("read"),
        "target/\n"
    );
}

#[test]
fn help_text_mentions_init() {
    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"));
}

#[test]
fn init_help_says_what_it_writes_and_refuses() {
    let output = Command::cargo_bin("wirebug")
        .expect("binary present")
        .args(["init", "--help"])
        .output()
        .expect("runs");
    assert!(output.status.success());
    insta::assert_snapshot!(
        "init_help",
        String::from_utf8(output.stdout).expect("utf-8")
    );
}

#[test]
fn init_refuses_a_path_that_is_a_file() {
    let tmp = tempdir().expect("tempdir");
    let file = tmp.path().join("afile");
    std::fs::write(&file, "mine").expect("write");

    Command::cargo_bin("wirebug")
        .expect("binary present")
        .arg("init")
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "{} is not a directory; cannot create a project in it",
            file.display()
        )));

    assert_eq!(std::fs::read_to_string(&file).expect("read"), "mine");
}
