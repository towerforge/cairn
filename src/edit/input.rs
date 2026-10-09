//! Single-line text field (prompt, picker filters, forms).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

#[derive(Default, Clone)]
pub struct LineInput {
    pub text: String,
    /// Cursor position in chars (not bytes).
    cursor: usize,
    /// Fixed end of the selection, in chars, while there is one; the cursor
    /// is the moving end.
    anchor: Option<usize>,
}

impl LineInput {
    pub fn with(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
            anchor: None,
        }
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = text.chars().count();
        self.anchor = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        self.anchor = None;
        std::mem::take(&mut self.text)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Cursor position in chars.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Moves the cursor to char `at` (clamped to the text); any selection
    /// is dropped.
    pub fn set_cursor(&mut self, at: usize) {
        self.cursor = at.min(self.len());
        self.anchor = None;
    }

    /// Selects from `anchor` to `head` (chars); the cursor ends at `head`.
    pub fn select(&mut self, anchor: usize, head: usize) {
        let len = self.len();
        self.anchor = Some(anchor.min(len));
        self.cursor = head.min(len);
    }

    /// Moves the selection's end to `head`, starting one at the cursor if
    /// there was none.
    pub fn extend_to(&mut self, head: usize) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        self.select(anchor, head);
    }

    /// Selected chars `start..end`, if the selection is not empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection()?;
        Some(self.text.chars().skip(a).take(b - a).collect())
    }

    /// Deletes the selection, leaving the cursor where it started.
    /// Returns `false` if there was none.
    pub fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        let (x, y) = (self.byte_at(a), self.byte_at(b));
        self.text.replace_range(x..y, "");
        self.cursor = a;
        self.anchor = None;
        true
    }

    /// Replaces chars `start..end` with `s` and leaves the cursor after it.
    pub fn replace_chars(&mut self, start: usize, end: usize, s: &str) {
        let (a, b) = (self.byte_at(start), self.byte_at(end));
        self.text.replace_range(a..b, s);
        self.cursor = start + s.chars().count();
        self.anchor = None;
    }

    /// Inserts at the cursor, replacing the selection if there is one.
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
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
        if self.cursor < self.len() {
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
    ///
    /// With a selection: typing, `Backspace` and `Delete` replace it, a
    /// movement key with `Shift` extends it, and a plain movement key
    /// collapses it to the end the key points at.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End if shift => {
                self.anchor.get_or_insert(self.cursor);
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                if let Some((a, b)) = self.selection() {
                    self.anchor = None;
                    self.cursor = match key.code {
                        KeyCode::Left => a,
                        KeyCode::Right => b,
                        KeyCode::Home => 0,
                        _ => self.len(),
                    };
                    return true;
                }
                self.anchor = None;
            }
            KeyCode::Char(_) if !ctrl => {
                self.delete_selection();
            }
            KeyCode::Backspace | KeyCode::Delete => {
                if self.delete_selection() {
                    return true;
                }
            }
            _ => self.anchor = None,
        }
        let len = self.len();
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

    /// First char shown for a given width: the text scrolls horizontally
    /// so the cursor always stays in view.
    pub fn first_visible(&self, width: usize) -> usize {
        let cursor = self.byte_at(self.cursor);
        let mut start = 0;
        while start < self.cursor
            && UnicodeWidthStr::width(&self.text[self.byte_at(start)..cursor]) >= width.max(1)
        {
            start += 1;
        }
        start
    }

    /// Visible slice for a given width and the cursor column within it.
    pub fn view(&self, width: usize) -> (String, u16) {
        let start = self.byte_at(self.first_visible(width));
        let cursor = self.byte_at(self.cursor);
        let cursor_x = UnicodeWidthStr::width(&self.text[start..cursor]) as u16;
        (self.text[start..].to_string(), cursor_x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn the_cursor_can_be_placed_anywhere_in_the_text() {
        let mut input = LineInput::with("echo hola");
        input.set_cursor(2);
        assert_eq!(input.cursor(), 2);
        input.set_cursor(99);
        assert_eq!(input.cursor(), 9);
        input.set_cursor(0);
        input.insert_str("> ");
        assert_eq!(input.text, "> echo hola");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn scrolls_so_the_cursor_stays_in_view() {
        let mut input = LineInput::with("0123456789");
        assert_eq!(input.first_visible(4), 7);
        assert_eq!(input.view(4), ("789".to_string(), 3));
        input.set_cursor(0);
        assert_eq!(input.first_visible(4), 0);
        assert_eq!(input.view(4), ("0123456789".to_string(), 0));
    }

    #[test]
    fn typing_replaces_the_selection_and_backspace_deletes_it() {
        let mut input = LineInput::with("echo hola");
        input.select(5, 9);
        assert_eq!(input.selected_text().as_deref(), Some("hola"));
        input.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(input.text, "echo x");
        assert_eq!(input.cursor(), 6);
        assert_eq!(input.selection(), None);
        input.select(4, 0);
        input.handle_key(key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.text, " x");
        assert_eq!(input.cursor(), 0);
        input.select(0, 1);
        input.insert_str("ls");
        assert_eq!(input.text, "lsx");
        input.select(0, 3);
        input.handle_key(key(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(input.text, "");
    }

    #[test]
    fn shift_with_a_movement_key_extends_and_without_it_collapses() {
        let mut input = LineInput::with("abcd");
        input.handle_key(key(KeyCode::Left, KeyModifiers::SHIFT));
        input.handle_key(key(KeyCode::Left, KeyModifiers::SHIFT));
        assert_eq!(input.selection(), Some((2, 4)));
        input.handle_key(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.selection(), None);
        assert_eq!(input.cursor(), 2);
        input.handle_key(key(KeyCode::Home, KeyModifiers::SHIFT));
        assert_eq!(input.selection(), Some((0, 2)));
        input.handle_key(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor(), 2);
        // Without a selection, Delete removes the char under the cursor.
        input.handle_key(key(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(input.text, "abd");
        // An empty selection is no selection.
        input.select(1, 1);
        assert_eq!(input.selection(), None);
        input.extend_to(3);
        assert_eq!(input.selected_text().as_deref(), Some("bd"));
    }
}
