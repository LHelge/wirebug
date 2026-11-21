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
    Extend,
    ConnectorType,
    Connector,
    Inline,
    Port,
    Pub,
    Pin,
    Pins,
    Wire,
    Cable,
    Twisted,
    View,
    For,
    Include,
    Text,
    At,
    Grid,
    Face,
    Cavity,
    Layout,
    Rows,
    Cols,
    Numbering,
    Size,
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
    /// A single `/` — the base/tracer separator in a two-tone wire color
    /// (`green/yellow`). `//` is still a comment.
    Slash,

    // Leaves. Raw text is preserved so a future `fmt` can round-trip and
    // so the parser can interpret a [`Number`](Token::Number) as either a
    // pin (`u32`) or a gauge/coordinate (`f64`).
    Ident(String),
    /// Decoded contents of a `"quoted label"` (without the quotes).
    Str(String),
    /// A numeric literal: its raw text (`50`, `-0.25`), and the unit
    /// written directly after it (`mm` in `150mm`). Any name-shaped run
    /// touching the digits is the unit, whatever it spells: the parser
    /// decides where a unit may go, and validation which units exist.
    Number {
        digits: String,
        unit: Option<String>,
    },
}

/// Every keyword with its surface text: the lexer's lookup table and the
/// keywords' [`Display`](fmt::Display) form, so the two cannot drift.
pub(super) static KEYWORDS: [(&str, Token); 29] = [
    ("use", Token::Use),
    ("from", Token::From),
    ("component", Token::Component),
    ("extend", Token::Extend),
    ("connector_type", Token::ConnectorType),
    ("connector", Token::Connector),
    ("inline", Token::Inline),
    ("port", Token::Port),
    ("pub", Token::Pub),
    ("pin", Token::Pin),
    ("pins", Token::Pins),
    ("wire", Token::Wire),
    ("cable", Token::Cable),
    ("twisted", Token::Twisted),
    ("view", Token::View),
    ("for", Token::For),
    ("include", Token::Include),
    ("text", Token::Text),
    ("at", Token::At),
    ("grid", Token::Grid),
    ("face", Token::Face),
    ("cavity", Token::Cavity),
    ("layout", Token::Layout),
    ("rows", Token::Rows),
    ("cols", Token::Cols),
    ("numbering", Token::Numbering),
    ("size", Token::Size),
    ("ports", Token::Ports),
    ("enclosure", Token::Enclosure),
];

impl Token {
    /// Map an identifier-shaped slice to its keyword token, if any.
    pub(super) fn keyword(text: &str) -> Option<Token> {
        KEYWORDS
            .iter()
            .find(|(word, _)| *word == text)
            .map(|(_, token)| token.clone())
    }

    /// The surface text of a keyword token; `None` for punctuation and
    /// leaves.
    fn keyword_text(&self) -> Option<&'static str> {
        KEYWORDS
            .iter()
            .find(|(_, token)| token == self)
            .map(|(word, _)| *word)
    }

    /// A contextual keyword: one that means something only inside a view,
    /// a `connector_type` layout, a cavity line, after `use <Name>`, or
    /// after a view's title (`for <Component>`).
    /// Where the grammar expects a name, it is an ordinary name
    /// (`pub port size "SIZE";`), so the word stays free for users.
    ///
    /// Every other keyword is reserved: the words that declare the design
    /// itself (components, ports and pins, connectors, wires, cables,
    /// views, imports). A new keyword is reserved until it is listed here.
    pub(crate) fn is_contextual(&self) -> bool {
        matches!(
            self,
            Token::From
                | Token::For
                | Token::Include
                | Token::Text
                | Token::At
                | Token::Grid
                | Token::Face
                | Token::Cavity
                | Token::Layout
                | Token::Rows
                | Token::Cols
                | Token::Numbering
                | Token::Size
                | Token::Ports
                | Token::Enclosure
        )
    }

    /// A reserved keyword: never a name. See [`is_contextual`](Self::is_contextual).
    pub(crate) fn is_reserved(&self) -> bool {
        self.keyword_text().is_some() && !self.is_contextual()
    }

    /// The name this token spells where the grammar expects one: an
    /// identifier's text, or a contextual keyword's. `None` for a reserved
    /// keyword, punctuation, strings and numbers.
    pub(crate) fn as_name(&self) -> Option<&str> {
        match self {
            Token::Ident(name) => Some(name.as_str()),
            token if token.is_contextual() => token.keyword_text(),
            _ => None,
        }
    }
}

/// Surface form, for diagnostics ("expected `port`", "unexpected `[`").
impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(word) = self.keyword_text() {
            return f.write_str(word);
        }
        let s = match self {
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
            Token::Slash => "/",
            Token::Ident(name) => name,
            Token::Str(s) => return write!(f, "{s:?}"),
            Token::Number { digits, unit } => {
                return write!(f, "{digits}{}", unit.as_deref().unwrap_or_default());
            }
            // Only a keyword left out of `KEYWORDS` lands here, and that
            // keyword would not lex either.
            unlisted => return write!(f, "{unlisted:?}"),
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
