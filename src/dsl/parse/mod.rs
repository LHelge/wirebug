//! chumsky parser: a significant `(Token, Span)` stream → an [`ast::File`].
//!
//! Built bottom-up from small combinators. The only recursive production
//! is `definition` (components nest), handled with [`recursive`]. Errors
//! are chumsky `Rich` values carrying a span and an expected-set; we copy
//! them into owned [`ParseError`]s so no parser lifetime escapes.
//!
//! Names go through two productions, `ident` (a name after a keyword) and
//! `leading_name` (a name that starts a statement): an identifier, or a
//! contextual keyword ([`Token::is_contextual`]) read as a name. A reserved
//! keyword in name position is read as the name too, so the parse carries
//! on, and recorded in the parser state: every occurrence gets its own
//! reserved-word error.
//!
//! A statement that does not parse is reported and skipped, and the parse
//! reads on, so one run reports every broken statement (see `block`). The
//! skipped statement leaves a hole in the AST; [`Completeness`] says
//! whether a file has holes, or braces that do not balance.

use chumsky::error::RichPattern;
use chumsky::input::{Input, MapExtra, Stream, ValueInput};
use chumsky::inspector::TruncateState;
use chumsky::label::LabelError;
use chumsky::prelude::{
    IterParser, Parser, Rich, any, choice, end, extra, just, recursive, select,
};
use chumsky::primitive::custom;
use chumsky::recovery::via_parser;
use chumsky::util::MaybeRef;

use crate::dsl::ast::*;
use crate::dsl::lex::Token;
use crate::dsl::span::{FileId, Span, Spanned};

/// Use our file-tagged [`Span`] as chumsky's span type, so `e.span()`
/// yields spans that already know their file.
impl chumsky::span::Span for Span {
    type Context = FileId;
    type Offset = usize;

    fn new(context: FileId, range: std::ops::Range<usize>) -> Self {
        Span {
            file: context,
            start: range.start,
            end: range.end,
        }
    }

    fn context(&self) -> FileId {
        self.file
    }

    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.end
    }
}

/// What the parse notes beside the AST, in the parser state.
#[derive(Debug, Clone)]
enum Note {
    /// A reserved keyword read as a name.
    Reserved(Spanned<Token>),
    /// A statement that did not parse, skipped: the span of its tokens,
    /// and whether it starts with a reserved keyword.
    Skipped { span: Span, keyword_led: bool },
    /// A block that the end of input closed.
    Unclosed,
}

/// The notes of a parse. A `TruncateState` forgets the notes taken on a
/// branch the parser backtracks out of, and the same rewind drops the
/// errors chumsky emitted there, so the notes that remain are those of the
/// parse that stood, and each recovery's note pairs, in order, with the
/// syntax error it emitted.
type Notes = TruncateState<Note>;

/// The parser's extra: `Rich` errors, plus the notes.
type Extra<'tok> = extra::Full<Rich<'tok, Token, Span>, Notes, ()>;

/// An owned parse error, independent of any parser lifetime. Becomes a
/// miette diagnostic in the diagnostics layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// A syntax error: the span and chumsky's rendered reason.
    Syntax { span: Span, message: String },
    /// A reserved keyword written where a name belongs
    /// (`pub port wire "W";`). The parse took it as the name and went on.
    ReservedWord { span: Span, word: String },
}

impl ParseError {
    /// Where in the source the error points.
    pub fn span(&self) -> Span {
        match self {
            ParseError::Syntax { span, .. } | ParseError::ReservedWord { span, .. } => *span,
        }
    }
}

/// Result of parsing one file: the AST (absent only on unrecoverable
/// failure), any errors encountered, in source order, and how much of the
/// source the AST stands for.
#[derive(Debug)]
pub struct Parsed {
    pub file: Option<File>,
    pub errors: Vec<ParseError>,
    pub completeness: Completeness,
}

impl Parsed {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// How much of its source a file's AST stands for, once the parser has
/// skipped the statements it could not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// Every statement parsed (a reserved word read as a name included).
    Whole,
    /// Each statement reported as a syntax error was skipped: the AST lacks
    /// what it declared, and the rest stands as written.
    Holes,
    /// The braces do not balance, so from the unmatched one on the AST
    /// puts statements in other blocks than the author meant.
    Misnested,
}

impl Completeness {
    fn of(balanced: bool, errors: &[ParseError]) -> Self {
        if !balanced {
            Completeness::Misnested
        } else if errors
            .iter()
            .any(|e| matches!(e, ParseError::Syntax { .. }))
        {
            Completeness::Holes
        } else {
            Completeness::Whole
        }
    }
}

/// One member of a view body, parsed in any order and folded into a [`View`].
enum ViewItem {
    Grid(Spanned<f64>),
    Enclosure(Spanned<Vec<EnclosurePort>>),
    Include(Include),
    Text(TextBox),
}

/// One entry of a cable body, parsed in any order and folded into a
/// [`Cable`]: a property (`type`/`length`), or a conductor entry (a plain
/// wire or a `twisted { }` group).
enum CableItem {
    Property(CableProperty),
    Member(CableMember),
}

/// One top-level file entry, parsed in source order and folded into
/// [`File::uses`] plus [`File::items`].
enum FileEntry {
    Use(Use),
    Item(Item),
}

/// One member of a `connector_type` body.
enum ConnectorTypeItem {
    Property(ConnectorProperty),
    Layout(ConnectorLayout),
}

/// One entry of an `inline` body, parsed in any order and folded into an
/// [`Inline`]: a housing-half line (`male:`/`female:`) or a port.
enum InlineItem {
    Half(InlineHalfDecl),
    Port(Port),
}

/// Parse the significant token stream of one file into an [`ast::File`].
pub fn parse_file(tokens: Vec<(Token, Span)>, file: FileId, src_len: usize) -> Parsed {
    let eoi = Span {
        file,
        start: src_len,
        end: src_len,
    };
    let balanced = braces_balance(&tokens);
    let token_starts: Vec<usize> = tokens.iter().map(|(_, span)| span.start).collect();
    let stream = Stream::from_iter(tokens).map(eoi, |(t, s)| (t, s));
    let mut notes = Notes::from(Vec::new());
    let (file, syntax) = parser()
        .parse_with_state(stream, &mut notes)
        .into_output_errors();

    let mut reserved = Vec::new();
    let mut recoveries = Vec::new();
    for note in notes.0 {
        match note {
            Note::Reserved(word) => reserved.push(word),
            recovery => recoveries.push(recovery),
        }
    }
    let mut syntax = without_tails(syntax, &recoveries, &token_starts);
    if !balanced {
        // An unmatched brace puts every statement after it in another
        // block, and each would report an error of its own: keep the first
        // error alone, as the parse did before it could recover.
        syntax.sort_by_key(|e| e.span().start);
        syntax.truncate(1);
        let first = syntax.first().map_or(usize::MAX, |e| e.span().start);
        reserved.retain(|word| word.span.start < first);
    }
    let mut errors: Vec<ParseError> = reserved
        .into_iter()
        .map(|word| ParseError::ReservedWord {
            span: word.span,
            word: word.node.to_string(),
        })
        .chain(syntax)
        .collect();
    errors.sort_by_key(|e| e.span().start);
    let completeness = Completeness::of(balanced, &errors);
    Parsed {
        file,
        errors,
        completeness,
    }
}

/// The syntax errors to report: each recovered statement's, except where a
/// statement is the rest of the one before it. A statement that fails right
/// after one that failed on its own `;` (a `;` too early: `pub port ; a
/// "A";`) is that statement's tail, unless it starts with a reserved
/// keyword and so is a statement of its own; its error would only repeat
/// the first.
///
/// `recoveries` are the recovery notes, one per emitted error and in the
/// same order; `token_starts` the start of every token, in order.
fn without_tails(
    syntax: Vec<Rich<'_, Token, Span>>,
    recoveries: &[Note],
    token_starts: &[usize],
) -> Vec<ParseError> {
    let mut reported = Vec::new();
    // The span of the statement skipped last, and whether it failed on the
    // `;` that ends it.
    let mut previous: Option<(Span, bool)> = None;
    for (error, recovery) in syntax
        .into_iter()
        .zip(recoveries.iter().map(Some).chain(std::iter::repeat(None)))
    {
        let span = *error.span();
        let tail = match recovery {
            Some(Note::Skipped {
                span: skipped,
                keyword_led,
            }) => {
                let tail = !keyword_led
                    && previous.is_some_and(|(before, on_semicolon)| {
                        on_semicolon
                            && next_token_starts_at(token_starts, before.end, skipped.start)
                    });
                let on_semicolon =
                    error.found() == Some(&Token::Semicolon) && span.end == skipped.end;
                previous = Some((*skipped, on_semicolon));
                tail
            }
            _ => {
                previous = None;
                false
            }
        };
        if !tail {
            reported.push(ParseError::Syntax {
                span,
                message: error.reason().to_string(),
            });
        }
    }
    reported
}

