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
            // A `-` is only a sign: it lexes when a digit follows (negative
            // grid coordinates), there is no standalone minus token.
            c @ (b'0'..=b'9' | b'-')
                if c != b'-' || (i + 1 < n && bytes[i + 1].is_ascii_digit()) =>
            {
                i += 1; // sign or first digit
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
                let digits = src[start..i].to_string();
                // A unit touches its number (`150mm`); after a space a name
                // is a token of its own.
                let unit_start = i;
                if i < n && (bytes[i] == b'_' || bytes[i].is_ascii_alphabetic()) {
                    i = name_end(bytes, i);
                }
                let unit = (i > unit_start).then(|| src[unit_start..i].to_string());
                push(
                    &mut out,
                    Lexeme::Token(Token::Number { digits, unit }),
                    start,
                    i,
                );
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                i = name_end(bytes, i);
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
                    b':' => Some(Token::Colon),
                    // A lone `/`; `//` was already taken as a comment above.
                    b'/' => Some(Token::Slash),
                    _ => None,
                };
                match token {
                    Some(token) => {
                        i += 1;
                        push(&mut out, Lexeme::Token(token), start, i);
                    }
                    None => {
                        let Some(ch) = src[start..].chars().next() else {
                            return Ok(out);
                        };
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

/// The end of the name-shaped run (`[A-Za-z0-9_]*`) whose first byte, a
/// letter or `_`, is at `start`: an identifier, or a number's unit.
fn name_end(bytes: &[u8], start: usize) -> usize {
    bytes[start + 1..]
        .iter()
        .position(|&b| b != b'_' && !b.is_ascii_alphanumeric())
        .map_or(bytes.len(), |len| start + 1 + len)
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

    /// A number without a unit.
    fn num(digits: &str) -> Token {
        Token::Number {
            digits: digits.into(),
            unit: None,
        }
    }

    /// A number with a unit written directly after it.
    fn with_unit(digits: &str, unit: &str) -> Token {
        Token::Number {
            digits: digits.into(),
            unit: Some(unit.into()),
        }
    }

    #[test]
    fn a_unit_touching_its_number_lexes_into_the_number() {
        assert_eq!(
            tokens("2m 150mm -0.5cm 1.5e3 2in"),
            vec![
                with_unit("2", "m"),
                with_unit("150", "mm"),
                with_unit("-0.5", "cm"),
                with_unit("1.5", "e3"),
                with_unit("2", "in"),
            ]
        );
        // The token's span covers the number and its unit.
        let src = "length: 150mm;";
        let sig = significant(&lex(src, F).expect("lexes"));
        let (_, span) = &sig[2];
        assert_eq!(&src[span.start..span.end], "150mm");
    }

    #[test]
    fn a_name_after_a_space_stays_its_own_token() {
        assert_eq!(
            tokens("2 m cavity 1 at"),
            vec![
                num("2"),
                Token::Ident("m".into()),
                Token::Cavity,
                num("1"),
                Token::At,
            ]
        );
    }

    #[test]
    fn a_name_with_digits_is_still_an_identifier() {
        // Only a run that starts with a digit is a number: `A1` and `x1`
        // stay names, and `2.m` keeps its dot.
        assert_eq!(
            tokens("A1 x1 2.m"),
            vec![
                Token::Ident("A1".into()),
                Token::Ident("x1".into()),
                num("2"),
                Token::Dot,
                Token::Ident("m".into()),
            ]
        );
    }

    #[test]
    fn a_number_displays_with_its_unit() {
        assert_eq!(with_unit("150", "mm").to_string(), "150mm");
        assert_eq!(num("-0.5").to_string(), "-0.5");
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
                num("1"),
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
                num("0.25"),
            ]
        );
    }

    #[test]
    fn lone_slash_is_a_token_double_slash_a_comment() {
        // `green/yellow` is three tokens; `// yellow` is trivia.
        assert_eq!(
            tokens("green/yellow // yellow"),
            vec![
                Token::Ident("green".into()),
                Token::Slash,
                Token::Ident("yellow".into()),
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
    fn every_keyword_displays_as_the_text_it_lexes_from() {
        for (word, token) in &token::KEYWORDS {
            assert_eq!(&Token::keyword(word).as_ref(), &Some(token));
            assert_eq!(token.to_string(), *word);
        }
    }

    #[test]
    fn view_and_layout_words_are_contextual_names() {
        for word in [
            "from",
            "fragment",
            "shield",
            "splice",
            "for",
            "include",
            "text",
            "at",
            "grid",
            "face",
            "cavity",
            "layout",
            "rows",
            "cols",
            "numbering",
            "size",
            "ports",
            "enclosure",
        ] {
            let token = Token::keyword(word).expect("a keyword");
            assert!(token.is_contextual() && !token.is_reserved(), "{word}");
            assert_eq!(token.as_name(), Some(word));
        }
    }

    #[test]
    fn declaring_words_are_reserved_and_never_names() {
        for word in [
            "use",
            "component",
            "extend",
            "connector_type",
            "connector",
            "inline",
            "port",
            "pub",
            "pin",
            "pins",
            "wire",
            "cable",
            "twisted",
            "view",
        ] {
            let token = Token::keyword(word).expect("a keyword");
            assert!(token.is_reserved() && !token.is_contextual(), "{word}");
            assert_eq!(token.as_name(), None, "{word}");
        }
        // Only keywords are reserved; an identifier is its own name.
        let ident = Token::Ident("lamp".into());
        assert!(!ident.is_reserved());
        assert_eq!(ident.as_name(), Some("lamp"));
        assert!(!Token::Semicolon.is_reserved());
    }

    #[test]
    fn keywords_lists_every_word_that_lexes_as_a_keyword() {
        let words: Vec<&str> = Token::keywords().collect();
        assert_eq!(words.len(), 32);
        assert!(words.contains(&"connector_type") && words.contains(&"enclosure"));
        for word in words {
            assert!(Token::keyword(word).is_some(), "{word}");
        }
    }

    #[test]
    fn pins_list_lexes() {
        assert_eq!(
            tokens("pins [2, 3, 4]"),
            vec![
                Token::Pins,
                Token::LBracket,
                num("2"),
                Token::Comma,
                num("3"),
                Token::Comma,
                num("4"),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn negative_numbers_lex_as_one_token() {
        assert_eq!(
            tokens("at (-2, -0.5)"),
            vec![
                Token::At,
                Token::LParen,
                num("-2"),
                Token::Comma,
                num("-0.5"),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn minus_without_a_digit_is_an_error() {
        // `-` exists only as a numeric sign; `=` left the grammar with the
        // `pin N: port` binding form.
        for src in ["a - b", "pin 1 = a;"] {
            assert!(
                matches!(lex(src, F), Err(LexError::UnexpectedChar { .. })),
                "{src:?} should not lex"
            );
        }
    }

    #[test]
    fn connector_type_and_instance_punctuation_lex() {
        assert_eq!(
            tokens(
                "connector_type ampseal \"A\" { part: \"TE\"; } connector x1: ampseal { port a \"A\" pin 1; }"
            ),
            vec![
                Token::ConnectorType,
                Token::Ident("ampseal".into()),
                Token::Str("A".into()),
                Token::LBrace,
                Token::Ident("part".into()),
                Token::Colon,
                Token::Str("TE".into()),
                Token::Semicolon,
                Token::RBrace,
                Token::Connector,
                Token::Ident("x1".into()),
                Token::Colon,
                Token::Ident("ampseal".into()),
                Token::LBrace,
                Token::Port,
                Token::Ident("a".into()),
                Token::Str("A".into()),
                Token::Pin,
                num("1"),
                Token::Semicolon,
                Token::RBrace,
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
    fn lexes_every_fixture_file_without_error() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/basic_project");
        let mut count = 0;
        for entry in walk_wb(std::path::Path::new(root)) {
            let src = std::fs::read_to_string(&entry).expect("read fixture");
            lex(&src, F).unwrap_or_else(|e| panic!("lex {}: {e:?}", entry.display()));
            count += 1;
        }
        assert_eq!(count, 4, "expected the full fixture corpus");
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
