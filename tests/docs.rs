//! The language reference (`docs/`) and the agent skill
//! (`.claude/skills/wirebug-dsl/SKILL.md`) show only `.wb` code the parser
//! accepts.
//!
//! Every fenced block in those files carries an info string, so no `.wb`
//! code can hide in an untagged block. A block tagged `wb` is checked:
//!
//! - ` ```wb ` — a whole file;
//! - ` ```wb component ` — statements of a component body, checked inside
//!   `component Example { … }`;
//! - ` ```wb view ` — items of a view body, checked inside
//!   `view schematic "Example" { … }`;
//! - `error` after any of these (` ```wb error `, ` ```wb component error `)
//!   — an example of a mistake, which must fail to parse.
//!
//! Any other tag (`text`, `toml`, `sh`) is not `.wb` and is not checked.
//!
//! Both also list the language's keywords, each list a paragraph of nothing
//! but backticked words, and between them the lists of each must name every
//! keyword the lexer knows and nothing else, so neither can fall behind the
//! language or the other.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use wirebug::dsl::lex::{Token, lex, significant};
use wirebug::dsl::parse::{Completeness, parse_file};
use wirebug::dsl::span::FileId;

const DOCS_DIR: &str = "docs";
const SKILL: &str = ".claude/skills/wirebug-dsl/SKILL.md";

/// What a `wb` block holds, and so what it is wrapped in to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    File,
    Component,
    View,
}

impl Context {
    fn wrap(self, body: &str) -> String {
        match self {
            Context::File => body.to_owned(),
            Context::Component => format!("component Example {{\n{body}\n}}\n"),
            Context::View => format!("view schematic \"Example\" {{\n{body}\n}}\n"),
        }
    }
}

/// A `wb` block's tag: its context, and whether it shows a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WbTag {
    context: Context,
    error: bool,
}

impl WbTag {
    /// The tag of a block whose info string is `info`: `None` for a block
    /// that is not `.wb`, an error for a `wb` info string with a word this
    /// convention does not know.
    fn parse(info: &str) -> Result<Option<WbTag>, String> {
        let mut words = info.split_whitespace();
        if words.next() != Some("wb") {
            return Ok(None);
        }
        let mut tag = WbTag {
            context: Context::File,
            error: false,
        };
        for word in words {
            match word {
                "component" if tag.context == Context::File => tag.context = Context::Component,
                "view" if tag.context == Context::File => tag.context = Context::View,
                "error" if !tag.error => tag.error = true,
                other => return Err(format!("unknown word `{other}` in info string `{info}`")),
            }
        }
        Ok(Some(tag))
    }
}

/// One fenced block of a Markdown file.
#[derive(Debug)]
struct Block {
    /// The 1-based line of the opening fence.
    line: usize,
    info: String,
    body: String,
}

/// Every fenced (```` ``` ````) block of `markdown`, in order. A fence may
/// be indented (inside a list item).
fn fenced_blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut open: Option<Block> = None;
    for (index, line) in markdown.lines().enumerate() {
        let fence = line.trim_start().strip_prefix("```");
        match (fence, open.as_mut()) {
            (Some(_), Some(_)) => blocks.extend(open.take()),
            (Some(info), None) => {
                open = Some(Block {
                    line: index + 1,
                    info: info.trim().to_owned(),
                    body: String::new(),
                });
            }
            (None, Some(block)) => {
                block.body.push_str(line);
                block.body.push('\n');
            }
            (None, None) => {}
        }
    }
    blocks
}

/// Whether `src` lexes and parses without a single error.
fn parses(src: &str) -> bool {
    let Ok(lexemes) = lex(src, FileId(0)) else {
        return false;
    };
    let parsed = parse_file(significant(&lexemes), FileId(0), src);
    parsed.is_ok() && parsed.completeness == Completeness::Whole
}

