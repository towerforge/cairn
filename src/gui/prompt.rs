//! Prompt panel at the bottom: context chips (folder, branch, git changes
//! and shell, or a message), the input line with its cursor, and the
//! shortcut hints.
//!
//! The input line is Cairn's own, so the mouse is handled here: a click
//! moves the cursor, a drag selects and a double click selects a word. The
//! selection belongs to the line editor, so typing replaces it, `Backspace`
//! deletes it and `⌘C` / `⌘X` copy or cut it.

use eframe::egui::{self, Align2, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use unicode_width::UnicodeWidthChar;

use crate::app::tab::{CmdState, Focus, Tab, short_path};

use super::text::shorten_path;
use super::theme::{
    ACCENT, DIM, GREEN, META, PAD, RULE, SMALL_SIZE, TEXT_SIZE, TEXT2, YELLOW, mono,
};
use super::widgets::{Icon, chip, diff_chip, paint_hints, paint_selection};
use super::{CellSize, Gui};

impl Gui {
    /// Copies the text selected in the prompt, if any.
    pub(super) fn copy_prompt_sel(&mut self) -> bool {
        let Some(text) = self.app.tab().input.selected_text() else {
            return false;
        };
        if let Err(e) = self.app.set_clipboard(&text) {
            self.app.flash(format!("could not copy: {e}"));
        }
        true
    }

    /// Cuts the text selected in the prompt, if any.
    pub(super) fn cut_prompt_sel(&mut self) -> bool {
        if !self.copy_prompt_sel() {
            return false;
        }
        let active = self.app.active;
        self.app.tabs[active].input.delete_selection();
        true
    }

    pub(super) fn draw_prompt(&mut self, ui: &mut Ui, cell: CellSize) {
        let rect = ui.max_rect();
        let overlay_open = self.app.overlay.is_some() || self.app.prefix;
        let palette_open = self.app.palette().is_some();
        let completion_open = self.app.completion.is_some();
        let message = self.app.message.as_ref().map(|(m, _)| m.clone());
        let tab = &mut self.app.tabs[self.app.active];
        let focused = tab.focus == Focus::Terminal || !tab.has_editor();
        if ui
            .interact(rect, Id::new("prompt"), Sense::click())
            .clicked()
        {
            tab.focus = Focus::Terminal;
        }
        let p = ui.painter().clone();
        p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, RULE));

        // Chips: folder, branch, git changes · shell (or message).
        let cy = rect.top() + 12.0 + 11.0;
        let mut x = rect.left() + PAD;
        // The path gives way: branch, changes and shell always fit.
        let reserved = 140.0
            + tab
                .branch
                .as_ref()
                .map_or(0.0, |b| b.chars().count() as f32 * cell.w + 40.0)
            + if tab.git.is_some_and(|g| g.files > 0) {
                150.0
            } else {
                0.0
            };
        let max_chars = ((rect.width() - 2.0 * PAD - reserved - 40.0) / cell.w).max(8.0) as usize;
        if let Some(host) = &tab.remote {
            x = chip(&p, x, cy, Icon::Remote, host, ACCENT, true) + 8.0;
        }
        let local_path = if tab.remote.is_some() {
            tab.cwd.clone()
        } else {
            short_path(&tab.cwd)
        };
        let path = shorten_path(&local_path, max_chars);
        x = chip(&p, x, cy, Icon::Folder, &path, TEXT2, true) + 8.0;
        if let Some(br) = &tab.branch {
            x = chip(&p, x, cy, Icon::Branch, br, GREEN, true) + 8.0;
        }
        if let Some(g) = tab.git.filter(|g| g.files > 0) {
            diff_chip(&p, x, cy, g);
        }
        let (right, color) = match message {
            Some(m) => (m, YELLOW),
            None => (tab.session.shell_label.clone(), DIM),
        };
        p.text(
            Pos2::new(rect.right() - PAD, cy),
            Align2::RIGHT_CENTER,
            right,
            mono(SMALL_SIZE),
            color,
        );

        // Input line.
        let in_y = rect.top() + 12.0 + 22.0 + 10.0 + 10.0;
        let tx = rect.left() + PAD;
        let cols = (((rect.right() - PAD - tx) / cell.w) as usize).max(4);
        let busy = tab.busy();
        let cursor_at = |cx: u16| {
            Rect::from_min_size(
                Pos2::new(tx + cx as f32 * cell.w, in_y - 9.0),
                Vec2::new(8.0, 18.0),
            )
        };
        if busy {
            p.text(
                Pos2::new(tx, in_y),
                Align2::LEFT_CENTER,
                "Running · keyboard goes to the program",
                mono(TEXT_SIZE),
                META,
            );
        } else if tab.state == CmdState::Starting && tab.input.text.is_empty() {
            p.text(
                Pos2::new(tx, in_y),
                Align2::LEFT_CENTER,
                "starting…",
                mono(TEXT_SIZE),
                META,
            );
        } else {
            let show_cursor = focused && !overlay_open;
            let line = Rect::from_min_max(
                Pos2::new(tx, in_y - 12.0),
                Pos2::new(rect.right() - PAD, in_y + 12.0),
            );
            input_mouse(ui, tab, line, cell, cols);
            let (text, cx) = tab.input.view(cols);

            paint_selection(
                &p,
                &tab.input,
                cols,
                Rect::from_min_max(
                    Pos2::new(tx, in_y - 9.0),
                    Pos2::new(line.right(), in_y + 9.0),
                ),
                cell.w,
            );
            if text.is_empty() {
                let x0 = if show_cursor { tx + 12.0 } else { tx };
                p.text(
                    Pos2::new(x0, in_y),
                    Align2::LEFT_CENTER,
                    "Type a command…",
                    mono(TEXT_SIZE),
                    META,
                );
            } else {
                p.text(
                    Pos2::new(tx, in_y),
                    Align2::LEFT_CENTER,
                    text,
                    mono(TEXT_SIZE),
                    TEXT2,
                );
            }
            if show_cursor {
                p.rect_filled(cursor_at(cx), CornerRadius::ZERO, ACCENT);
            }
        }

        // Shortcuts depending on state.
        // With a list open, the footer stays clean.
        let update = self.app.update_available.as_ref().map(|v| format!("v{v}"));
        let mut hints: Vec<(&str, &str)> = if completion_open || palette_open {
            vec![]
        } else if busy {
            vec![("⌃C", "interrupt"), ("⌃G", "menu")]
        } else if tab.has_editor() && tab.input.text.is_empty() {
            vec![("Tab", "focus editor"), ("/help", "commands")]
        } else {
            vec![("/help", "commands"), ("/snippets", ""), ("/profiles", "")]
        };
        // A newer release, until it is installed: next to the rest.
        if let Some(v) = &update
            && !hints.is_empty()
            && !busy
        {
            hints.push(("/update", v));
        }
        let hy = in_y + 10.0 + 10.0 + 8.0;
        paint_hints(&p, Pos2::new(tx, hy), &hints);
    }
}

