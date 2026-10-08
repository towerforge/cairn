//! Single-line text field (prompt, picker filters, forms).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

#[derive(Default, Clone)]
pub struct LineInput {
    pub text: String,
    /// Cursor position in chars (not bytes).
    cursor: usize,
}

impl LineInput {
    pub fn with(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = text.chars().count();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    /// Cursor position in chars.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Replaces chars `start..end` with `s` and leaves the cursor after it.
    pub fn replace_chars(&mut self, start: usize, end: usize, s: &str) {
        let (a, b) = (self.byte_at(start), self.byte_at(end));
        self.text.replace_range(a..b, s);
        self.cursor = start + s.chars().count();
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert(c);
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_at(self.cursor);
            self.text.remove(at);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let at = self.byte_at(self.cursor);
            self.text.remove(at);
        }
    }

    /// Ctrl+W: deletes the word before the cursor.
    fn delete_word(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let (a, b) = (self.byte_at(start), self.byte_at(self.cursor));
        self.text.replace_range(a..b, "");
        self.cursor = start;
    }

    /// Handles an editing key. Returns `false` if it doesn't consume it.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let len = self.text.chars().count();
        match key.code {
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = len,
            KeyCode::Char('u') if ctrl => {
                let at = self.byte_at(self.cursor);
                self.text.replace_range(..at, "");
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => {
                let at = self.byte_at(self.cursor);
                self.text.truncate(at);
            }
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Char(_) if ctrl => return false,
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            _ => return false,
        }
        true
    }

    /// Visible slice for a given width and the cursor column within it.
    /// Scrolls horizontally so the cursor always stays in view.
    pub fn view(&self, width: usize) -> (String, u16) {
        let chars: Vec<char> = self.text.chars().collect();
        let before: String = chars[..self.cursor].iter().collect();
        let mut start = 0;
        while start < self.cursor
            && UnicodeWidthStr::width(&before[self.byte_at(start)..]) >= width.max(1)
        {
            start += 1;
        }
        let visible: String = chars[start..].iter().collect();
        let cursor_x = UnicodeWidthStr::width(&before[self.byte_at(start)..]) as u16;
        (visible, cursor_x)
    }
}