/// The problems of the blocks of `markdown` (from `path`, for the messages),
/// and how many `wb` blocks it holds.
fn check_markdown(path: &Path, markdown: &str) -> (Vec<String>, usize) {
    let mut problems = Vec::new();
    let mut checked = 0;
    for block in fenced_blocks(markdown) {
        let at = format!("{}:{}", path.display(), block.line);
        if block.info.is_empty() {
            problems.push(format!(
                "{at}: fenced block has no info string (tag `.wb` code `wb`, anything else `text`)"
            ));
            continue;
        }
        match WbTag::parse(&block.info) {
            Err(message) => problems.push(format!("{at}: {message}")),
            Ok(None) => {}
            Ok(Some(tag)) => {
                checked += 1;
                let ok = parses(&tag.context.wrap(&block.body));
                if ok && tag.error {
                    problems.push(format!(
                        "{at}: block is tagged `error` but parses; drop the tag"
                    ));
                } else if !ok && !tag.error {
                    problems.push(format!(
                        "{at}: block does not parse (`wirebug check` shows why); \
                         fix it, or tag it `error` if it shows a mistake"
                    ));
                }
            }
        }
    }
    (problems, checked)
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Every Markdown file under `dir`, sorted.
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            files.extend(markdown_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "md") {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// Checks every file of `files`, failing with every problem at once.
fn assert_blocks_parse(files: &[PathBuf]) {
    let mut problems = Vec::new();
    for path in files {
        let markdown =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let shown = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(path);
        let (found, checked) = check_markdown(shown, &markdown);
        problems.extend(found);
        if checked == 0 && shown.starts_with(DOCS_DIR) && !shown.ends_with("README.md") {
            problems.push(format!("{}: holds no `wb` block", shown.display()));
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

#[test]
fn every_wb_block_in_the_reference_parses() {
    let files = markdown_files(&repo_path(DOCS_DIR));
    assert!(
        files.len() > 1,
        "the reference is the Markdown files under docs/"
    );
    assert_blocks_parse(&files);
}

#[test]
fn every_wb_block_in_the_skill_parses() {
    assert_blocks_parse(&[repo_path(SKILL)]);
}

#[test]
fn skill_does_not_point_at_this_repository() {
    // The skill is installed into other repositories, where none of these
    // exist.
    let skill = std::fs::read_to_string(repo_path(SKILL)).expect("skill readable");
    for pointer in ["docs/", "examples/", "tests/fixtures", "cargo run"] {
        assert!(
            !skill.contains(pointer),
            "the skill must stay self-contained, but it names `{pointer}`"
        );
    }
}

#[test]
fn skill_fits_its_context_budget() {
    // The skill is loaded whole into an agent's context whenever a `.wb`
    // file is touched.
    let bytes = std::fs::metadata(repo_path(SKILL))
        .expect("skill readable")
        .len();
    assert!(bytes <= 20_000, "the skill is {bytes} bytes, over 20 000");
}

/// The keyword lists of `markdown`: each paragraph, outside fenced blocks,
/// made of nothing but backticked names (`` `use` `component` … ``).
fn keyword_lists(markdown: &str) -> Vec<Vec<String>> {
    let mut lists = Vec::new();
    let mut paragraph = String::new();
    let mut in_fence = false;
    for line in markdown.lines().chain([""]) {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if line.trim().is_empty() {
            lists.extend(backticked_names(&paragraph));
            paragraph.clear();
        } else {
            paragraph.push_str(line);
            paragraph.push(' ');
        }
    }
    lists
}

/// The names of `paragraph` when it is only backticked names, else `None`.
fn backticked_names(paragraph: &str) -> Option<Vec<String>> {
    let names: Option<Vec<String>> = paragraph
        .split_whitespace()
        .map(|word| {
            let name = word.strip_prefix('`')?.strip_suffix('`')?;
            let name_shaped =
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            name_shaped.then(|| name.to_owned())
        })
        .collect();
    names.filter(|names| names.len() > 1)
}

/// The token `word` lexes to, when it is a keyword.
fn keyword_token(word: &str) -> Option<Token> {
    let lexemes = lex(word, FileId(0)).ok()?;
    match significant(&lexemes).as_slice() {
        [(token, _)] if !matches!(token, Token::Ident(_)) => Some(token.clone()),
        _ => None,
    }
}

/// What is wrong with the keyword lists of `markdowns` (named `what` in
/// the messages): a listed word the lexer does not know, a list mixing
/// reserved and contextual words, a word listed twice, or a keyword not
/// listed.
fn keyword_list_problems(what: &str, markdowns: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut listed = BTreeSet::new();
    for markdown in markdowns {
        for list in keyword_lists(markdown) {
            let mut reserved = BTreeSet::new();
            for word in list {
                match keyword_token(&word) {
                    None => problems.push(format!(
                        "{what}: lists `{word}` among the keywords, but it is no keyword"
                    )),
                    Some(token) => {
                        reserved.insert(token.is_reserved());
                    }
                }
                if !listed.insert(word.clone()) {
                    problems.push(format!("{what}: lists the keyword `{word}` twice"));
                }
            }
            if reserved.len() > 1 {
                problems.push(format!(
                    "{what}: a keyword list mixes reserved and contextual words"
                ));
            }
        }
    }
    for keyword in Token::keywords() {
        if !listed.contains(keyword) {
            problems.push(format!("{what}: does not list the keyword `{keyword}`"));
        }
    }
    problems
}

fn read_all(files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .map(|path| {
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

#[test]
fn the_reference_and_the_skill_list_exactly_the_lexers_keywords() {
    let docs = read_all(&markdown_files(&repo_path(DOCS_DIR)));
    let mut problems = keyword_list_problems("docs/", &docs);
    problems.extend(keyword_list_problems(
        "the skill",
        &read_all(&[repo_path(SKILL)]),
    ));
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

#[test]
fn a_keyword_list_is_a_paragraph_of_backticked_names() {
    let markdown = "Reserved:\n\n`use` `component`\n`view`\n\n\
                    Colors `red` `blue` are words.\n\n```text\n`a` `b`\n```\n";
    assert_eq!(
        keyword_lists(markdown),
        vec![vec![
            "use".to_owned(),
            "component".to_owned(),
            "view".to_owned()
        ]]
    );
}

#[test]
fn a_wrong_or_missing_keyword_is_reported() {
    let list = |reserved: bool| -> String {
        Token::keywords()
            .filter(|word| keyword_token(word).is_some_and(|t| t.is_reserved() == reserved))
            .map(|word| format!("`{word}`"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let complete = format!("{}\n\n{}\n", list(true), list(false));
    assert_eq!(
        keyword_list_problems("x", &[complete]),
        Vec::<String>::new()
    );

    let markdown = "`use` `shield`\n\n`wire` `harness`\n".to_owned();
    let problems = keyword_list_problems("x", &[markdown]);
    assert!(problems.contains(&"x: a keyword list mixes reserved and contextual words".to_owned()));
    assert!(
        problems
            .contains(&"x: lists `harness` among the keywords, but it is no keyword".to_owned())
    );
    assert!(problems.contains(&"x: does not list the keyword `component`".to_owned()));
    assert!(!problems.iter().any(|p| p.contains("`wire`")));
}

#[test]
fn a_block_that_does_not_parse_is_reported() {
    let (problems, checked) = check_markdown(
        Path::new("x.md"),
        "```wb\ncomponent A { pub port a \"A\" }\n```\n",
    );
    assert_eq!(checked, 1);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("x.md:1: block does not parse"));
}

#[test]
fn a_fragment_is_checked_in_its_context() {
    let markdown = "```wb component\npub port a \"A\";\n```\n\n\
                    ```wb view\ninclude a at (0, 0);\n```\n";
    assert_eq!(check_markdown(Path::new("x.md"), markdown), (Vec::new(), 2));
    // A component body statement is no view item.
    let (problems, _) = check_markdown(Path::new("x.md"), "```wb view\npub port a \"A\";\n```\n");
    assert_eq!(problems.len(), 1);
}

#[test]
fn an_error_block_must_fail_to_parse() {
    let wrong = "```wb component error\npub port wire \"W\";\n```\n";
    assert_eq!(check_markdown(Path::new("x.md"), wrong), (Vec::new(), 1));
    let right = "```wb component error\npub port w \"W\";\n```\n";
    let (problems, _) = check_markdown(Path::new("x.md"), right);
    assert!(problems[0].contains("tagged `error` but parses"));
}

#[test]
fn an_untagged_block_or_an_unknown_tag_word_is_reported() {
    let (problems, checked) = check_markdown(
        Path::new("x.md"),
        "```\ncomponent A {}\n```\n\n```wb harness\nview harness \"H\" {}\n```\n",
    );
    assert_eq!(checked, 0);
    assert_eq!(problems.len(), 2);
    assert!(problems[0].contains("no info string"));
    assert!(problems[1].contains("unknown word `harness`"));
}

#[test]
fn a_block_that_is_not_wb_is_not_parsed() {
    let markdown = "```toml\n[project]\n```\n\n- item\n\n  ```text\nwire <color>;\n  ```\n";
    assert_eq!(check_markdown(Path::new("x.md"), markdown), (Vec::new(), 0));
}
