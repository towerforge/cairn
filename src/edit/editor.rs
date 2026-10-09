//! Simple side editor, with a preview for Markdown files.
//!
//! Opened with `edit <file>` from the prompt. vim/nano really work in Cairn
//! (full-screen mode); this is for quick tweaks without losing sight of the
//! terminal.

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

pub enum EditorAction {
    None,
    /// Return focus to the terminal.
    Unfocus,
    Close,
}

pub struct Editor {
    pub path: PathBuf,
    pub lines: Vec<String>,
    /// Cursor: line and column in chars.
    pub cy: usize,
    pub cx: usize,
    pub top: usize,
    pub left: usize,
    pub dirty: bool,
    pub is_markdown: bool,
    pub preview: bool,
    pub preview_scroll: u16,
    pub status: Option<String>,
    confirm_close: bool,
}

impl Editor {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let content = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.iter().take(8192).any(|b| *b == 0) {
                    return Err(format!("{}: binary file, not editable", path.display()));
                }
                String::from_utf8_lossy(&bytes).into_owned()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let is_markdown = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd"
            )
        });
        let status = (!path.exists()).then(|| "new file".to_string());
        Ok(Self {
            path,
            lines: content
                .split('\n')
                .map(|l| l.trim_end_matches('\r').to_string())
                .collect(),
            cy: 0,
            cx: 0,
            top: 0,
            left: 0,
            dirty: false,
            is_markdown,
            preview: false,
            preview_scroll: 0,
            status,
            confirm_close: false,
        })
    }

    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    fn save(&mut self) {
        match std::fs::write(&self.path, self.content()) {
            Ok(()) => {
                self.dirty = false;
                self.status = Some("saved".into());
            }
            Err(e) => self.status = Some(format!("error saving: {e}")),
        }
    }

    fn line_len(&self, y: usize) -> usize {
        self.lines[y].chars().count()
    }

    fn byte_at(&self, y: usize, x: usize) -> usize {
        let l = &self.lines[y];
        l.char_indices().nth(x).map_or(l.len(), |(i, _)| i)
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '\n' => self.newline(),
                '\r' => {}
                '\t' => self.insert_char(' '),
                c => self.insert_char(c),
            }
        }
    }

    fn insert_char(&mut self, c: char) {
        let at = self.byte_at(self.cy, self.cx);
        self.lines[self.cy].insert(at, c);
        self.cx += 1;
        self.dirty = true;
    }

    fn newline(&mut self) {
        let at = self.byte_at(self.cy, self.cx);
        let rest = self.lines[self.cy].split_off(at);
        // Keep the current line's indentation.
        let indent: String = self.lines[self.cy]
            .chars()
            .take_while(|c| *c == ' ')
            .collect();
        self.cy += 1;
        self.cx = indent.chars().count();
        self.lines.insert(self.cy, indent + &rest);
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
            let at = self.byte_at(self.cy, self.cx);
            self.lines[self.cy].remove(at);
        } else if self.cy > 0 {
            let line = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.line_len(self.cy);
            self.lines[self.cy].push_str(&line);
        } else {
            return;
        }
        self.dirty = true;
    }

    fn delete(&mut self) {
        if self.cx < self.line_len(self.cy) {
            let at = self.byte_at(self.cy, self.cx);
            self.lines[self.cy].remove(at);
        } else if self.cy + 1 < self.lines.len() {
            let next = self.lines.remove(self.cy + 1);
            self.lines[self.cy].push_str(&next);
        } else {
            return;
        }
        self.dirty = true;
    }

    fn clamp_x(&mut self) {
        self.cx = self.cx.min(self.line_len(self.cy));
    }

    pub fn handle_key(&mut self, key: KeyEvent, page: usize) -> EditorAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if !matches!(key.code, KeyCode::Char('x')) || !ctrl {
            self.confirm_close = false;
        }
        match key.code {
            KeyCode::Char('s') if ctrl => self.save(),
            KeyCode::Char('p') if ctrl && self.is_markdown => {
                self.preview = !self.preview;
                self.preview_scroll = 0;
            }
            KeyCode::Char('x') if ctrl => {
                if self.dirty && !self.confirm_close {
                    self.confirm_close = true;
                    self.status = Some("unsaved changes · Ctrl+X again to discard".into());
                } else {
                    return EditorAction::Close;
                }
            }
            KeyCode::Esc => return EditorAction::Unfocus,
            _ if self.preview => match key.code {
                KeyCode::Up => self.preview_scroll = self.preview_scroll.saturating_sub(1),
                KeyCode::Down => self.preview_scroll = self.preview_scroll.saturating_add(1),
                KeyCode::PageUp => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(page as u16)
                }
                KeyCode::PageDown => {
                    self.preview_scroll = self.preview_scroll.saturating_add(page as u16)
                }
                KeyCode::Home => self.preview_scroll = 0,
                _ => {}
            },
            KeyCode::Char('a') if ctrl => self.cx = 0,
            KeyCode::Char('e') if ctrl => self.cx = self.line_len(self.cy),
            KeyCode::Char(_) if ctrl => {}
            KeyCode::Char(c) => self.insert_char(c),
            KeyCode::Enter => self.newline(),
            KeyCode::Tab => self.insert_str("    "),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                } else if self.cy > 0 {
                    self.cy -= 1;
                    self.cx = self.line_len(self.cy);
                }
            }
            KeyCode::Right => {
                if self.cx < self.line_len(self.cy) {
                    self.cx += 1;
                } else if self.cy + 1 < self.lines.len() {
                    self.cy += 1;
                    self.cx = 0;
                }
            }
            KeyCode::Up => {
                self.cy = self.cy.saturating_sub(1);
                self.clamp_x();
            }
            KeyCode::Down => {
                self.cy = (self.cy + 1).min(self.lines.len() - 1);
                self.clamp_x();
            }
            KeyCode::PageUp => {
                self.cy = self.cy.saturating_sub(page);
                self.clamp_x();
            }
            KeyCode::PageDown => {
                self.cy = (self.cy + page).min(self.lines.len() - 1);
                self.clamp_x();
            }
            KeyCode::Home => self.cx = 0,
            KeyCode::End => self.cx = self.line_len(self.cy),
            _ => {}
        }
        if self.dirty && self.status.as_deref() == Some("saved") {
            self.status = None;
        }
        EditorAction::None
    }

    /// Visual column (in cells) of the cursor within its line.
    pub fn cursor_col(&self) -> usize {
        self.lines[self.cy]
            .chars()
            .take(self.cx)
            .map(|c| c.width().unwrap_or(0))
            .sum()
    }

    /// Adjusts scrolling so the cursor stays in view.
    pub fn scroll_into_view(&mut self, height: usize, width: usize) {
        let (height, width) = (height.max(1), width.max(1));
        if self.cy < self.top {
            self.top = self.cy;
        } else if self.cy >= self.top + height {
            self.top = self.cy + 1 - height;
        }
        let col = self.cursor_col();
        if col < self.left {
            self.left = col;
        } else if col >= self.left + width {
            self.left = col + 1 - width;
        }
    }
}
