//! `textDocument/formatting`: the open buffer laid out by the same
//! formatter as `wirebug fmt`, as one edit over the whole document. The
//! client's formatting options (tab size, spaces) are not consulted: the
//! house style is the only one.

use lsp_types::{Position, Range, TextEdit};

use super::line_index::LineIndex;
use crate::dsl::format::Formatted;

/// The edits that format `text`: none when it is formatted already, or
/// when it does not parse (formatting a broken buffer would move the
/// mistake), else one replacing the whole document.
pub(crate) fn edits(text: &str) -> Vec<TextEdit> {
    let Ok(formatted) = Formatted::new(text) else {
        return Vec::new();
    };
    if !formatted.changes(text) {
        return Vec::new();
    }
    let end = LineIndex::new(text).position(text, text.len());
    vec![TextEdit {
        range: Range {
            start: Position::new(0, 0),
            end,
        },
        new_text: formatted.into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unformatted_buffer_is_replaced_whole() {
        let text = "component A {\npub port a \"A\";\n}";
        let edits = edits(text);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(0, 0));
        assert_eq!(edits[0].range.end, Position::new(2, 1));
        assert_eq!(
            edits[0].new_text,
            "component A {\n    pub port a \"A\";\n}\n"
        );
    }

    #[test]
    fn a_formatted_or_broken_buffer_gets_no_edits() {
        assert!(edits("component A {\n    pub port a \"A\";\n}\n").is_empty());
        assert!(edits("component A {\npub port a \"A\"\n}").is_empty());
    }
}
