//! The trivia collector: the other side of the dial from
//! [`significant`](super::significant). Where the parser drops trivia, the
//! formatter needs the comments and the blank lines, each attached to the
//! token it belongs with, so it can lay the tokens out afresh and put every
//! comment back beside its statement.

use super::{Lexeme, SpannedLexeme, Token, Trivia};

/// Something on its own lines before a token: a comment, or the blank
/// line(s) the author left between two things (one or many, it is one gap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leading<'src> {
    /// A `//` comment on a line of its own, trailing whitespace trimmed.
    Comment(&'src str),
    /// At least one empty line.
    BlankLine,
}

/// A significant token, its source text, and the trivia that belongs to
/// it: what stands on its own lines before it, and a comment that ends its
/// line.
#[derive(Debug, Clone, PartialEq)]
pub struct Attached<'src> {
    pub token: Token,
    /// The token as written (a string with its quotes, a number with its
    /// unit).
    pub text: &'src str,
    /// Own-line comments and blank lines between the previous token and
    /// this one, in order.
    pub leading: Vec<Leading<'src>>,
    /// A comment after this token on the same line, trailing whitespace
    /// trimmed.
    pub trailing: Option<&'src str>,
}

/// A file's tokens with their trivia attached, and the trivia after the
/// last token.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachedTokens<'src> {
    pub tokens: Vec<Attached<'src>>,
    /// Comments and blank lines after the last token, before the end of
    /// the file.
    pub end: Vec<Leading<'src>>,
}

impl<'src> AttachedTokens<'src> {
    /// Attach the trivia of `lexemes`, lexed from `src`, to their tokens. A
    /// comment that shares its line with the token before it trails that
    /// token; any other comment, and a run of empty lines, leads the token
    /// after it. Blank lines before the first token or comment are dropped.
    pub fn collect(src: &'src str, lexemes: &[SpannedLexeme]) -> Self {
        let mut tokens: Vec<Attached<'src>> = Vec::new();
        let mut pending = Vec::new();
        // Newlines since the last token or comment, and whether there was one.
        let mut newlines = 0usize;
        let mut started = false;
        for lexeme in lexemes {
            let text = &src[lexeme.span.start..lexeme.span.end];
            match &lexeme.lexeme {
                Lexeme::Trivia(Trivia::Whitespace) => {}
                Lexeme::Trivia(Trivia::Newline) => newlines += 1,
                Lexeme::Trivia(Trivia::LineComment) => {
                    let comment = text.trim_end();
                    match tokens.last_mut() {
                        Some(last) if newlines == 0 && pending.is_empty() => {
                            last.trailing = Some(comment);
                        }
                        _ => {
                            if started && newlines >= 2 {
                                pending.push(Leading::BlankLine);
                            }
                            pending.push(Leading::Comment(comment));
                        }
                    }
                    newlines = 0;
                    started = true;
                }
                Lexeme::Token(token) => {
                    if started && newlines >= 2 {
                        pending.push(Leading::BlankLine);
                    }
                    tokens.push(Attached {
                        token: token.clone(),
                        text,
                        leading: std::mem::take(&mut pending),
                        trailing: None,
                    });
                    newlines = 0;
                    started = true;
                }
            }
        }
        if started && newlines >= 2 {
            pending.push(Leading::BlankLine);
        }
        AttachedTokens {
            tokens,
            end: pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::lex::lex;
    use crate::dsl::span::FileId;

    fn collect(src: &str) -> AttachedTokens<'_> {
        AttachedTokens::collect(src, &lex(src, FileId(0)).expect("lexes"))
    }

    #[test]
    fn a_comment_on_the_line_of_a_token_trails_it() {
        let attached = collect("a: B; // the b\nc: D;\n");
        assert_eq!(attached.tokens[3].text, ";");
        assert_eq!(attached.tokens[3].trailing, Some("// the b"));
        assert!(attached.tokens[4].leading.is_empty());
    }

    #[test]
    fn a_comment_on_its_own_line_leads_the_next_token() {
        let attached = collect("a: B;\n  // about c   \nc: D;\n");
        assert_eq!(attached.tokens[3].trailing, None);
        assert_eq!(
            attached.tokens[4].leading,
            vec![Leading::Comment("// about c")]
        );
    }

    #[test]
    fn blank_lines_become_one_gap_and_none_at_the_start() {
        let attached = collect("\n\n// head\n\n\n\na;\n\n// tail\n\n");
        assert_eq!(
            attached.tokens[0].leading,
            vec![Leading::Comment("// head"), Leading::BlankLine]
        );
        assert_eq!(
            attached.end,
            vec![
                Leading::BlankLine,
                Leading::Comment("// tail"),
                Leading::BlankLine
            ]
        );
    }

    #[test]
    fn tokens_keep_their_source_text() {
        let attached = collect("length: 150mm; label: \"A B\";");
        let texts: Vec<&str> = attached.tokens.iter().map(|t| t.text).collect();
        assert_eq!(
            texts,
            ["length", ":", "150mm", ";", "label", ":", "\"A B\"", ";"]
        );
    }
}
