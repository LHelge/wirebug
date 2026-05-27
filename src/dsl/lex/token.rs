//! Tokens and lexemes for the `.wb` lexer.
//!
//! A [`Lexeme`] is either a significant [`Token`] or discardable
//! [`Trivia`] (whitespace, newlines, comments). The lexer emits both,
//! spanned; [`significant`](super::significant) is the dial that drops
//! trivia today and that a future `fmt` will replace with a collector.

use std::fmt;

/// A significant token — everything the parser cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    // Keywords.
    Use,
    From,
    Component,
    Connector,
    Port,
    Pub,
    Pin,
    Pins,
    Wire,
    Cable,
    View,
    Include,
    At,
    Grid,
    Ports,
    Enclosure,

    // Punctuation.
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    Semicolon,
    Dot,
    Colon,

    // Leaves. Raw text is preserved so a future `fmt` can round-trip and
    // so the parser can interpret a [`Number`](Token::Number) as either a
    // pin (`u32`) or a gauge/coordinate (`f64`).
    Ident(String),
    /// Decoded contents of a `"quoted label"` (without the quotes).
    Str(String),
    /// Raw numeric literal text, e.g. `50`, `0.25`.
    Number(String),
}

impl Token {
    /// Map an identifier-shaped slice to its keyword token, if any.
    pub(super) fn keyword(text: &str) -> Option<Token> {
        Some(match text {
            "use" => Token::Use,
            "from" => Token::From,
            "component" => Token::Component,
            "connector" => Token::Connector,
            "port" => Token::Port,
            "pub" => Token::Pub,
            "pin" => Token::Pin,
            "pins" => Token::Pins,
            "wire" => Token::Wire,
            "cable" => Token::Cable,
            "view" => Token::View,
            "include" => Token::Include,
            "at" => Token::At,
            "grid" => Token::Grid,
            "ports" => Token::Ports,
            "enclosure" => Token::Enclosure,
            _ => return None,
        })
    }
}

/// Surface form, for diagnostics ("expected `port`", "unexpected `[`").
impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Token::Use => "use",
            Token::From => "from",
            Token::Component => "component",
            Token::Connector => "connector",
            Token::Port => "port",
            Token::Pub => "pub",
            Token::Pin => "pin",
            Token::Pins => "pins",
            Token::Wire => "wire",
            Token::Cable => "cable",
            Token::View => "view",
            Token::Include => "include",
            Token::At => "at",
            Token::Grid => "grid",
            Token::Ports => "ports",
            Token::Enclosure => "enclosure",
            Token::LBrace => "{",
            Token::RBrace => "}",
            Token::LBracket => "[",
            Token::RBracket => "]",
            Token::LParen => "(",
            Token::RParen => ")",
            Token::Comma => ",",
            Token::Semicolon => ";",
            Token::Dot => ".",
            Token::Colon => ":",
            Token::Ident(name) => name,
            Token::Str(s) => return write!(f, "{s:?}"),
            Token::Number(n) => n,
        };
        f.write_str(s)
    }
}

/// Insignificant source text, recognised but discarded today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trivia {
    /// A run of spaces, tabs, and carriage returns.
    Whitespace,
    /// A single `\n`.
    Newline,
    /// A `//` comment, up to (but not including) the newline.
    LineComment,
}

/// One lexed unit: a significant [`Token`] or discardable [`Trivia`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lexeme {
    Token(Token),
    Trivia(Trivia),
}
