//! Byte-offset ↔ LSP position mapping for one source file.
//!
//! [`Span`](crate::dsl::span::Span)s carry byte offsets; LSP positions are
//! a zero-based line plus a UTF-16 code-unit column (the protocol's default
//! encoding). Out-of-range inputs clamp rather than panic — a span at EOF
//! or a client position past the end of a line are both routine.

use lsp_types::Position;

pub(crate) struct LineIndex {
    /// Byte offset of each line start; `line_starts[0] == 0`.
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(src: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            src.char_indices()
                .filter(|&(_, c)| c == '\n')
                .map(|(i, _)| i + 1),
        );
        Self { line_starts }
    }

    /// The position of byte `offset` in `src`, clamped into the text and
    /// floored to a char boundary.
    pub(crate) fn position(&self, src: &str, offset: usize) -> Position {
        let offset = floor_char_boundary(src, offset.min(src.len()));
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let column: usize = src[self.line_starts[line]..offset]
            .chars()
            .map(char::len_utf16)
            .sum();
        Position::new(line as u32, column as u32)
    }
}

fn floor_char_boundary(src: &str, mut i: usize) -> usize {
    while i > 0 && !src.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_of_ascii_offset() {
        let src = "wire red\nwire blue\n";
        let index = LineIndex::new(src);
        assert_eq!(index.position(src, 0), Position::new(0, 0));
        assert_eq!(index.position(src, 5), Position::new(0, 5));
        assert_eq!(index.position(src, 9), Position::new(1, 0));
        assert_eq!(index.position(src, 14), Position::new(1, 5));
    }

    #[test]
    fn utf16_column_counts_code_units() {
        // `å` is one UTF-16 unit but two UTF-8 bytes; `🔌` is two UTF-16
        // units and four UTF-8 bytes.
        let src = "å🔌x";
        let index = LineIndex::new(src);
        assert_eq!(index.position(src, 2), Position::new(0, 1), "after å");
        assert_eq!(index.position(src, 6), Position::new(0, 3), "after 🔌");
    }

    #[test]
    fn crlf_line_ends_do_not_shift_lines() {
        let src = "port a;\r\nport b;\r\n";
        let index = LineIndex::new(src);
        assert_eq!(index.position(src, 9), Position::new(1, 0));
        assert_eq!(index.position(src, 14), Position::new(1, 5));
    }

    #[test]
    fn offset_inside_clamped_span_floors_to_char_boundary() {
        let src = "🔌";
        let index = LineIndex::new(src);
        assert_eq!(index.position(src, 2), Position::new(0, 0));
    }
}
