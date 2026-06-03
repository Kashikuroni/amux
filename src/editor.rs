//! Reusable multi-line text buffer with a character-indexed cursor. Newlines are
//! stored literally in `buffer`. Shared by the reply composer and the note editor.

#[derive(Debug, Clone, Default)]
pub struct TextArea {
    pub buffer: String,
    /// Cursor as a character index into `buffer` (not a byte offset).
    pub cursor: usize,
}

impl TextArea {
    pub fn new(initial: impl Into<String>) -> Self {
        let buffer = initial.into();
        let cursor = buffer.chars().count();
        Self { buffer, cursor }
    }

    pub fn char_count(&self) -> usize {
        self.buffer.chars().count()
    }

    /// Byte offset of character `idx` (or end of buffer if out of range).
    fn byte_at(&self, idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    pub fn insert_char(&mut self, c: char) {
        let b = self.byte_at(self.cursor);
        self.buffer.insert(b, c);
        self.cursor += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        let b = self.byte_at(self.cursor);
        self.buffer.insert_str(b, s);
        self.cursor += s.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let b = self.byte_at(self.cursor - 1);
        self.buffer.remove(b);
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let b = self.byte_at(self.cursor);
        self.buffer.remove(b);
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// Start/end character index of the logical line the cursor sits on.
    pub(crate) fn line_bounds(&self) -> (usize, usize) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut start = self.cursor.min(chars.len());
        while start > 0 && chars[start - 1] != '\n' {
            start -= 1;
        }
        let mut end = self.cursor.min(chars.len());
        while end < chars.len() && chars[end] != '\n' {
            end += 1;
        }
        (start, end)
    }

    pub fn home(&mut self) {
        self.cursor = self.line_bounds().0;
    }

    pub fn end(&mut self) {
        self.cursor = self.line_bounds().1;
    }

    /// Move up one logical line, preserving the column where possible.
    pub fn up(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let (start, _) = self.line_bounds();
        if start == 0 {
            self.cursor = 0;
            return;
        }
        let col = self.cursor - start;
        let prev_end = start - 1; // the '\n'
        let mut prev_start = prev_end;
        while prev_start > 0 && chars[prev_start - 1] != '\n' {
            prev_start -= 1;
        }
        let prev_len = prev_end - prev_start;
        self.cursor = prev_start + col.min(prev_len);
    }

    /// Move down one logical line, preserving the column where possible.
    pub fn down(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let (start, end) = self.line_bounds();
        if end >= chars.len() {
            self.cursor = chars.len();
            return;
        }
        let col = self.cursor - start;
        let next_start = end + 1;
        let mut next_end = next_start;
        while next_end < chars.len() && chars[next_end] != '\n' {
            next_end += 1;
        }
        let next_len = next_end - next_start;
        self.cursor = next_start + col.min(next_len);
    }

    /// Delete the word (and any trailing spaces) before the cursor (Ctrl+W).
    pub fn delete_word(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && chars[i - 1] != ' ' && chars[i - 1] != '\n' {
            i -= 1;
        }
        let (sb, eb) = (self.byte_at(i), self.byte_at(self.cursor));
        self.buffer.replace_range(sb..eb, "");
        self.cursor = i;
    }

    /// Delete from the start of the current line to the cursor (Ctrl+U).
    pub fn delete_to_line_start(&mut self) {
        let (start, _) = self.line_bounds();
        let (sb, eb) = (self.byte_at(start), self.byte_at(self.cursor));
        self.buffer.replace_range(sb..eb, "");
        self.cursor = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_track_cursor() {
        let mut a = TextArea::default();
        a.insert_char('h');
        a.insert_char('i');
        assert_eq!(a.buffer, "hi");
        assert_eq!(a.cursor, 2);
        a.backspace();
        assert_eq!(a.buffer, "h");
        assert_eq!(a.cursor, 1);
    }

    #[test]
    fn newline_then_up_down_preserve_column() {
        let mut a = TextArea::new("abc");
        a.insert_char('\n');
        a.insert_str("de");
        assert_eq!(a.buffer, "abc\nde");
        a.up();
        a.down();
        assert_eq!(a.buffer, "abc\nde");
    }

    #[test]
    fn left_right_step_over_multibyte_chars() {
        // Cyrillic letters are 2 bytes each; the cursor is a CHAR index, so
        // motion + editing must never split a UTF-8 boundary.
        let mut a = TextArea::new("абв"); // cursor at end (3 chars)
        assert_eq!(a.cursor, 3);
        a.left();
        a.left();
        assert_eq!(a.cursor, 1); // before 'б'
        a.insert_char('Я');
        assert_eq!(a.buffer, "аЯбв");
        assert_eq!(a.cursor, 2);
    }

    #[test]
    fn backspace_and_delete_remove_whole_multibyte_chars() {
        let mut a = TextArea::new("аб");
        a.backspace(); // removes 'б'
        assert_eq!(a.buffer, "а");
        assert_eq!(a.cursor, 1);
        a.home();
        a.delete(); // removes 'а' (forward)
        assert_eq!(a.buffer, "");
        assert_eq!(a.cursor, 0);
    }

    #[test]
    fn delete_word_and_to_line_start_are_char_indexed() {
        let mut a = TextArea::new("привет мир"); // cursor at end
        a.delete_word(); // removes "мир"
        assert_eq!(a.buffer, "привет ");
        a.delete_to_line_start(); // clears the whole (single) line up to cursor
        assert_eq!(a.buffer, "");
        assert_eq!(a.cursor, 0);
    }
}
