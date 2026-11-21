//! Laying the statements out: four spaces a block deep, one statement a
//! line, and the statements of a run of one kind aligned in columns.

use super::tree::{Body, Statement};
use crate::dsl::lex::Token;
use crate::dsl::lex::trivia::{Attached, AttachedTokens, Leading};

const INDENT: &str = "    ";

/// The text being laid out.
pub(super) struct Printer {
    out: String,
}

impl Printer {
    /// Lay out a whole file.
    pub(super) fn print(attached: &AttachedTokens<'_>) -> String {
        let mut printer = Printer { out: String::new() };
        printer.body(&Body::file(&attached.tokens), &attached.end, 0);
        printer.out
    }

    /// The statements of a block at `depth`, then `end`: the comments
    /// before its `}` (or the end of the file).
    fn body(&mut self, body: &Body<'_, '_>, end: &[Leading<'_>], depth: usize) {
        let rows: Vec<Option<Row<'_>>> = body.statements.iter().map(Row::of).collect();
        let mut started = false;
        let mut at = 0;
        while at < body.statements.len() {
            let run = match &rows[at] {
                Some(first) => {
                    let len = rows[at + 1..]
                        .iter()
                        .zip(&body.statements[at + 1..])
                        .take_while(|(row, statement)| {
                            row.as_ref().is_some_and(|row| row.kind == first.kind)
                                && !statement.first().leading.contains(&Leading::BlankLine)
                        })
                        .count();
                    let rows: Vec<&Row<'_>> = rows[at..=at + len].iter().flatten().collect();
                    Row::align(&rows)
                }
                None => Vec::new(),
            };
            let count = run.len().max(1);
            for (offset, statement) in body.statements[at..at + count].iter().enumerate() {
                self.leading(&statement.first().leading, depth, &mut started, false);
                match run.get(offset) {
                    Some(line) => self.line(depth, line),
                    None => self.statement(statement, depth),
                }
                started = true;
            }
            at += count;
        }
        self.leading(end, depth, &mut started, true);
    }

    /// The own-line comments and blank lines before a statement (or, with
    /// `closing`, before the end of its block). A blank line is kept only
    /// between two things of the block: never first, and never last.
    fn leading(&mut self, items: &[Leading<'_>], depth: usize, started: &mut bool, closing: bool) {
        for (at, item) in items.iter().enumerate() {
            match item {
                Leading::BlankLine => {
                    let more = !closing
                        || items[at + 1..]
                            .iter()
                            .any(|item| matches!(item, Leading::Comment(_)));
                    if *started && more {
                        self.out.push('\n');
                    }
                }
                Leading::Comment(comment) => {
                    self.line(depth, comment);
                    *started = true;
                }
            }
        }
    }

    /// A statement that is not part of an aligned run: a block, a statement
    /// with a comment inside it, or one with a list split over lines.
    fn statement(&mut self, statement: &Statement<'_, '_>, depth: usize) {
        if let Some(tokens) = statement.one_line() {
            self.tokens(tokens, depth);
            return;
        }
        match statement {
            Statement::Simple(tokens) => self.tokens(tokens, depth),
            Statement::Block { header, body, .. } => {
                let open = &header[header.len() - 1];
                let empty = body.statements.is_empty()
                    && open.trailing.is_none()
                    && body
                        .close
                        .is_none_or(|close| !close.leading.contains_comment());
                match body.close.filter(|_| empty) {
                    Some(close) => {
                        self.tokens_then(header, depth);
                        self.out.push_str(close.text);
                        self.trailing(close.trailing);
                        self.out.push('\n');
                    }
                    None => {
                        self.tokens(header, depth);
                        if let Some(close) = body.close {
                            self.body(body, &close.leading, depth + 1);
                            self.indent(depth);
                            self.out.push_str(close.text);
                            self.trailing(close.trailing);
                            self.out.push('\n');
                        }
                    }
                }
            }
        }
    }

    /// `tokens` as one statement starting a line at `depth`, with the last
    /// token's comment and the line's end.
    fn tokens(&mut self, tokens: &[Attached<'_>], depth: usize) {
        self.tokens_then(tokens, depth);
        self.out.push('\n');
    }

    /// `tokens` from the start of a line at `depth`, single-spaced, and the
    /// last one's comment; the line is left open. A comment inside them
    /// ends its line, and the statement goes on a level deeper on the next;
    /// a `[ … ]` list the author split over lines is written one item a
    /// line ([`split_list`](Self::split_list)).
    fn tokens_then(&mut self, tokens: &[Attached<'_>], depth: usize) {
        self.indent(depth);
        let mut pen = Pen {
            fresh: false,
            prev: None,
        };
        let mut at = 0;
        while let Some(token) = tokens.get(at) {
            let split = (token.token == Token::LBracket)
                .then(|| split_close(tokens, at))
                .flatten();
            let last = match split {
                Some(close) => {
                    self.split_list(&tokens[at..=close], depth, &mut pen);
                    at = close;
                    &tokens[close]
                }
                None if dangling(tokens, at) => {
                    at += 1;
                    continue;
                }
                None => {
                    if at > 0 {
                        self.comments_before(token, depth + 1, &mut pen);
                    }
                    self.text(token, depth + 1, &mut pen);
                    token
                }
            };
            at += 1;
            if at == tokens.len() {
                self.trailing(last.trailing);
            } else {
                self.break_after(last, &mut pen);
            }
        }
    }

    /// A `[ … ]` list the author split over lines: the `[` ends its line,
    /// each item takes a line of its own a level deeper than the statement,
    /// with a comma after it (the last one's added), and the `]` starts the
    /// statement's last line. The `]`'s own comment is left to the caller.
    fn split_list<'t>(&mut self, list: &'t [Attached<'_>], depth: usize, pen: &mut Pen<'t>) {
        let (open, rest) = list
            .split_first()
            .map_or((None, list), |(o, r)| (Some(o), r));
        let (close, inner) = rest
            .split_last()
            .map_or((None, rest), |(c, i)| (Some(c), i));
        let item_depth = depth + 1;
        if let Some(open) = open {
            self.comments_before(open, item_depth, pen);
            self.text(open, item_depth, pen);
            self.trailing(open.trailing);
            self.out.push('\n');
            pen.fresh = true;
        }
        for (item, comma) in Items::new(inner) {
            let Some((last, init)) = item.split_last() else {
                continue;
            };
            for (at, token) in init.iter().enumerate() {
                let line_depth = if at == 0 { item_depth } else { item_depth + 1 };
                self.comments_before(token, line_depth, pen);
                self.text(token, line_depth, pen);
                self.break_after(token, pen);
            }
            let line_depth = if init.is_empty() {
                item_depth
            } else {
                item_depth + 1
            };
            self.comments_before(last, line_depth, pen);
            self.text(last, line_depth, pen);
            // The comma goes right after the item, then the comments around
            // it in source order: the item's own, those on lines before the
            // comma, the comma's.
            self.out.push(',');
            let before_comma = comma.into_iter().flat_map(|comma| {
                comma.leading.iter().filter_map(|item| match item {
                    Leading::Comment(comment) => Some(*comment),
                    Leading::BlankLine => None,
                })
            });
            let mut comments = last
                .trailing
                .into_iter()
                .chain(before_comma)
                .chain(comma.and_then(|comma| comma.trailing));
            self.trailing(comments.next());
            self.out.push('\n');
            for comment in comments {
                self.line(item_depth, comment);
            }
            pen.fresh = true;
        }
        if let Some(close) = close {
            for item in &close.leading {
                if let Leading::Comment(comment) = item {
                    self.line(item_depth, comment);
                }
            }
            self.text(close, depth, pen);
        }
    }

    /// The own-line comments before a token inside a statement: they end
    /// the line, each takes one at `depth`, and the token starts the next.
    fn comments_before(&mut self, token: &Attached<'_>, depth: usize, pen: &mut Pen<'_>) {
        if !token.leading.contains_comment() {
            return;
        }
        if !pen.fresh {
            self.out.push('\n');
        }
        for item in &token.leading {
            if let Leading::Comment(comment) = item {
                self.line(depth, comment);
            }
        }
        pen.fresh = true;
    }

    /// A token's text: at `depth` on a fresh line, else after a space
    /// where one goes.
    fn text<'t>(&mut self, token: &'t Attached<'_>, depth: usize, pen: &mut Pen<'t>) {
        if pen.fresh {
            self.indent(depth);
            pen.fresh = false;
        } else if pen.prev.is_some_and(|prev| spaced(prev, &token.token)) {
            self.out.push(' ');
        }
        self.out.push_str(token.text);
        pen.prev = Some(&token.token);
    }

    /// A token's comment inside a statement, which ends the line there.
    fn break_after(&mut self, token: &Attached<'_>, pen: &mut Pen<'_>) {
        if let Some(comment) = token.trailing {
            self.trailing(Some(comment));
            self.out.push('\n');
            pen.fresh = true;
        }
    }

    fn trailing(&mut self, comment: Option<&str>) {
        if let Some(comment) = comment {
            self.out.push(' ');
            self.out.push_str(comment);
        }
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
    }

    fn line(&mut self, depth: usize, text: &str) {
        self.indent(depth);
        self.out.push_str(text);
        self.out.push('\n');
    }
}

trait ContainsComment {
    fn contains_comment(&self) -> bool;
}

impl ContainsComment for [Leading<'_>] {
    fn contains_comment(&self) -> bool {
        self.iter().any(|item| matches!(item, Leading::Comment(_)))
    }
}

/// Whether a space goes between two tokens of a line: none before
/// `;`, `,`, `:`, `.`, `/` and a closing bracket, nor after an opening
/// bracket, `.` and `/`.
fn spaced(prev: &Token, next: &Token) -> bool {
    !matches!(
        next,
        Token::Semicolon
            | Token::Comma
            | Token::Colon
            | Token::Dot
            | Token::Slash
            | Token::RParen
            | Token::RBracket
    ) && !matches!(
        prev,
        Token::LParen | Token::LBracket | Token::Dot | Token::Slash
    )
}

/// The tokens a statement is written with, each with its index: all of
/// them but a comma right before a closing bracket (`[a, b,]`), which the
/// list reads the same without, unless a comment sits on it.
struct Kept<'t, 'src> {
    tokens: &'t [Attached<'src>],
    at: usize,
}

impl<'t, 'src> Kept<'t, 'src> {
    fn new(tokens: &'t [Attached<'src>]) -> Self {
        Kept { tokens, at: 0 }
    }
}

impl<'t, 'src> Iterator for Kept<'t, 'src> {
    type Item = (usize, &'t Attached<'src>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let at = self.at;
            let token = self.tokens.get(at)?;
            self.at += 1;
            if !dangling(self.tokens, at) {
                return Some((at, token));
            }
        }
    }
}

