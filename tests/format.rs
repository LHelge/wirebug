//! `fmt` over every fixture: each `.wb` file that parses formats to a text
//! that formats to itself and means what the file did.

mod common;

use std::path::{Path, PathBuf};

/// Every `.wb` file under `dir`, sorted.
fn wb_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            files.extend(wb_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "wb") {
            files.push(path);
        }
    }
    files.sort();
    files
}

#[test]
fn every_fixture_formats_idempotently_to_the_same_ast() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut problems = Vec::new();
    let mut formatted = 0;
    for path in wb_files(&fixtures) {
        let src = std::fs::read_to_string(&path).expect("fixture readable");
        match common::format_faithfully(&src) {
            Some(Ok(())) => formatted += 1,
            Some(Err(problem)) => problems.push(format!("{}: {problem}", path.display())),
            None => {}
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n\n"));
    assert!(
        formatted > 60,
        "only {formatted} fixture files formatted; the walk missed the corpus"
    );
}
