//! Translation between LSP positions and byte offsets.
//!
//! LSP counts characters in UTF-16 code units by default; `ra_ap_syntax` works in
//! UTF-8 byte offsets. For ASCII source the two coincide, which is exactly why
//! getting this wrong stays invisible until someone puts an emoji in a string
//! literal and every range silently shifts.

use ra_ap_syntax::{TextRange, TextSize};
use tower_lsp_server::ls_types::{Position, Range};

pub struct LineIndex {
    /// Byte offset at which each line starts.
    line_starts: Vec<usize>,
    text: String,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { line_starts, text: text.to_owned() }
    }

    fn line_text(&self, line: usize) -> Option<&str> {
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text.get(start..end)
    }

    /// LSP position -> byte offset. Clamps past-the-end characters to the line
    /// end rather than failing, matching how editors send a cursor at EOL.
    pub fn offset(&self, pos: Position) -> Option<TextSize> {
        let line = pos.line as usize;
        let start = *self.line_starts.get(line)?;
        let line_text = self.line_text(line)?;

        let mut utf16 = 0u32;
        for (byte_off, ch) in line_text.char_indices() {
            if utf16 >= pos.character {
                return Some(TextSize::new((start + byte_off) as u32));
            }
            utf16 += ch.len_utf16() as u32;
        }
        Some(TextSize::new((start + line_text.len()) as u32))
    }

    /// Byte offset -> LSP position.
    pub fn position(&self, offset: TextSize) -> Position {
        let offset = usize::from(offset).min(self.text.len());
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let line_start = self.line_starts[line];
        let character = self.text[line_start..offset]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        Position { line: line as u32, character }
    }

    pub fn range(&self, range: TextRange) -> Range {
        Range { start: self.position(range.start()), end: self.position(range.end()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn ascii_roundtrip() {
        // Line 1 starts at byte 12; character 8 on it is the `x`.
        let idx = LineIndex::new("fn main() {\n    let x = 1;\n}\n");
        let off = idx.offset(pos(1, 8)).unwrap();
        assert_eq!(u32::from(off), 20);
        assert_eq!(idx.position(off), pos(1, 8));
    }

    #[test]
    fn utf16_units_not_bytes() {
        // "é" is two UTF-8 bytes but one UTF-16 unit; "𝄞" is four bytes, two units.
        let idx = LineIndex::new("let s = \"é𝄞\";\n");
        let after_e = idx.offset(pos(0, 10)).unwrap();
        assert_eq!(u32::from(after_e), 11, "one UTF-16 unit past a 2-byte char");
        assert_eq!(idx.position(after_e), pos(0, 10));
    }

    #[test]
    fn past_end_of_line_clamps() {
        let idx = LineIndex::new("ab\ncd\n");
        assert_eq!(u32::from(idx.offset(pos(0, 99)).unwrap()), 3);
    }
}