/// Whether the first token at or after byte `after` starts at byte `at`:
/// nothing parsed between a statement that ends at `after` and one that
/// starts at `at`.
fn next_token_starts_at(token_starts: &[usize], after: usize, at: usize) -> bool {
    let next = token_starts.partition_point(|&start| start < after);
    token_starts.get(next) == Some(&at)
}

/// Whether every `}` closes a `{` and every `{` is closed. Recovery only
/// ever skips whole `{ … }` groups, so when this holds each block of the
/// AST holds what the author put in it.
fn braces_balance(tokens: &[(Token, Span)]) -> bool {
    tokens
        .iter()
        .try_fold(0usize, |depth, (token, _)| match token {
            Token::LBrace => Some(depth + 1),
            Token::RBrace => depth.checked_sub(1),
            _ => Some(depth),
        })
        == Some(0)
}

/// A parser that never matches, and that says a block may end here: where
/// `close` (the block's `}`, or the file's end) is next it fails without a
/// word, and anywhere else `close` fails first and adds itself to the
/// expected set. Tried after a block's statements, it keeps the error of a
/// statement that does not parse what it was before recovery: "expected
/// <the statements>, or '}'".
fn never_but_expect<'tok, I, C, O>(
    close: impl Parser<'tok, I, C, Extra<'tok>> + Clone,
) -> impl Parser<'tok, I, O, Extra<'tok>> + Clone
where
    I: ValueInput<'tok, Token = Token, Span = Span>,
{
    close.try_map(|_, span: Span| {
        Err(LabelError::<'tok, I, RichPattern<'tok, Token>>::expected_found([], None, span))
    })
}

/// Skip a statement that does not parse: up to and including the first `;`
/// outside braces, or through the `{ … }` block it opens, stopping before a
/// `}` that closes the enclosing block. Yields the statement's first token.
/// Fails, consuming nothing, at a `}` or the end of input, so a block's
/// close is never skipped as a statement. The skipped tokens are not read
/// as names, so they note no reserved words.
fn skip_statement<'tok, I>() -> impl Parser<'tok, I, Token, Extra<'tok>> + Clone
where
    I: ValueInput<'tok, Token = Token, Span = Span>,
{
    custom(|inp| {
        let start = inp.cursor();
        let mut first = None;
        let mut depth = 0usize;
        loop {
            let token = match (inp.peek(), depth) {
                (None, _) | (Some(Token::RBrace), 0) => break,
                (Some(token), _) => token,
            };
            inp.skip();
            let ends = match token {
                Token::Semicolon => depth == 0,
                Token::LBrace => {
                    depth += 1;
                    false
                }
                Token::RBrace => {
                    depth -= 1;
                    depth == 0
                }
                _ => false,
            };
            first.get_or_insert(token);
            if ends {
                break;
            }
        }
        match first {
            Some(token) => Ok(token),
            None => {
                let found = inp.peek_maybe();
                let span = inp.span_since(&start);
                Err(
                    LabelError::<'tok, I, RichPattern<'tok, Token>>::expected_found(
                        [],
                        found,
                        span,
                    ),
                )
            }
        }
    })
}

/// `skip`, as the recovery of a statement that does not parse: it notes the
/// skipped statement, and stands for no item.
fn skipped<'tok, I, O>(
    skip: impl Parser<'tok, I, Token, Extra<'tok>> + Clone,
) -> impl Parser<'tok, I, Option<O>, Extra<'tok>> + Clone
where
    I: ValueInput<'tok, Token = Token, Span = Span>,
{
    skip.map_with(|first, e| {
        let span = e.span();
        e.state().push(Note::Skipped {
            span,
            keyword_led: first.is_reserved(),
        });
        None
    })
}

/// A `{ item* }` block that reports every statement it cannot parse.
///
/// A statement that does not parse is reported, with the error the block
/// gave before recovery (the furthest point any statement got to, and what
/// would have been accepted there, the block's `}` included), and skipped
/// by [`skip_statement`]; the block reads on. The parser rewinds out of the
/// failed attempt before it skips, and the rewind forgets the reserved
/// words that attempt noted, so a skipped statement reports its syntax
/// error alone. A block still open at the end of input is reported there
/// and closed, so the errors already reported inside it stand.
fn block<'tok, I, O>(
    item: impl Parser<'tok, I, O, Extra<'tok>> + 'tok,
) -> impl Parser<'tok, I, Vec<O>, Extra<'tok>> + Clone
where
    I: ValueInput<'tok, Token = Token, Span = Span> + 'tok,
    O: 'tok,
{
    let unclosed = end().map_with(|(), e: &mut MapExtra<'tok, '_, I, Extra<'tok>>| {
        e.state().push(Note::Unclosed);
    });
    choice((item.map(Some), never_but_expect(just(Token::RBrace))))
        .recover_with(via_parser(skipped(skip_statement())))
        .repeated()
        .collect::<Vec<Option<O>>>()
        .map(|items| items.into_iter().flatten().collect())
        .delimited_by(
            just(Token::LBrace),
            just(Token::RBrace)
                .ignored()
                .recover_with(via_parser(unclosed)),
        )
        .boxed()
}

/// Read the reserved keyword `word` as a name, recording it for the
/// reserved-word error.
fn reserved_as_name<'tok, I>(
    word: Token,
    e: &mut MapExtra<'tok, '_, I, Extra<'tok>>,
) -> Spanned<Ident>
where
    I: ValueInput<'tok, Token = Token, Span = Span>,
{
    let span = e.span();
    let name = Ident(word.to_string());
    e.state().push(Note::Reserved(Spanned::new(word, span)));
    Spanned::new(name, span)
}

