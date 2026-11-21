//! Statements, read off a file's attached tokens by the one rule every
//! `.wb` block shares (the parser's recovery skips by it too): a statement
//! runs to its `;`, or through the `{ … }` block it opens.

use crate::dsl::lex::Token;
use crate::dsl::lex::trivia::Attached;

/// One statement of a file or of a block.
#[derive(Debug)]
pub(super) enum Statement<'a, 'src> {
    /// The tokens of a statement up to and including its `;`.
    Simple(&'a [Attached<'src>]),
    /// A statement that opens a block: its tokens up to and including the
    /// `{`, the block's statements, and its `}`; and all of its tokens, from
    /// the first to the `}`.
    Block {
        header: &'a [Attached<'src>],
        body: Body<'a, 'src>,
        tokens: &'a [Attached<'src>],
    },
}

impl<'a, 'src> Statement<'a, 'src> {
    /// The statement's first token, which carries the trivia before it.
    pub(super) fn first(&self) -> &'a Attached<'src> {
        match self {
            Statement::Simple(tokens) | Statement::Block { header: tokens, .. } => &tokens[0],
        }
    }

    /// The tokens of a statement that prints on one line, as one: a simple
    /// statement, or a block holding a single simple statement that the
    /// author wrote on one line with its `{` and `}` and no comment inside
    /// (`include sw at (4, 0) ports { west: a; }`). `None` for any other
    /// block.
    pub(super) fn one_line(&self) -> Option<&'a [Attached<'src>]> {
        match self {
            Statement::Simple(tokens) => Some(tokens),
            Statement::Block {
                header,
                body,
                tokens,
            } => {
                let inner = &tokens[header.len()..];
                let single = matches!(body.statements.as_slice(), [Statement::Simple(_)])
                    && body.close.is_some();
                let on_its_line = header.last().is_some_and(|open| open.trailing.is_none())
                    && inner.iter().all(|token| !token.newline_before);
                (single && on_its_line).then_some(*tokens)
            }
        }
    }
}

/// The statements of a block, and the `}` closing it (absent only for the
/// file's top level).
#[derive(Debug)]
pub(super) struct Body<'a, 'src> {
    pub(super) statements: Vec<Statement<'a, 'src>>,
    pub(super) close: Option<&'a Attached<'src>>,
}

impl<'a, 'src> Body<'a, 'src> {
    /// The statements of a whole file. Every token lands in a statement:
    /// a `}` that closes nothing (which a file that parses never holds)
    /// stands as a statement of its own.
    pub(super) fn file(tokens: &'a [Attached<'src>]) -> Self {
        let mut at = 0;
        let mut statements = Vec::new();
        loop {
            statements.extend(Self::statements(tokens, &mut at));
            match tokens.get(at) {
                Some(_) => {
                    statements.push(Statement::Simple(&tokens[at..=at]));
                    at += 1;
                }
                None => break,
            }
        }
        Body {
            statements,
            close: None,
        }
    }

    /// The statements from `at` up to the `}` that closes their block (left
    /// for the caller) or the end of the tokens.
    fn statements(tokens: &'a [Attached<'src>], at: &mut usize) -> Vec<Statement<'a, 'src>> {
        let mut statements = Vec::new();
        while let Some(first) = tokens.get(*at) {
            if first.token == Token::RBrace {
                break;
            }
            let start = *at;
            let end = tokens[start..]
                .iter()
                .position(|t| matches!(t.token, Token::Semicolon | Token::LBrace))
                .map_or(tokens.len(), |offset| start + offset + 1);
            *at = end;
            if tokens[end - 1].token == Token::LBrace {
                let inner = Self::statements(tokens, at);
                let close = tokens.get(*at).inspect(|_| *at += 1);
                statements.push(Statement::Block {
                    header: &tokens[start..end],
                    body: Body {
                        statements: inner,
                        close,
                    },
                    tokens: &tokens[start..*at],
                });
            } else {
                statements.push(Statement::Simple(&tokens[start..end]));
            }
        }
        statements
    }
}
