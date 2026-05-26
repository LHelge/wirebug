//! Source spans: byte ranges tagged with the file they belong to, plus
//! [`Spanned<T>`] for attaching a span to any node.
//!
//! Spans are byte offsets so they map directly onto miette's
//! [`SourceSpan`](miette::SourceSpan) (offset + length) for diagnostics.

use std::fmt;

/// Identifies one source file within a project — an index into the
/// project's file list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub usize);

/// A byte range `[start, end)` within a single source [`file`](Span::file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub file: FileId,
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(file: FileId, start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "span start must not exceed end");
        Self { file, start, end }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Smallest span covering both `self` and `other`. They are assumed
    /// to share a file (asserted in debug builds); the result takes
    /// `self`'s file.
    #[must_use]
    pub fn to(self, other: Span) -> Span {
        debug_assert_eq!(self.file, other.file, "cannot merge spans across files");
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl From<Span> for miette::SourceSpan {
    /// Drops the file association — the caller pairs the span with the
    /// right `NamedSource` when building a diagnostic.
    fn from(span: Span) -> Self {
        (span.start, span.len()).into()
    }
}

/// A value paired with its source [`Span`]. The workhorse for AST nodes,
/// which carry a span on every node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    /// Transform the wrapped value, keeping the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    /// Borrow the wrapped value with the same span.
    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }
}

/// Compact `Debug`: `node @ start..end` — keeps AST dumps readable.
impl<T: fmt::Debug> fmt::Debug for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} @ {}..{}",
            self.node, self.span.start, self.span.end
        )
    }
}