/// Whether the token at `at` is a comma right before a closing bracket
/// with no comment on it, which a list on one line is written without.
fn dangling(tokens: &[Attached<'_>], at: usize) -> bool {
    tokens.get(at).is_some_and(|token| {
        token.token == Token::Comma
            && token.trailing.is_none()
            && !token.leading.contains_comment()
            && tokens
                .get(at + 1)
                .is_some_and(|next| matches!(next.token, Token::RBracket | Token::RParen))
    })
}

/// The index of the `]` closing the `[` at `open` when the author broke a
/// line anywhere inside the list; `None` for a list written on one line.
fn split_close(tokens: &[Attached<'_>], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (at, token) in tokens.iter().enumerate().skip(open) {
        match token.token {
            Token::LBracket => depth += 1,
            Token::RBracket => {
                depth -= 1;
                if depth == 0 {
                    return tokens[open + 1..=at]
                        .iter()
                        .any(|token| token.newline_before)
                        .then_some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the writing of a statement stands.
struct Pen<'t> {
    /// On a fresh line, its indent not yet written.
    fresh: bool,
    /// The token written last, which decides the space before the next.
    prev: Option<&'t Token>,
}

/// The items of a list's inside, each with the comma after it (none after
/// the last, unless the author wrote one).
struct Items<'t, 'src> {
    tokens: &'t [Attached<'src>],
}

impl<'t, 'src> Items<'t, 'src> {
    fn new(tokens: &'t [Attached<'src>]) -> Self {
        Items { tokens }
    }
}

impl<'t, 'src> Iterator for Items<'t, 'src> {
    type Item = (&'t [Attached<'src>], Option<&'t Attached<'src>>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.tokens.is_empty() {
            return None;
        }
        let mut depth = 0usize;
        let comma = self.tokens.iter().position(|token| {
            match token.token {
                Token::LBracket | Token::LParen => depth += 1,
                Token::RBracket | Token::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            depth == 0 && token.token == Token::Comma
        });
        let tokens = self.tokens;
        Some(match comma {
            Some(at) => {
                self.tokens = &tokens[at + 1..];
                (&tokens[..at], Some(&tokens[at]))
            }
            None => {
                self.tokens = &[];
                (tokens, None)
            }
        })
    }
}

/// Which statements align with each other, column by column: those of one
/// kind, and only those whose columns mean the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowKind<'src> {
    /// `use <Name> from "<path>";` — `from` aligned.
    Use,
    /// `[pub] port <name> "<label>" [pin …];` — label and pins aligned;
    /// public and private ports apart.
    Port { public: bool },
    /// `<name>: <value>;` — an instance, part data, a property, a side's
    /// ports, an inline half: what follows the colon aligned, and an
    /// instance's label after its type.
    Keyed,
    /// `wire <color> <gauge> ["<label>"] [<end>, …];` — gauge, label, the
    /// endpoint list and each endpoint in it aligned; labelled and
    /// unlabelled wires apart.
    Wire { labelled: bool },
    /// `include <instance> at (…);`, `<port> at (…);` — `at` aligned.
    Placed,
    /// Any other statement, by its first word: nothing to align but the
    /// comments that end the lines.
    Other(&'src str),
}

/// A statement that fits on one line, cut into the columns its kind
/// aligns.
#[derive(Debug)]
struct Row<'src> {
    kind: RowKind<'src>,
    cells: Vec<String>,
    comment: Option<&'src str>,
}

impl<'src> Row<'src> {
    /// The row of a statement that prints on one line
    /// ([`Statement::one_line`]) and holds no comment but the one ending it
    /// and no list split over lines; `None` for any other statement.
    fn of(statement: &Statement<'_, 'src>) -> Option<Self> {
        let tokens = statement.one_line()?;
        let (last, inner) = tokens.split_last()?;
        if tokens[1..].iter().any(|t| t.leading.contains_comment())
            || inner.iter().any(|t| t.trailing.is_some())
            || (0..tokens.len())
                .any(|at| tokens[at].token == Token::LBracket && split_close(tokens, at).is_some())
        {
            return None;
        }
        let (kind, starts) = RowKind::columns(tokens);
        let mut cells = vec![String::new()];
        let mut prev: Option<&Token> = None;
        for (at, token) in Kept::new(tokens) {
            if starts.contains(&at) && cells.last().is_some_and(|cell| !cell.is_empty()) {
                cells.push(String::new());
            }
            let cell = cells.last_mut()?;
            if !cell.is_empty() && prev.is_some_and(|prev| spaced(prev, &token.token)) {
                cell.push(' ');
            }
            cell.push_str(token.text);
            prev = Some(&token.token);
        }
        Some(Row {
            kind,
            cells,
            comment: last.trailing,
        })
    }

    /// The lines of a run of rows: every cell but a row's last padded to
    /// the widest of its column, and the comments that end the lines lined
    /// up after the longest line that has one.
    fn align(rows: &[&Row<'_>]) -> Vec<String> {
        let mut widths: Vec<usize> = Vec::new();
        for row in rows {
            let Some((_, padded)) = row.cells.split_last() else {
                continue;
            };
            for (column, cell) in padded.iter().enumerate() {
                match widths.get_mut(column) {
                    Some(width) => *width = (*width).max(width_of(cell)),
                    None => widths.push(width_of(cell)),
                }
            }
        }
        let lines: Vec<String> = rows
            .iter()
            .map(|row| {
                let mut line = String::new();
                if let Some((last, padded)) = row.cells.split_last() {
                    for (cell, width) in padded.iter().zip(&widths) {
                        pad(&mut line, cell, *width);
                        line.push(' ');
                    }
                    line.push_str(last);
                }
                line
            })
            .collect();
        let comment_at = rows
            .iter()
            .zip(&lines)
            .filter(|(row, _)| row.comment.is_some())
            .map(|(_, line)| width_of(line))
            .max()
            .unwrap_or(0);
        rows.iter()
            .zip(lines)
            .map(|(row, line)| match row.comment {
                Some(comment) => {
                    let mut padded = String::new();
                    pad(&mut padded, &line, comment_at);
                    format!("{padded} {comment}")
                }
                None => line,
            })
            .collect()
    }
}

impl<'src> RowKind<'src> {
    /// The kind of a one-line statement, and the indices of the tokens that
    /// start each of its columns after the first.
    fn columns(tokens: &[Attached<'src>]) -> (Self, Vec<usize>) {
        let token = |at: usize| tokens.get(at).map(|t| &t.token);
        let is_str = |at: usize| matches!(token(at), Some(Token::Str(_)));
        let not_end = |at: usize| token(at).is_some_and(|t| *t != Token::Semicolon);
        match (token(0), token(1)) {
            (Some(Token::Use), _) => {
                let from = tokens.iter().position(|t| t.token == Token::From);
                (RowKind::Use, from.into_iter().collect())
            }
            (Some(Token::Pub), Some(Token::Port)) | (Some(Token::Port), _) => {
                let public = token(0) == Some(&Token::Pub);
                let label = if public { 3 } else { 2 };
                let starts = [label, label + 1]
                    .into_iter()
                    .filter(|&at| (at == label && is_str(at)) || (at > label && not_end(at)))
                    .collect();
                (RowKind::Port { public }, starts)
            }
            (Some(Token::Wire), _) => {
                let gauge = if token(2) == Some(&Token::Slash) {
                    4
                } else {
                    2
                };
                let labelled = is_str(gauge + 1);
                let list = tokens.iter().position(|t| t.token == Token::LBracket);
                let ends = list.into_iter().flat_map(|list| {
                    (list + 1..tokens.len()).filter(move |&at| {
                        token(at - 1) == Some(&Token::Comma)
                            && !matches!(token(at), Some(Token::RBracket))
                    })
                });
                let starts = std::iter::once(gauge)
                    .chain(labelled.then_some(gauge + 1))
                    .chain(list)
                    .chain(ends)
                    .collect();
                (RowKind::Wire { labelled }, starts)
            }
            (Some(Token::Include), _) | (Some(_), Some(Token::At)) => {
                let at = tokens.iter().position(|t| t.token == Token::At);
                (RowKind::Placed, at.into_iter().collect())
            }
            (Some(_), Some(Token::Colon)) => {
                let label = (!is_str(2) && is_str(3)).then_some(3);
                (RowKind::Keyed, std::iter::once(2).chain(label).collect())
            }
            _ => (RowKind::Other(tokens[0].text), Vec::new()),
        }
    }
}

/// How many columns `text` takes: one a character.
fn width_of(text: &str) -> usize {
    text.chars().count()
}

/// `text`, then spaces up to `width` columns.
fn pad(line: &mut String, text: &str, width: usize) {
    line.push_str(text);
    line.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(width_of(text)),
    ));
}