/// Mouse on the input line: a click moves the cursor, a drag selects and
/// a double click selects the word under the pointer; with `Shift`, a click
/// or a drag extends the selection. The cursor follows the selection, so
/// dragging past an edge scrolls the text.
fn input_mouse(ui: &mut Ui, tab: &mut Tab, line: Rect, cell: CellSize, cols: usize) {
    let resp = ui.interact(line, Id::new("prompt-input"), Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    let chars: Vec<char> = tab.input.text.chars().collect();
    let start = tab.input.first_visible(cols);
    let col = |pos: Pos2| (pos.x - line.left()) / cell.w;
    let shift = ui.input(|i| i.modifiers.shift);
    if resp.double_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let (a, b) = word_at(&chars, char_under(&chars, start, col(pos)));
            tab.input.select(a, b);
        }
    } else if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let at = char_at_x(&chars, start, col(pos));
            if shift {
                tab.input.extend_to(at);
            } else {
                tab.input.set_cursor(at);
            }
        }
    } else if resp.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
            let at = char_at_x(&chars, start, col(pos));
            if shift {
                tab.input.extend_to(at);
            } else {
                tab.input.select(at, at);
            }
        }
    } else if resp.dragged_by(egui::PointerButton::Primary)
        && let Some(pos) = resp.interact_pointer_pos()
    {
        tab.input.extend_to(char_at_x(&chars, start, col(pos)));
    }
    if resp.clicked() || resp.drag_started() {
        tab.focus = Focus::Terminal;
    }
}

