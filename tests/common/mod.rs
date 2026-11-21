//! Helpers shared by integration tests (not a test target of its own).

use wirebug::dsl::format::Formatted;
use wirebug::dsl::lex::{lex, significant};
use wirebug::dsl::parse::parse_file;
use wirebug::dsl::span::FileId;

/// Whether `fmt` keeps `src` what it was: formatting it again changes
/// nothing, and the formatted text parses to the AST `src` does, spans
/// aside. `None` for a source the formatter refuses (one that does not
/// parse); otherwise the first property that fails, or `Ok`.
pub fn format_faithfully(src: &str) -> Option<Result<(), String>> {
    let once = Formatted::new(src).ok()?;
    let twice = match Formatted::new(once.as_str()) {
        Ok(twice) => twice,
        Err(error) => {
            return Some(Err(format!(
                "the formatted text does not format ({error}):\n{}",
                once.as_str()
            )));
        }
    };
    Some(if twice != once {
        Err(format!(
            "formatting again changes it:\n--- once\n{}--- twice\n{}",
            once.as_str(),
            twice.as_str()
        ))
    } else if ast(src) != ast(once.as_str()) {
        Err(format!(
            "the formatted text means something else:\n{}",
            once.as_str()
        ))
    } else {
        Ok(())
    })
}

/// The AST `src` parses to, as its `Debug` text without the spans.
fn ast(src: &str) -> String {
    let lexemes = lex(src, FileId(0)).expect("formatted text lexes");
    let file = parse_file(significant(&lexemes), FileId(0), src).file;
    without_spans(&format!("{file:?}"))
}

/// `debug` with every span left out: a `Spanned`'s ` @ <start>..<end>` and
/// a node's `Span { … }`.
fn without_spans(debug: &str) -> String {
    let mut out = String::new();
    let mut rest = debug;
    while let Some(ch) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix(" @ ") {
            let len = after
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(after.len());
            rest = &after[len..];
        } else if let Some(after) = rest.strip_prefix("Span { ") {
            let len = after.find('}').map_or(after.len(), |at| at + 1);
            out.push_str("Span");
            rest = &after[len..];
        } else {
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}