fn parser<'tok, I>() -> impl Parser<'tok, I, File, Extra<'tok>>
where
    I: ValueInput<'tok, Token = Token, Span = Span>,
{
    // --- Leaves ---

    // An identifier, or a contextual keyword read as a name.
    let plain_name = select! {
        Token::Ident(name) => name,
        token if token.is_contextual() => token.to_string(),
    }
    .map_with(|name, e| Spanned::new(Ident(name), e.span()));

    let reserved_word = select! { token if Token::is_reserved(&token) => token };

    // A name after a keyword (`port <name>`, `include <name>`, `[<name>.…`).
    // A reserved word there is still read as the name, and recorded so
    // each occurrence gets a reserved-word error; the parse goes on past it.
    let ident = choice((
        plain_name,
        reserved_word.map_with(|word, e| reserved_as_name(word, e)),
    ))
    .boxed();

    // A name that starts a statement: `<name>: Type;`, `<key>: value;`,
    // `<side>: ports;`, `<port> at (…);`. Here a reserved word is the name
    // only when `separator` follows it. Otherwise the branch fails on the
    // keyword itself without consuming it, so a misplaced keyword (`view`
    // inside a component) gets the error of the statements that could
    // start there, on the keyword, and not "expected ':'" one token on.
    let leading_name = |separator: Token| {
        choice((
            plain_name,
            reserved_word
                .then(any().rewind().or_not())
                .try_map_with(move |(word, next), e| {
                    if next.as_ref() == Some(&separator) {
                        Ok(reserved_as_name(word, e))
                    } else {
                        // Expecting nothing itself, this error leaves the
                        // other statements' expected set as it was.
                        Err(
                            LabelError::<'tok, I, RichPattern<'tok, Token>>::expected_found(
                                [],
                                Some(MaybeRef::Val(word)),
                                e.span(),
                            ),
                        )
                    }
                }),
        ))
        .boxed()
    };

    let string = select! { Token::Str(s) => s }.map_with(|s, e| Spanned::new(s, e.span()));

    let number = select! { Token::Number(n) => n }.try_map(|n, span: Span| {
        n.parse::<f64>()
            .map(|v| Spanned::new(v, span))
            .map_err(|_| Rich::custom(span, format!("invalid number `{n}`")))
    });

    let pin = select! { Token::Number(n) => n }.try_map(|n, span: Span| {
        n.parse::<u32>()
            .map(|v| Spanned::new(v, span))
            .map_err(|_| Rich::custom(span, format!("pin must be a whole number, got `{n}`")))
    });

    let connector_property = leading_name(Token::Colon)
        .then_ignore(just(Token::Colon))
        .then(choice((
            string.map(ConnectorPropertyValue::Str),
            number.map(ConnectorPropertyValue::Number),
        )))
        .then_ignore(just(Token::Semicolon))
        .map_with(|(key, value), e| ConnectorProperty {
            key,
            value,
            span: e.span(),
        });

    let connector_grid_layout = just(Token::Layout)
        .ignore_then(just(Token::Grid))
        .ignore_then(
            just(Token::Rows)
                .ignore_then(just(Token::Colon))
                .ignore_then(pin)
                .then_ignore(just(Token::Semicolon))
                .then(
                    just(Token::Cols)
                        .ignore_then(just(Token::Colon))
                        .ignore_then(pin)
                        .then_ignore(just(Token::Semicolon)),
                )
                .then(
                    just(Token::Numbering)
                        .ignore_then(just(Token::Colon))
                        .ignore_then(ident.clone())
                        .then_ignore(just(Token::Semicolon))
                        .or_not(),
                )
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((rows, cols), numbering), e| {
            ConnectorLayout::Grid(ConnectorGridLayout {
                rows,
                cols,
                numbering,
                span: e.span(),
            })
        });

    let connector_cavity = just(Token::Cavity)
        .ignore_then(pin)
        .then_ignore(just(Token::At))
        .then(
            number
                .then_ignore(just(Token::Comma))
                .then(number)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(just(Token::Size).ignore_then(ident.clone()).or_not())
        .then_ignore(just(Token::Semicolon))
        .map_with(|((pin, (x, y)), size), e| ConnectorCavity {
            pin,
            x,
            y,
            size,
            span: e.span(),
        });

    let connector_face_layout = just(Token::Layout)
        .ignore_then(just(Token::Face))
        .ignore_then(
            connector_cavity
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|cavities, e| {
            ConnectorLayout::Face(ConnectorFaceLayout {
                cavities,
                span: e.span(),
            })
        });

    let connector_type_item = choice((
        connector_face_layout.map(ConnectorTypeItem::Layout),
        connector_grid_layout.map(ConnectorTypeItem::Layout),
        connector_property.map(ConnectorTypeItem::Property),
    ));

    let connector_type = just(Token::ConnectorType)
        .ignore_then(ident.clone())
        .then(string)
        .then(block(connector_type_item))
        .map_with(|((name, description), items), e| {
            let mut properties = Vec::new();
            let mut layout = None;
            for item in items {
                match item {
                    ConnectorTypeItem::Property(property) => properties.push(property),
                    ConnectorTypeItem::Layout(next_layout) => layout = Some(next_layout),
                }
            }
            ConnectorType {
                name,
                description,
                properties,
                layout,
                span: e.span(),
            }
        })
        .boxed();

    // --- Ports & connectors ---

    let single_pin = just(Token::Pin).ignore_then(pin).map(|p| vec![p]);
    let multi_pins = just(Token::Pins).ignore_then(
        pin.separated_by(just(Token::Comma))
            .allow_trailing()
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBracket), just(Token::RBracket)),
    );
    let pins = choice((single_pin, multi_pins))
        .or_not()
        .map(|o| o.unwrap_or_default());

    let port = just(Token::Pub)
        .or_not()
        .then_ignore(just(Token::Port))
        .then(ident.clone())
        .then(string)
        .then(pins)
        .then_ignore(just(Token::Semicolon))
        .map_with(|(((vis, name), label), pins), e| Port {
            visibility: if vis.is_some() {
                Visibility::Public
            } else {
                Visibility::Private
            },
            name,
            label,
            pins,
            span: e.span(),
        })
        .boxed();

    let connector = just(Token::Connector)
        .ignore_then(ident.clone())
        .then(string.or_not())
        .then(block(port.clone()))
        .map_with(|((name, description), ports), e| Connector {
            name,
            description,
            ports,
            span: e.span(),
        })
        .boxed();

    let connector_instance = just(Token::Connector)
        .ignore_then(ident.clone())
        .then_ignore(just(Token::Colon))
        .then(ident.clone())
        .then(block(port.clone()))
        .map_with(|((name, type_name), ports), e| ConnectorInstance {
            name,
            type_name,
            ports,
            span: e.span(),
        })
        .boxed();

    // --- Inline (mid-harness) connectors ---

    // A `<key>: <Type>;` housing-half line. The key is any identifier here
    // (`male`/`female` are validated in resolve, not the grammar) so a typo
    // gets a targeted diagnostic instead of a generic parse error.
    let inline_half = leading_name(Token::Colon)
        .then_ignore(just(Token::Colon))
        .then(ident.clone())
        .then_ignore(just(Token::Semicolon))
        .map_with(|(key, type_name), e| InlineHalfDecl {
            key,
            type_name,
            span: e.span(),
        });

    // Half lines and ports may interleave; the fold partitions them, each
    // keeping its source order.
    let inline_item = choice((
        inline_half.map(InlineItem::Half),
        port.clone().map(InlineItem::Port),
    ));

    let inline = just(Token::Inline)
        .ignore_then(ident.clone())
        .then(string.or_not())
        .then(block(inline_item))
        .map_with(|((name, label), items), e| {
            let mut halves = Vec::new();
            let mut ports = Vec::new();
            for item in items {
                match item {
                    InlineItem::Half(h) => halves.push(h),
                    InlineItem::Port(p) => ports.push(p),
                }
            }
            Inline {
                name,
                label,
                halves,
                ports,
                span: e.span(),
            }
        })
        .boxed();

    // --- Instances ---

    let instance = leading_name(Token::Colon)
        .then_ignore(just(Token::Colon))
        .then(ident.clone())
        .then(string.or_not())
        .then_ignore(just(Token::Semicolon))
        .map_with(|((name, type_name), label), e| Instance {
            type_name,
            name,
            label,
            span: e.span(),
        })
        .boxed();

    // --- Wires ---

    let endpoint = ident
        .clone()
        .then(just(Token::Dot).ignore_then(ident.clone()).or_not())
        .map_with(|(first, second), e| match second {
            Some(port) => Endpoint {
                instance: Some(first),
                port,
                span: e.span(),
            },
            None => Endpoint {
                instance: None,
                port: first,
                span: e.span(),
            },
        });

    // A color is `base` or two-tone `base/tracer` (`green/yellow`).
    let wire = just(Token::Wire)
        .ignore_then(ident.clone())
        .then(just(Token::Slash).ignore_then(ident.clone()).or_not())
        .then(number)
        .then(string.or_not())
        .then(
            endpoint
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBracket), just(Token::RBracket)),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|((((color, tracer), gauge), label), endpoints), e| Wire {
            color,
            tracer,
            gauge,
            label,
            endpoints,
            span: e.span(),
        })
        .boxed();

    // --- Cables ---

    let cable_property = leading_name(Token::Colon)
        .then_ignore(just(Token::Colon))
        .then(choice((
            string.map(CablePropertyValue::Str),
            number.map(CablePropertyValue::Number),
            ident.clone().map(CablePropertyValue::Ident),
        )))
        .then_ignore(just(Token::Semicolon))
        .map_with(|(key, value), e| CableProperty {
            key,
            value,
            span: e.span(),
        });

    // `twisted { <wire> <wire> }` — a twisted pair. Exactly two
    // conductors, converted from the repeated parse by `try_map` so a
    // wrong count is a parse error with a purpose-built message.
    // (Deliberately NOT `wire.then(wire)`: that embeds the huge wire
    // combinator type twice and blows rustc's trait solving up —
    // compile times went from seconds to unbounded.)
    let twisted_group = just(Token::Twisted)
        .ignore_then(
            wire.clone()
                .repeated()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .try_map_with(|wires, e| {
            let pair: [Wire; 2] = wires.try_into().map_err(|_| {
                Rich::custom(e.span(), "a twisted pair needs exactly two conductors")
            })?;
            Ok(TwistedGroup {
                wires: Box::new(pair),
                span: e.span(),
            })
        });

    // Properties and conductor entries may interleave in any order; the
    // fold partitions them, each keeping its source order.
    let cable_item = choice((
        cable_property.map(CableItem::Property),
        twisted_group.map(|t| CableItem::Member(CableMember::Twisted(t))),
        wire.clone()
            .map(|w| CableItem::Member(CableMember::Wire(w))),
    ));

    let cable = just(Token::Cable)
        .ignore_then(ident.clone())
        .then(string.or_not())
        .then(block(cable_item))
        .map_with(|((name, label), items), e| {
            let mut properties = Vec::new();
            let mut members = Vec::new();
            for item in items {
                match item {
                    CableItem::Property(p) => properties.push(p),
                    CableItem::Member(m) => members.push(m),
                }
            }
            Cable {
                name,
                label,
                properties,
                members,
                span: e.span(),
            }
        })
        .boxed();

    // --- Views ---

    // A `<side>: <port>, <port>;` line, flattened to one placement per port
    // (all sharing the line's side and span).
    let port_line = leading_name(Token::Colon)
        .then_ignore(just(Token::Colon))
        .then(
            ident
                .clone()
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|(side, ports), e| {
            let span = e.span();
            ports
                .into_iter()
                .map(move |port| PortPlacement {
                    side: side.clone(),
                    port,
                    span,
                })
                .collect::<Vec<_>>()
        });

    let ports_block = just(Token::Ports)
        .ignore_then(block(port_line))
        .map(|lines| lines.into_iter().flatten().collect::<Vec<PortPlacement>>());

    // One anchor slot: a grid coordinate or a side keyword. Resolve checks
    // that exactly one of an enclosure port's two slots is a side.
    let anchor = choice((number.map(Anchor::Coord), ident.clone().map(Anchor::Edge)));

    // `<port> at (<x>, <y>);` — a subject port placed on the enclosure edge.
    let enclosure_port = leading_name(Token::At)
        .then_ignore(just(Token::At))
        .then(
            anchor
                .clone()
                .then_ignore(just(Token::Comma))
                .then(anchor)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then_ignore(just(Token::Semicolon))
        .map_with(|(port, (x, y)), e| EnclosurePort {
            port,
            x,
            y,
            span: e.span(),
        });

    let enclosure_block = just(Token::Enclosure)
        .ignore_then(block(enclosure_port))
        .map_with(|ports, e| Spanned::new(ports, e.span()));

    let include = just(Token::Include)
        .ignore_then(ident.clone())
        .then(just(Token::Dot).ignore_then(ident.clone()).or_not())
        .then_ignore(just(Token::At))
        .then(
            number
                .then_ignore(just(Token::Comma))
                .then(number)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(choice((
            // A ports block ends the include — no trailing `;` after `}`.
            ports_block.map(Some),
            just(Token::Semicolon).to(None),
        )))
        .map_with(|(((instance, connector), (x, y)), ports), e| Include {
            instance,
            connector,
            x,
            y,
            ports: ports.unwrap_or_default(),
            span: e.span(),
        });

    let text_box = just(Token::Text)
        .ignore_then(ident.clone())
        .then_ignore(just(Token::At))
        .then(
            number
                .then_ignore(just(Token::Comma))
                .then(number)
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(string)
        .then_ignore(just(Token::Semicolon))
        .map_with(|((name, (x, y)), label), e| TextBox {
            name,
            x,
            y,
            label,
            span: e.span(),
        });

    let grid = just(Token::Grid)
        .ignore_then(just(Token::Colon))
        .ignore_then(number)
        .then_ignore(just(Token::Semicolon));

    // The view body is a free-order list of these; the fold below keeps the
    // first `grid`/`enclosure` and records any extras for a duplicate report.
    let view_item = choice((
        grid.map(ViewItem::Grid),
        enclosure_block.map(ViewItem::Enclosure),
        include.map(ViewItem::Include),
        text_box.map(ViewItem::Text),
    ));

    let view = just(Token::View)
        .ignore_then(ident.clone())
        .then(string)
        .then(block(view_item))
        .map_with(|((kind, title), items), e| {
            let mut grid = None;
            let mut enclosure = None;
            let mut includes = Vec::new();
            let mut texts = Vec::new();
            let mut duplicate_items = Vec::new();
            for item in items {
                match item {
                    ViewItem::Grid(g) if grid.is_some() => {
                        duplicate_items.push(Spanned::new("grid", g.span));
                    }
                    ViewItem::Grid(g) => grid = Some(g),
                    ViewItem::Enclosure(block) if enclosure.is_some() => {
                        duplicate_items.push(Spanned::new("enclosure", block.span));
                    }
                    ViewItem::Enclosure(block) => enclosure = Some(block.node),
                    ViewItem::Include(inc) => includes.push(inc),
                    ViewItem::Text(text) => texts.push(text),
                }
            }
            let has_enclosure = enclosure.is_some();
            View {
                kind,
                title,
                grid,
                has_enclosure,
                enclosure: enclosure.unwrap_or_default(),
                includes,
                texts,
                duplicate_items,
                span: e.span(),
            }
        })
        .boxed();

    // --- Definitions (recursive: components nest) ---

    let definition = recursive(|definition| {
        let member = choice((
            port.map(Member::Port),
            connector_instance.map(Member::ConnectorInstance),
            connector.map(Member::Connector),
            inline.map(Member::Inline),
            wire.map(Member::Wire),
            cable.map(Member::Cable),
            definition.map(Member::Definition),
            instance.map(Member::Instance),
        ));
        let head = choice((
            just(Token::Component).to(DefKind::Component),
            just(Token::Extend).to(DefKind::Extend),
        ));
        head.then(ident.clone())
            .then(block(member))
            .map_with(|((kind, name), members), e| Definition {
                kind,
                name,
                members,
                span: e.span(),
            })
    });

    // --- File ---

    let item = choice((
        definition.map(Item::Definition),
        connector_type.map(Item::ConnectorType),
        view.map(Item::View),
    ));

    let use_decl = just(Token::Use)
        .ignore_then(ident)
        .then_ignore(just(Token::From))
        .then(string)
        .then_ignore(just(Token::Semicolon))
        .map_with(|(name, path), e| Use {
            name,
            path,
            span: e.span(),
        });

    let file_entry = choice((use_decl.map(FileEntry::Use), item.map(FileEntry::Item)));

    // The file reads like a block that ends at the end of input: a broken
    // entry is reported and skipped (a stray `}` alone, since no block
    // here could close on it), and the parse reads on.
    let stray_close = just(Token::RBrace);
    choice((file_entry.map(Some), never_but_expect(end())))
        .recover_with(via_parser(skipped(choice((stray_close, skip_statement())))))
        .repeated()
        .collect::<Vec<Option<FileEntry>>>()
        .then_ignore(end())
        .map_with(|entries, e| {
            let mut uses = Vec::new();
            let mut items = Vec::new();
            for entry in entries.into_iter().flatten() {
                match entry {
                    FileEntry::Use(use_decl) => uses.push(use_decl),
                    FileEntry::Item(item) => items.push(item),
                }
            }
            let span: Span = e.span();
            File {
                id: span.file,
                uses,
                items,
                span,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::lex::{lex, significant};

    fn parse_str(src: &str) -> Parsed {
        let lexemes = lex(src, FileId(0)).expect("lexes");
        parse_file(significant(&lexemes), FileId(0), src.len())
    }

    fn parse_ok(src: &str) -> File {
        let parsed = parse_str(src);
        assert!(
            parsed.errors.is_empty(),
            "unexpected parse errors: {:?}",
            parsed.errors
        );
        parsed.file.expect("a file")
    }

    /// Members of the sole top-level definition.
    fn members(file: &File) -> &[Member] {
        match &file.items[0] {
            Item::Definition(d) => &d.members,
            other => panic!("expected a definition, got {other:?}"),
        }
    }

    #[test]
    fn pub_port_with_pin() {
        let file = parse_ok(r#"component c { pub port hv_pos "HV+" pin 1; }"#);
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(p.visibility, Visibility::Public);
        assert_eq!(p.name.node.as_str(), "hv_pos");
        assert_eq!(p.label.node, "HV+");
        assert_eq!(p.pins.iter().map(|s| s.node).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn bare_private_port_has_no_pins() {
        let file = parse_ok(r#"component c { port coil_n "COIL-"; }"#);
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(p.visibility, Visibility::Private);
        assert!(p.pins.is_empty());
    }

    #[test]
    fn port_with_ganged_pins() {
        let file = parse_ok(r#"component c { pub port gnd "GND" pins [2, 3, 4]; }"#);
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(
            p.pins.iter().map(|s| s.node).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn connector_groups_ports() {
        let file = parse_ok(
            r#"component c { connector can "CAN 4p" { pub port can_h "CAN H" pin 1; pub port can_l "CAN L" pin 2; } }"#,
        );
        let Member::Connector(conn) = &members(&file)[0] else {
            panic!("expected a connector");
        };
        assert_eq!(conn.name.node.as_str(), "can");
        assert_eq!(
            conn.description.as_ref().map(|d| d.node.as_str()),
            Some("CAN 4p")
        );
        assert_eq!(conn.ports.len(), 2);
    }

    #[test]
    fn connector_description_is_optional() {
        let file = parse_ok(r#"component c { connector hv { pub port p "DC+" pin 1; } }"#);
        let Member::Connector(conn) = &members(&file)[0] else {
            panic!("expected a connector");
        };
        assert_eq!(conn.name.node.as_str(), "hv");
        assert!(conn.description.is_none(), "description string is optional");
    }

    #[test]
    fn inline_carries_halves_and_ports() {
        let file = parse_ok(
            r#"component c {
                inline ic_pedal "Pedal branch" {
                    male:   Dt04_6p;
                    female: Dt06_6s;
                    port apps1 "APPS1" pin 1;
                    port apps2 "APPS2" pin 2;
                    port gnd   "GND"   pin 3;
                }
            }"#,
        );
        let Member::Inline(inline) = &members(&file)[0] else {
            panic!("expected an inline");
        };
        assert_eq!(inline.name.node.as_str(), "ic_pedal");
        assert_eq!(
            inline.label.as_ref().map(|l| l.node.as_str()),
            Some("Pedal branch")
        );
        let halves: Vec<(&str, &str)> = inline
            .halves
            .iter()
            .map(|h| (h.key.node.as_str(), h.type_name.node.as_str()))
            .collect();
        assert_eq!(halves, vec![("male", "Dt04_6p"), ("female", "Dt06_6s")]);
        assert_eq!(inline.ports.len(), 3);
        assert_eq!(inline.ports[0].name.node.as_str(), "apps1");
        assert_eq!(
            inline.ports[2]
                .pins
                .iter()
                .map(|s| s.node)
                .collect::<Vec<_>>(),
            vec![3]
        );
    }

    #[test]
    fn inline_label_and_halves_are_optional() {
        let file = parse_ok(r#"component c { inline ic { port sig "SIG" pin 1; } }"#);
        let Member::Inline(inline) = &members(&file)[0] else {
            panic!("expected an inline");
        };
        assert!(inline.label.is_none());
        assert!(inline.halves.is_empty());
        assert_eq!(inline.ports.len(), 1);
    }

    #[test]
    fn inline_half_lines_and_ports_interleave() {
        let file = parse_ok(
            r#"component c { inline ic { port a "A" pin 1; female: F2p; port b "B" pin 2; } }"#,
        );
        let Member::Inline(inline) = &members(&file)[0] else {
            panic!("expected an inline");
        };
        assert_eq!(inline.halves.len(), 1);
        assert_eq!(inline.ports.len(), 2);
        assert_eq!(inline.ports[1].name.node.as_str(), "b");
    }

    #[test]
    fn inline_accepts_pub_ports() {
        // `pub` parses (shared port grammar); resolve warns it has no effect.
        let file = parse_ok(r#"component c { inline ic { pub port sig "SIG" pin 1; } }"#);
        let Member::Inline(inline) = &members(&file)[0] else {
            panic!("expected an inline");
        };
        assert_eq!(inline.ports[0].visibility, Visibility::Public);
    }

    #[test]
    fn inline_without_designator_is_a_parse_error() {
        let parsed = parse_str(r#"component c { inline { port sig "SIG" pin 1; } }"#);
        assert!(!parsed.errors.is_empty(), "expected a parse error");
    }

    #[test]
    fn connector_type_carries_shared_metadata() {
        let file = parse_ok(
            r#"connector_type ampseal_35p "AMPSEAL 35p" { part: "TE 776164-1"; cavities: 35; }"#,
        );
        let Item::ConnectorType(conn) = &file.items[0] else {
            panic!("expected a connector type");
        };
        assert_eq!(conn.name.node.as_str(), "ampseal_35p");
        assert_eq!(conn.description.node, "AMPSEAL 35p");
        assert_eq!(conn.properties.len(), 2);
        assert_eq!(conn.properties[0].key.node.as_str(), "part");
        assert!(matches!(
            conn.properties[1].value,
            ConnectorPropertyValue::Number(_)
        ));
        assert!(conn.layout.is_none());
    }

    #[test]
    fn connector_type_carries_grid_layout() {
        let file = parse_ok(
            r#"connector_type mx150_8 "Molex MX-150 8p" {
                part: "Molex";
                layout grid {
                    rows: 2;
                    cols: 4;
                    numbering: row_major;
                }
            }"#,
        );
        let Item::ConnectorType(conn) = &file.items[0] else {
            panic!("expected a connector type");
        };
        let Some(ConnectorLayout::Grid(layout)) = &conn.layout else {
            panic!("expected a grid layout");
        };
        assert_eq!(layout.rows.node, 2);
        assert_eq!(layout.cols.node, 4);
        assert_eq!(
            layout.numbering.as_ref().map(|n| n.node.as_str()),
            Some("row_major")
        );
    }

    #[test]
    fn connector_type_carries_explicit_face_layout() {
        let file = parse_ok(
            r#"connector_type inv_control "Inverter control" {
                layout face {
                    cavity 47 at (0, 0) size large;
                    cavity 21 at (2, 0);
                }
            }"#,
        );
        let Item::ConnectorType(conn) = &file.items[0] else {
            panic!("expected a connector type");
        };
        let Some(ConnectorLayout::Face(layout)) = &conn.layout else {
            panic!("expected a face layout");
        };
        assert_eq!(layout.cavities.len(), 2);
        assert_eq!(layout.cavities[0].pin.node, 47);
        assert_eq!(
            layout.cavities[0].size.as_ref().map(|s| s.node.as_str()),
            Some("large")
        );
        assert_eq!(layout.cavities[1].pin.node, 21);
        assert!(layout.cavities[1].size.is_none());
    }

    #[test]
    fn connector_type_layouts_snapshot() {
        let file = parse_ok(
            r#"connector_type linear_8p "Linear 8p" {
                layout grid {
                    rows: 1;
                    cols: 8;
                    numbering: row_major;
                }
            }

            connector_type dual_8p "Dual 8p" {
                layout grid {
                    rows: 2;
                    cols: 8;
                    numbering: odd_even;
                }
            }

            connector_type inverter_control "Inverter control" {
                part: "OEM";
                layout face {
                    cavity 47 at (1, 0) size large;
                    cavity 21 at (5, 0);
                    cavity 13 at (16, 3);
                }
            }"#,
        );

        let mut summary = String::new();
        for item in &file.items {
            let Item::ConnectorType(connector) = item else {
                continue;
            };
            summary.push_str(connector.name.node.as_str());
            summary.push_str(" = ");
            match connector.layout.as_ref().expect("layout") {
                ConnectorLayout::Grid(layout) => {
                    summary.push_str(&format!(
                        "grid {}x{} {:?}\n",
                        layout.rows.node,
                        layout.cols.node,
                        layout.numbering.as_ref().map(|n| n.node.as_str())
                    ));
                }
                ConnectorLayout::Face(layout) => {
                    summary.push_str("face");
                    for cavity in &layout.cavities {
                        summary.push_str(&format!(
                            " {}@({}, {})",
                            cavity.pin.node, cavity.x.node, cavity.y.node
                        ));
                        if let Some(size) = &cavity.size {
                            summary.push_str(&format!(" {}", size.node));
                        }
                    }
                    summary.push('\n');
                }
            }
        }

        insta::assert_snapshot!(summary, @r###"
linear_8p = grid 1x8 Some("row_major")
dual_8p = grid 2x8 Some("odd_even")
inverter_control = face 47@(1, 0) large 21@(5, 0) 13@(16, 3)
"###);
    }

    #[test]
    fn connector_instance_declares_ports_with_pins() {
        let file = parse_ok(
            r#"component c {
                connector x1: ampseal_35p {
                    pub port can_h "CAN H" pin 1;
                    pub port b_pos "B+" pins [2, 3];
                }
            }"#,
        );
        let Member::ConnectorInstance(conn) = &members(&file)[0] else {
            panic!("expected a connector instance");
        };
        assert_eq!(conn.name.node.as_str(), "x1");
        assert_eq!(conn.type_name.node.as_str(), "ampseal_35p");
        assert_eq!(conn.ports.len(), 2);
        assert_eq!(conn.ports[0].name.node.as_str(), "can_h");
        assert_eq!(conn.ports[0].pins[0].node, 1);
        // Ganged cavities ride one declaration, same as inline connectors.
        assert_eq!(
            conn.ports[1]
                .pins
                .iter()
                .map(|p| p.node)
                .collect::<Vec<_>>(),
            [2, 3]
        );
    }

    #[test]
    fn wire_carries_label() {
        let file = parse_ok(r#"component c { wire orange 50 "HV+" [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.label.as_ref().map(|l| l.node.as_str()), Some("HV+"));
        assert_eq!(w.endpoints.len(), 2);
        // Unlabelled wires still parse.
        let bare = parse_ok(r#"component c { wire orange 50 [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&bare)[0] else {
            panic!("expected wire");
        };
        assert!(w.label.is_none());
    }

    #[test]
    fn cable_groups_wires_with_properties() {
        let file = parse_ok(
            r#"component c {
                cable can_bus "Vehicle CAN" {
                    type:   "Twisted pair";
                    length: 2.5;
                    wire yellow 0.5 "CAN H" [vcu.can_h, inv.can_h];
                    wire green  0.5 "CAN L" [vcu.can_l, inv.can_l];
                }
            }"#,
        );
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert_eq!(cable.name.node.as_str(), "can_bus");
        assert_eq!(
            cable.label.as_ref().map(|l| l.node.as_str()),
            Some("Vehicle CAN")
        );
        assert_eq!(cable.properties.len(), 2);
        assert_eq!(cable.properties[0].key.node.as_str(), "type");
        assert!(matches!(
            cable.properties[1].value,
            CablePropertyValue::Number(_)
        ));
        assert_eq!(cable.wires().count(), 2);
    }

    #[test]
    fn cable_properties_and_wires_may_interleave() {
        // A wire before a property, and a property between wires — the fold
        // partitions them, each side keeping source order.
        let file = parse_ok(
            r#"component c {
                cable can_bus {
                    wire yellow 0.5 [vcu.can_h, inv.can_h];
                    type:   "Twisted pair";
                    wire green  0.5 [vcu.can_l, inv.can_l];
                    length: 2.5;
                }
            }"#,
        );
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert_eq!(cable.properties.len(), 2);
        assert_eq!(cable.properties[0].key.node.as_str(), "type");
        let colors: Vec<&str> = cable.wires().map(|w| w.color.node.as_str()).collect();
        assert_eq!(colors, ["yellow", "green"]);
    }

    #[test]
    fn twisted_group_wraps_its_wires() {
        let file = parse_ok(
            r#"component c {
                cable loom {
                    wire red 1.5 "12V" [a.pwr, b.pwr];
                    twisted {
                        wire white/blue 0.5 [a.h, b.h];
                        wire white/red 0.5 [a.l, b.l];
                    }
                    wire black 1.5 [a.gnd, b.gnd];
                }
            }"#,
        );
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert_eq!(cable.members.len(), 3);
        assert!(matches!(cable.members[0], CableMember::Wire(_)));
        let CableMember::Twisted(t) = &cable.members[1] else {
            panic!("expected a twisted group");
        };
        assert_eq!(t.wires[0].color.node.as_str(), "white");
        assert!(matches!(cable.members[2], CableMember::Wire(_)));
        // The flattening helper sees all four conductors in source order.
        assert_eq!(cable.wires().count(), 4);
    }

    #[test]
    fn twisted_group_must_hold_exactly_two_wires() {
        // The pair count is grammar-enforced, not a later diagnostic.
        for body in [
            "twisted { }",
            "twisted { wire red 1 [a.p, b.p]; }",
            "twisted { wire red 1 [a.p, b.p]; wire blue 1 [a.q, b.q]; wire green 1 [a.r, b.r]; }",
        ] {
            let parsed = parse_str(&format!("component c {{ cable cab {{ {body} }} }}"));
            assert!(!parsed.errors.is_empty(), "{body} should not parse");
        }
    }

    #[test]
    fn cable_without_label_or_properties_parses() {
        let file = parse_ok(r#"component c { cable cab { wire red 1.0 [a.p, b.p]; } }"#);
        let Member::Cable(cable) = &members(&file)[0] else {
            panic!("expected a cable");
        };
        assert!(cable.label.is_none());
        assert!(cable.properties.is_empty());
        assert_eq!(cable.wires().count(), 1);
    }

    #[test]
    fn harness_include_references_connector() {
        let file = parse_ok(r#"view harness "H" { include charger.hv at (2, 4); }"#);
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        assert_eq!(v.kind.node.as_str(), "harness");
        let inc = &v.includes[0];
        assert_eq!(inc.instance.node.as_str(), "charger");
        assert_eq!(inc.connector.as_ref().map(|c| c.node.as_str()), Some("hv"));
        assert!(inc.ports.is_empty());
    }

    #[test]
    fn instance_with_and_without_label() {
        let file =
            parse_ok(r#"component c { pack: cell_pack; front: front_battery "Front Battery"; }"#);
        let Member::Instance(a) = &members(&file)[0] else {
            panic!("expected instance");
        };
        assert_eq!(a.type_name.node.as_str(), "cell_pack");
        assert_eq!(a.name.node.as_str(), "pack");
        assert!(a.label.is_none());
        let Member::Instance(b) = &members(&file)[1] else {
            panic!("expected instance");
        };
        assert_eq!(
            b.label.as_ref().map(|l| l.node.as_str()),
            Some("Front Battery")
        );
    }

    #[test]
    fn wire_endpoints_bare_and_qualified() {
        let file = parse_ok(r#"component c { wire orange 50 [hv_pos, pack.hv_pos, inv.dc_pos]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.color.node.as_str(), "orange");
        assert_eq!(w.gauge.node, 50.0);
        assert_eq!(w.endpoints.len(), 3);
        // bare self-port
        assert!(w.endpoints[0].instance.is_none());
        assert_eq!(w.endpoints[0].port.node.as_str(), "hv_pos");
        // instance-qualified
        assert_eq!(
            w.endpoints[1].instance.as_ref().unwrap().node.as_str(),
            "pack"
        );
        assert_eq!(w.endpoints[1].port.node.as_str(), "hv_pos");
    }

    #[test]
    fn two_tone_wire_color_parses_base_and_tracer() {
        let file = parse_ok(r#"component c { wire green/yellow 2.5 "PE" [a.pe, b.pe]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.color.node.as_str(), "green");
        assert_eq!(w.tracer.as_ref().map(|t| t.node.as_str()), Some("yellow"));
    }

    #[test]
    fn single_color_wire_has_no_tracer() {
        let file = parse_ok(r#"component c { wire orange 50 [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert!(w.tracer.is_none());
    }

    #[test]
    fn dangling_color_slash_is_a_parse_error() {
        let parsed = parse_str(r#"component c { wire green/ 2.5 [a.p, b.p]; }"#);
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn fractional_gauge_parses() {
        let file = parse_ok(r#"component c { wire black 0.25 [a.p, b.p]; }"#);
        let Member::Wire(w) = &members(&file)[0] else {
            panic!("expected wire");
        };
        assert_eq!(w.gauge.node, 0.25);
    }

    #[test]
    fn trailing_commas_are_allowed_in_lists() {
        let file = parse_ok(
            r#"component c {
                pub port gnd "GND" pins [2, 3,];
                wire orange 50 [
                    a.p,
                    b.p,
                ];
            }
            view schematic "V" { include a at (1, 2) ports { west: p, q,; } }"#,
        );
        let Member::Port(p) = &members(&file)[0] else {
            panic!("expected a port");
        };
        assert_eq!(p.pins.len(), 2);
        let Member::Wire(w) = &members(&file)[1] else {
            panic!("expected a wire");
        };
        assert_eq!(w.endpoints.len(), 2);
        let Item::View(v) = &file.items[1] else {
            panic!("expected a view");
        };
        assert_eq!(v.includes[0].ports.len(), 2);
    }

    #[test]
    fn negative_coordinates_parse() {
        let file = parse_ok(r#"view schematic "V" { include a at (-2, -0.5); }"#);
        let Item::View(v) = &file.items[0] else {
            panic!("expected a view");
        };
        assert_eq!((v.includes[0].x.node, v.includes[0].y.node), (-2.0, -0.5));
    }

    #[test]
    fn nested_definition() {
        let file = parse_ok(r#"component outer { component inner { pub port a "A"; } i: inner; }"#);
        let ms = members(&file);
        assert!(matches!(ms[0], Member::Definition(_)));
        assert!(matches!(ms[1], Member::Instance(_)));
    }

    #[test]
    fn view_with_and_without_grid() {
        let file = parse_ok(
            r#"view schematic "Overview" { grid: 20; include a at (3, 4); include b at (5, 6); }"#,
        );
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        assert_eq!(v.kind.node.as_str(), "schematic");
        assert_eq!(v.title.node, "Overview");
        assert_eq!(v.grid.as_ref().map(|g| g.node), Some(20.0));
        assert_eq!(v.includes.len(), 2);
        assert_eq!(v.includes[0].instance.node.as_str(), "a");
        assert_eq!((v.includes[0].x.node, v.includes[0].y.node), (3.0, 4.0));

        let no_grid = parse_ok(r#"view schematic "X" { include a at (0, 0); }"#);
        let Item::View(v) = &no_grid.items[0] else {
            panic!("expected view");
        };
        assert!(v.grid.is_none());
        assert!(v.includes[0].ports.is_empty(), "bare include has no ports");
    }

    #[test]
    fn include_ports_block_flattens_in_order() {
        let file = parse_ok(
            r#"view schematic "V" { include a at (1, 2) ports { west: p, q; east: r; } }"#,
        );
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        let placements = &v.includes[0].ports;
        let got: Vec<(&str, &str)> = placements
            .iter()
            .map(|pl| (pl.side.node.as_str(), pl.port.node.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![("west", "p"), ("west", "q"), ("east", "r")],
            "placements keep their declaration order, one per port"
        );
    }

    #[test]
    fn view_text_box_parses() {
        let file =
            parse_ok(r#"view schematic "V" { text note_1 at (3, 4) "This is my textbox!"; }"#);
        let Item::View(v) = &file.items[0] else {
            panic!("expected view");
        };
        assert_eq!(v.texts.len(), 1);
        assert_eq!(v.texts[0].name.node.as_str(), "note_1");
        assert_eq!((v.texts[0].x.node, v.texts[0].y.node), (3.0, 4.0));
        assert_eq!(v.texts[0].label.node, "This is my textbox!");
    }

    #[test]
    fn use_declaration() {
        let file = parse_ok(
            "use cell_module from \"components/cell_module.wb\";\ncomponent c { pub port a \"A\"; }",
        );
        assert_eq!(file.uses.len(), 1);
        assert_eq!(file.uses[0].name.node.as_str(), "cell_module");
        assert_eq!(file.uses[0].path.node, "components/cell_module.wb");
    }

    #[test]
    fn use_declarations_may_interleave_with_items() {
        let file = parse_ok(
            r#"
component a { }
use leaf from "leaf.wb";
view schematic "A" { }
use relay from "relay.wb";
component b { }
"#,
        );

        let uses: Vec<&str> = file.uses.iter().map(|u| u.name.node.as_str()).collect();
        assert_eq!(uses, vec!["leaf", "relay"]);
        assert_eq!(file.items.len(), 3);
        assert!(matches!(file.items[0], Item::Definition(_)));
        assert!(matches!(file.items[1], Item::View(_)));
        assert!(matches!(file.items[2], Item::Definition(_)));
    }

    #[test]
    fn contextual_keywords_are_names_wherever_a_name_belongs() {
        let file = parse_ok(
            r#"use from from "from.wb";
            component c {
                pub port size "SIZE";
                grid: Lamp;
                connector layout "L" { pub port text "T" pin 1; }
                connector face: Plug { pub port rows "R" pin 2; }
                inline cavity { female: Plug; port numbering "N" pin 3; }
                cable ports { wire red 1 [at.from, size]; }
            }
            view schematic "V" {
                include at at (1, 2) ports { west: cols, enclosure; }
                enclosure { include at (west, 1); }
                text grid at (0, 4) "note";
            }"#,
        );
        assert_eq!(file.uses[0].name.node.as_str(), "from");
        let names: Vec<&str> = members(&file)
            .iter()
            .map(|m| match m {
                Member::Port(p) => p.name.node.as_str(),
                Member::Instance(i) => i.name.node.as_str(),
                Member::Connector(c) => c.name.node.as_str(),
                Member::ConnectorInstance(c) => c.name.node.as_str(),
                Member::Inline(i) => i.name.node.as_str(),
                Member::Cable(c) => c.name.node.as_str(),
                other => panic!("unexpected member {other:?}"),
            })
            .collect();
        assert_eq!(names, ["size", "grid", "layout", "face", "cavity", "ports"]);
        let Item::View(v) = &file.items[1] else {
            panic!("expected a view");
        };
        assert_eq!(v.includes[0].instance.node.as_str(), "at");
        let placed: Vec<&str> = v.includes[0]
            .ports
            .iter()
            .map(|p| p.port.node.as_str())
            .collect();
        assert_eq!(placed, ["cols", "enclosure"]);
        assert_eq!(v.enclosure[0].port.node.as_str(), "include");
        assert_eq!(v.texts[0].name.node.as_str(), "grid");
    }

    #[test]
    fn reserved_word_as_a_name_is_reported_on_the_word_and_parsed_through() {
        let src = r#"component c { pub port wire "W" pin 1; port ok "OK"; }"#;
        let parsed = parse_str(src);
        let [ParseError::ReservedWord { span, word }] = parsed.errors.as_slice() else {
            panic!("expected one reserved-word error, got {:?}", parsed.errors);
        };
        assert_eq!(word, "wire");
        assert_eq!(&src[span.start..span.end], "wire");
        // The parse carried on: both ports are in the AST.
        let file = parsed.file.expect("a file");
        assert_eq!(members(&file).len(), 2);
    }

    #[test]
    fn reserved_word_as_an_instance_name_is_reported() {
        let parsed = parse_str("component c { cable: Lamp; }");
        let words: Vec<&str> = parsed
            .errors
            .iter()
            .map(|e| match e {
                ParseError::ReservedWord { word, .. } => word.as_str(),
                ParseError::Syntax { message, .. } => message.as_str(),
            })
            .collect();
        assert_eq!(words, ["cable"]);
    }

    /// Each error of `src` as `<the text under its span>: <message>`.
    fn errors_at(src: &str) -> Vec<String> {
        parse_str(src)
            .errors
            .iter()
            .map(|e| {
                let span = e.span();
                let message = match e {
                    ParseError::Syntax { message, .. } => message.clone(),
                    ParseError::ReservedWord { word, .. } => format!("reserved word `{word}`"),
                };
                format!("{}: {message}", &src[span.start..span.end])
            })
            .collect()
    }

    #[test]
    fn misplaced_keyword_at_a_statement_start_fails_on_the_keyword() {
        // A reserved word may lead a `<name>:` statement only when the `:`
        // follows; anywhere else the error is the one the statements that
        // could start there give, on the keyword itself.
        let cases = [
            r#"component C { view schematic "V" { } }"#,
            r#"component C { use X from "x.wb"; }"#,
            r#"component C { connector_type T "t" { } }"#,
            "component C { twisted { } }",
            "component C { pin 1; }",
            r#"component C { cable k { port a "A"; } }"#,
            r#"connector_type T "t" { wire "x"; }"#,
            "component C { inline ic { cable Plug; } }",
            r#"view schematic "V" { include a at (0, 0) ports { view west: a; } }"#,
            r#"view schematic "V" { enclosure { wire (west, 1); } }"#,
        ];
        let report: Vec<String> = cases.iter().flat_map(|src| errors_at(src)).collect();
        insta::assert_snapshot!(report.join("\n"), @r"
        view: found 'view' expected 'pub', 'port', 'connector', 'inline', 'wire', 'cable', 'component', 'extend', something else, or '}'
        use: found 'use' expected 'pub', 'port', 'connector', 'inline', 'wire', 'cable', 'component', 'extend', something else, or '}'
        connector_type: found 'connector_type' expected 'pub', 'port', 'connector', 'inline', 'wire', 'cable', 'component', 'extend', something else, or '}'
        twisted: found 'twisted' expected 'pub', 'port', 'connector', 'inline', 'wire', 'cable', 'component', 'extend', something else, or '}'
        pin: found 'pin' expected 'pub', 'port', 'connector', 'inline', 'wire', 'cable', 'component', 'extend', something else, or '}'
        port: found 'port' expected something else, 'twisted', 'wire', or '}'
        wire: found 'wire' expected 'layout', something else, or '}'
        cable: found 'cable' expected something else, 'pub', 'port', or '}'
        view: found 'view' expected something else, or '}'
        wire: found 'wire' expected something else, or '}'
        ");
    }

    #[test]
    fn reserved_word_leading_a_name_statement_is_a_reserved_word_error() {
        let cases = [
            "component C { pin: Plug; }",
            r#"connector_type T "t" { port: "x"; }"#,
            "component C { inline ic { wire: Plug; } }",
            r#"component C { cable k { view: "x"; } }"#,
            r#"view schematic "V" { include a at (0, 0) ports { wire: a; } }"#,
            r#"view schematic "V" { enclosure { wire at (west, 1); } }"#,
        ];
        let report: Vec<String> = cases.iter().flat_map(|src| errors_at(src)).collect();
        assert_eq!(
            report,
            [
                "pin: reserved word `pin`",
                "port: reserved word `port`",
                "wire: reserved word `wire`",
                "view: reserved word `view`",
                "wire: reserved word `wire`",
                "wire: reserved word `wire`",
            ]
        );
    }

    #[test]
    fn reserved_words_read_as_names_on_an_abandoned_branch_are_not_reported() {
        // An inline body and a cable body try a `<name>:` line before the
        // statement that starts with `port`/`wire`/`twisted`; that attempt
        // backtracks and must leave no error behind.
        let parsed = parse_str(
            r#"component c {
                inline ic { port a "A" pin 1; pub port b "B" pin 2; }
                cable cab {
                    wire red 1 [x.a, y.a];
                    twisted { wire red 1 [x.b, y.b]; wire blue 1 [x.c, y.c]; }
                }
            }"#,
        );
        assert!(parsed.is_ok(), "{:?}", parsed.errors);
    }

    #[test]
    fn missing_semicolon_is_a_parse_error() {
        let parsed = parse_str(r#"component c { pub port a "A" }"#);
        assert!(!parsed.errors.is_empty());
    }

    /// One broken statement in each kind of block, and one at the top.
    const BROKEN_EVERYWHERE: &str = r#"use Lamp from ;
connector_type Plug "P" { part "x"; cavities: 2; }
component C {
    pub port a "A" pin;
    pub port b "B" pin 1;
    connector x1: Plug { pub port c "C" pin; pub port d "D" pin 2; }
    connector grp { port e "E" pins [1,; port f "F"; }
    inline ic { male Plug; port g "G" pin 1; }
    cable k { length: ; wire red 1 [l.a, b]; }
    wire red [a, b];
    l: Lamp;
}
view schematic "V" {
    include l at (0 0);
    include l at (1, 1) ports { west a; east: a; }
    enclosure { a at (west 1); b at (west, 2); }
    grid: 10;
}
"#;

    #[test]
    fn every_broken_statement_reports_its_own_error() {
        insta::assert_snapshot!(errors_at(BROKEN_EVERYWHERE).join("\n"), @r#"
        ;: found ';' expected something else
        "x": found '"x"' expected ':'
        ;: found ';' expected something else
        ;: found ';' expected something else
        ;: found ';' expected something else, or ']'
        Plug: found 'Plug' expected ':'
        ;: found ';' expected something else
        [: found '[' expected '/', or something else
        0: found '0' expected ','
        a: found 'a' expected ':'
        1: found '1' expected ','
        "#);
    }

    #[test]
    fn a_semicolon_too_early_reports_the_statement_once() {
        // The `;` ends the skip, and what follows it is the rest of the
        // broken statement, not a statement of its own.
        for body in [
            r#"pub port ; c0 "C0" pin 1;"#,
            r#"pub port c0 ; "C0" pin 1;"#,
            "wire red 1 [a.b; c.d];",
            "wire red 1 ; [a.b, c.d];",
        ] {
            let src = format!("component C {{ {body} pub port ok \"OK\"; }}");
            assert_eq!(errors_at(&src).len(), 1, "{body}: {:?}", errors_at(&src));
        }
    }

    #[test]
    fn a_statement_after_a_truncated_one_reports_its_own_error() {
        // A keyword starts a statement of its own, however the one before
        // it ended; so does a statement after one that broke before its `;`.
        let report = [
            r#"component C { pub port a "A" pin; wire "x"; }"#,
            r#"component C { pub port a "A" pin; pin 1; }"#,
            r#"component C { pub port a pin 1; b Lamp; }"#,
        ]
        .iter()
        .map(|src| errors_at(src).join(" | "))
        .collect::<Vec<_>>();
        insta::assert_snapshot!(report.join("\n"), @r#"
        ;: found ';' expected something else | "x": found '"x"' expected something else
        ;: found ';' expected something else | pin: found 'pin' expected 'pub', 'port', 'connector', 'inline', 'wire', 'cable', 'component', 'extend', something else, or '}'
        pin: found 'pin' expected something else | Lamp: found 'Lamp' expected ':'
        "#);
    }

    #[test]
    fn reserved_words_outside_a_skipped_statement_are_still_reported() {
        // The skipped statement's own reserved word goes with it (the
        // parser rewinds out of the attempt); the words of the statements
        // around it, read before and after the skip, all stand.
        let report = errors_at(
            r#"component C {
                pub port view "V";
                pub port wire "W" pin;
                pub port cable "C";
            }"#,
        );
        assert_eq!(
            report,
            [
                "view: reserved word `view`",
                ";: found ';' expected something else",
                "cable: reserved word `cable`",
            ]
        );
    }

    #[test]
    fn a_reserved_word_alone_leaves_the_file_whole() {
        let parsed = parse_str(r#"component C { pub port wire "W"; }"#);
        assert_eq!(parsed.completeness, Completeness::Whole);
        assert_eq!(
            parse_str("component C { }").completeness,
            Completeness::Whole
        );
    }

    #[test]
    fn unbalanced_braces_report_the_first_error_alone() {
        // A missing or extra `}` moves every statement after it into
        // another block, where each would fail: only the first error is
        // reported, and the file is marked misnested.
        for src in [
            r#"component C { cable k { wire red 1 [a.b, c.d]; } pub port a "A"; "#,
            r#"component C { } pub port a "A"; pub port b "B"; }"#,
            r#"component C { pub port a "A" pin; cable k { wire red 1 [a.b, c.d]; } "#,
        ] {
            let parsed = parse_str(src);
            assert_eq!(parsed.completeness, Completeness::Misnested, "{src}");
            assert_eq!(parsed.errors.len(), 1, "{src}: {:?}", parsed.errors);
        }
    }

    #[test]
    fn a_block_left_open_keeps_the_first_error_inside_it() {
        // The end of input closes the open block, so the error reported is
        // the broken statement inside it, as before recovery, not the
        // missing `}`.
        assert_eq!(
            errors_at(r#"component C { pub port a "A" pin; pub port b "B";"#),
            [";: found ';' expected something else"]
        );
    }

    #[test]
    fn a_skipped_statement_leaves_the_rest_of_its_block_in_the_ast() {
        let parsed = parse_str(BROKEN_EVERYWHERE);
        assert_eq!(parsed.completeness, Completeness::Holes);
        let file = parsed.file.expect("a file");
        assert!(file.uses.is_empty());
        let Item::ConnectorType(plug) = &file.items[0] else {
            panic!("expected the connector type");
        };
        assert_eq!(plug.properties.len(), 1, "cavities: 2 stays");
        let Item::Definition(component) = &file.items[1] else {
            panic!("expected the component");
        };
        let names: Vec<String> = component
            .members
            .iter()
            .map(|m| match m {
                Member::Port(p) => format!("port {}", p.name.node),
                Member::ConnectorInstance(c) => format!("{} {}", c.name.node, c.ports.len()),
                Member::Connector(c) => format!("{} {}", c.name.node, c.ports.len()),
                Member::Inline(i) => format!("{} {}", i.name.node, i.ports.len() + i.halves.len()),
                Member::Cable(c) => {
                    format!("{} {}", c.name.node, c.properties.len() + c.members.len())
                }
                Member::Instance(i) => format!("{}: {}", i.name.node, i.type_name.node),
                other => panic!("unexpected member {other:?}"),
            })
            .collect();
        assert_eq!(names, ["port b", "x1 1", "grp 1", "ic 1", "k 1", "l: Lamp"]);
        let Item::View(view) = &file.items[2] else {
            panic!("expected the view");
        };
        assert_eq!(view.includes.len(), 1);
        assert_eq!(view.includes[0].ports.len(), 1, "east: a stays");
        assert_eq!(view.enclosure.len(), 1);
        assert!(view.grid.is_some());
    }

    #[test]
    fn parses_every_fixture_file() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/basic_project");
        let mut count = 0;
        for path in super::tests::walk_wb(std::path::Path::new(root)) {
            let src = std::fs::read_to_string(&path).expect("read fixture");
            let parsed = parse_str(&src);
            assert!(
                parsed.errors.is_empty() && parsed.file.is_some(),
                "parse {}: {:?}",
                path.display(),
                parsed.errors
            );
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

    #[test]
    fn fixture_ast_snapshot() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/components/battery.wb"
        );
        let src = std::fs::read_to_string(path).expect("read fixture");
        let file = parse_ok(&src);
        insta::assert_debug_snapshot!(file);
    }
}
