//! Hand-written lexer for the `.wb` DSL.
//!
//! The lexer recognises trivia (whitespace, newlines, `//` comments) as
//! first-class lexemes and always emits them with spans. [`significant`]
//! is the dial: today it drops trivia, leaving the parser a clean
//! `(Token, Span)` stream; a future `fmt` swaps it for a collector that
//! attaches trivia to neighbouring tokens — without touching anything
//! downstream.

pub mod token;

use crate::dsl::span::{FileId, Span};
pub use token::{Lexeme, Token, Trivia};

/// A [`Lexeme`] with its source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedLexeme {
    pub lexeme: Lexeme,
    pub span: Span,
}

/// A lexical error. Becomes a miette diagnostic once the diagnostics
/// layer lands; for now it carries the span and enough context to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexError {
    /// A `"` string ran to end-of-file without a closing quote.
    UnterminatedString { span: Span },
    /// A `"` string contained a newline (labels are single-line).
    NewlineInString { span: Span },
    /// A character that can't begin any token.
    UnexpectedChar { ch: char, span: Span },
}

impl LexError {
    pub fn span(&self) -> Span {
        match self {
            LexError::UnterminatedString { span }
            | LexError::NewlineInString { span }
            | LexError::UnexpectedChar { span, .. } => *span,
        }
    }
}

/// Tokenise `src` (the contents of file `file`) into spanned lexemes,
/// including trivia. Fails on the first lexical error.
pub fn lex(src: &str, file: FileId) -> Result<Vec<SpannedLexeme>, LexError> {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0;

    let push = |out: &mut Vec<SpannedLexeme>, lexeme: Lexeme, start: usize, end: usize| {
        out.push(SpannedLexeme {
            lexeme,
            span: Span::new(file, start, end),
        });
    };

    while i < n {
        let start = i;
        match bytes[i] {
            b' ' | b'\t' | b'\r' => {
                i += 1;
                while i < n && matches!(bytes[i], b' ' | b'\t' | b'\r') {
                    i += 1;
                }
                push(&mut out, Lexeme::Trivia(Trivia::Whitespace), start, i);
            }
            b'\n' => {
                i += 1;
                push(&mut out, Lexeme::Trivia(Trivia::Newline), start, i);
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                i += 2;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                push(&mut out, Lexeme::Trivia(Trivia::LineComment), start, i);
            }
            b'"' => {
                i += 1;
                let content_start = i;
                loop {
                    if i >= n {
                        return Err(LexError::UnterminatedString {
                            span: Span::new(file, start, i),
                        });
                    }
                    match bytes[i] {
                        b'"' => break,
                        b'\n' => {
                            return Err(LexError::NewlineInString {
                                span: Span::new(file, start, i),
                            });
                        }
                        _ => i += 1,
                    }
                }
                let content = src[content_start..i].to_string();
                i += 1; // closing quote
                push(&mut out, Lexeme::Token(Token::Str(content)), start, i);
            }
            b'0'..=b'9' => {
                i += 1;
                while i < n && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                // A single fractional part: `.` only when a digit follows,
                // so `inst.port` keeps its `.` as a separate Dot token.
                if i + 1 < n && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < n && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text = src[start..i].to_string();
                push(&mut out, Lexeme::Token(Token::Number(text)), start, i);
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                i += 1;
                while i < n && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                let text = &src[start..i];
                let token = Token::keyword(text).unwrap_or_else(|| Token::Ident(text.to_string()));
                push(&mut out, Lexeme::Token(token), start, i);
            }
            _ => {
                let token = match bytes[i] {
                    b'{' => Some(Token::LBrace),
                    b'}' => Some(Token::RBrace),
                    b'[' => Some(Token::LBracket),
                    b']' => Some(Token::RBracket),
                    b'(' => Some(Token::LParen),
                    b')' => Some(Token::RParen),
                    b',' => Some(Token::Comma),
                    b';' => Some(Token::Semicolon),
                    b'.' => Some(Token::Dot),
                    _ => None,
                };
                match token {
                    Some(token) => {
                        i += 1;
                        push(&mut out, Lexeme::Token(token), start, i);
                    }
                    None => {
                        let ch = src[start..].chars().next().expect("byte implies a char");
                        return Err(LexError::UnexpectedChar {
                            ch,
                            span: Span::new(file, start, start + ch.len_utf8()),
                        });
                    }
                }
            }
        }
    }

    Ok(out)
}