/// Char boundary closest to `col` columns from the left of the visible
/// text, which begins at char `start`. Past the end: the end.
fn char_at_x(chars: &[char], start: usize, col: f32) -> usize {
    let col = col.max(0.0);
    let mut acc = 0.0;
    let mut best = start;
    let mut best_d = col;
    for (i, c) in chars.iter().enumerate().skip(start) {
        acc += c.width().unwrap_or(0) as f32;
        let d = (acc - col).abs();
        if d < best_d {
            best_d = d;
            best = i + 1;
        }
        if acc >= col {
            break;
        }
    }
    best
}

/// Char under `col` columns from the left of the visible text (which
/// begins at char `start`); past the end, the text length.
fn char_under(chars: &[char], start: usize, col: f32) -> usize {
    let mut acc = 0.0;
    for (i, c) in chars.iter().enumerate().skip(start) {
        acc += c.width().unwrap_or(0) as f32;
        if col < acc {
            return i;
        }
    }
    chars.len()
}

/// The run of blanks or of non-blanks around char `at`, as `start..end`.
/// Past the end, the last run.
fn word_at(chars: &[char], at: usize) -> (usize, usize) {
    let Some(last) = chars.len().checked_sub(1) else {
        return (0, 0);
    };
    let i = at.min(last);
    let blank = chars[i].is_whitespace();
    let mut a = i;
    while a > 0 && chars[a - 1].is_whitespace() == blank {
        a -= 1;
    }
    let mut b = i + 1;
    while b < chars.len() && chars[b].is_whitespace() == blank {
        b += 1;
    }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::widgets::cols_of;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn a_click_goes_to_the_nearest_char_boundary() {
        let c = chars("echo hola");
        assert_eq!(char_at_x(&c, 0, 0.0), 0);
        assert_eq!(char_at_x(&c, 0, 0.4), 0);
        assert_eq!(char_at_x(&c, 0, 0.6), 1);
        assert_eq!(char_at_x(&c, 0, 2.8), 3);
        assert_eq!(char_at_x(&c, 0, 40.0), 9);
        assert_eq!(char_at_x(&c, 0, -5.0), 0);
        // Scrolled: the visible text starts at `hola`.
        assert_eq!(char_at_x(&c, 5, 0.0), 5);
        assert_eq!(char_at_x(&c, 5, 1.2), 6);
    }

    #[test]
    fn wide_chars_take_two_columns() {
        let c = chars("日本x");
        assert_eq!(char_at_x(&c, 0, 1.2), 1);
        assert_eq!(char_at_x(&c, 0, 3.2), 2);
        assert_eq!(char_at_x(&c, 0, 4.7), 3);
        assert_eq!(char_under(&c, 0, 1.9), 0);
        assert_eq!(char_under(&c, 0, 2.0), 1);
        assert_eq!(cols_of(&c), 5.0);
    }

    #[test]
    fn a_double_click_takes_the_word_under_the_pointer() {
        let c = chars("echo  hola");
        assert_eq!(word_at(&c, char_under(&c, 0, 1.5)), (0, 4));
        assert_eq!(word_at(&c, char_under(&c, 0, 4.5)), (4, 6));
        assert_eq!(word_at(&c, char_under(&c, 0, 7.0)), (6, 10));
        assert_eq!(word_at(&c, char_under(&c, 0, 30.0)), (6, 10));
        assert_eq!(word_at(&[], 0), (0, 0));
    }
}