/// THE DIAL. Drop trivia, yielding the significant `(Token, Span)` stream
/// the parser consumes. A future `fmt` replaces this with a pass that
/// attaches trivia to neighbouring tokens instead of discarding it.
pub fn significant(lexemes: &[SpannedLexeme]) -> Vec<(Token, Span)> {
    lexemes
        .iter()
        .filter_map(|l| match &l.lexeme {
            Lexeme::Token(t) => Some((t.clone(), l.span)),
            Lexeme::Trivia(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: FileId = FileId(0);

    fn tokens(src: &str) -> Vec<Token> {
        let lexemes = lex(src, F).expect("lexes");
        significant(&lexemes).into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn lexes_keywords_punctuation_and_leaves() {
        let toks = tokens(r#"pub port hv_pos "HV+" pin 1;"#);
        assert_eq!(
            toks,
            vec![
                Token::Pub,
                Token::Port,
                Token::Ident("hv_pos".into()),
                Token::Str("HV+".into()),
                Token::Pin,
                Token::Number("1".into()),
                Token::Semicolon,
            ]
        );
    }

    #[test]
    fn endpoint_dot_is_separate_from_fractional_number() {
        // `pack.hv_pos` keeps its dot; `0.25` stays one number.
        assert_eq!(
            tokens("pack.hv_pos 0.25"),
            vec![
                Token::Ident("pack".into()),
                Token::Dot,
                Token::Ident("hv_pos".into()),
                Token::Number("0.25".into()),
            ]
        );
    }

    #[test]
    fn schematic_is_an_identifier_not_a_keyword() {
        assert_eq!(
            tokens("view schematic"),
            vec![Token::View, Token::Ident("schematic".into())]
        );
    }

    #[test]
    fn pins_list_lexes() {
        assert_eq!(
            tokens("pins (2, 3, 4)"),
            vec![
                Token::Pins,
                Token::LParen,
                Token::Number("2".into()),
                Token::Comma,
                Token::Number("3".into()),
                Token::Comma,
                Token::Number("4".into()),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn comments_and_whitespace_become_trivia_and_are_dropped() {
        let src = "// a comment\nport x \"X\"; // trailing\n";
        let lexemes = lex(src, F).expect("lexes");
        // Trivia is present in the raw lexeme stream...
        assert!(
            lexemes
                .iter()
                .any(|l| matches!(l.lexeme, Lexeme::Trivia(Trivia::LineComment)))
        );
        assert!(
            lexemes
                .iter()
                .any(|l| matches!(l.lexeme, Lexeme::Trivia(Trivia::Newline)))
        );
        // ...but significant() drops every trivium.
        let sig = significant(&lexemes);
        assert!(
            sig.iter()
                .all(|(t, _)| !matches!(t, Token::Ident(s) if s == "comment"))
        );
        assert_eq!(
            sig.into_iter().map(|(t, _)| t).collect::<Vec<_>>(),
            vec![
                Token::Port,
                Token::Ident("x".into()),
                Token::Str("X".into()),
                Token::Semicolon,
            ]
        );
    }

    #[test]
    fn spans_point_at_the_source_text() {
        let src = "port hv_pos";
        let lexemes = lex(src, F).expect("lexes");
        let sig = significant(&lexemes);
        let (_, hv_span) = &sig[1];
        assert_eq!(&src[hv_span.start..hv_span.end], "hv_pos");
    }

    #[test]
    fn newline_in_string_errors() {
        let err = lex("port x \"oops\n\"", F).expect_err("newline in string");
        assert!(matches!(err, LexError::NewlineInString { .. }));
    }

    #[test]
    fn unterminated_string_errors() {
        let err = lex("\"no end", F).expect_err("unterminated");
        assert!(matches!(err, LexError::UnterminatedString { .. }));
    }

    #[test]
    fn unexpected_char_errors() {
        let err = lex("port @", F).expect_err("at-sign is not a token");
        assert!(matches!(err, LexError::UnexpectedChar { ch: '@', .. }));
    }

    #[test]
    fn lexes_every_example_file_without_error() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
        let mut count = 0;
        for entry in walk_wb(std::path::Path::new(root)) {
            let src = std::fs::read_to_string(&entry).expect("read example");
            lex(&src, F).unwrap_or_else(|e| panic!("lex {}: {e:?}", entry.display()));
            count += 1;
        }
        assert!(count >= 13, "expected the full seed corpus, found {count}");
    }

    fn walk_wb(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                out.extend(walk_wb(&path));
            } else if path.extension().is_some_and(|e| e == "wb") {
                out.push(path);
            }
        }
        out
    }
}
