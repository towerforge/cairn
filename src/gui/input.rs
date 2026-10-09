//! Keyboard and wheel events, translated into what `App` understands.

use eframe::egui::{self, Key, Rect};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use crate::event::AppEvent;

use super::Gui;
use super::theme::{DOCK_ROWS, WHEEL_STEP};

impl Gui {
    /// Sends a key to `App`. Any mouse selection in the fullscreen
    /// terminal is dropped: the program will redraw. (The prompt's
    /// selection belongs to its line editor, which handles the key.)
    pub(super) fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.grid_sel = None;
        self.app
            .handle(AppEvent::Term(Event::Key(KeyEvent::new(code, mods))));
    }

    pub(super) fn handle_input(&mut self, ctx: &egui::Context) {
        let (events, shift) = ctx.input(|i| (i.events.clone(), i.modifiers.shift));
        let mac = cfg!(target_os = "macos");
        for ev in events {
            match ev {
                egui::Event::Text(text) => {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.key(KeyCode::Char(c), KeyModifiers::NONE);
                    }
                }
                egui::Event::Paste(text) => {
                    self.grid_sel = None;
                    self.app.handle(AppEvent::Term(Event::Paste(text)));
                }
                // Cmd+C (Ctrl+Shift+C on Linux) with text selected in the
                // fullscreen terminal or in the prompt; in blocks egui copies.
                egui::Event::Copy if self.copy_grid_sel() || self.copy_prompt_sel() => {}
                // Cmd+X (Ctrl+Shift+X on Linux) with text selected in the prompt.
                egui::Event::Cut if self.cut_prompt_sel() => {}
                // On Linux Ctrl+C/Ctrl+X arrive as Copy/Cut: send to the terminal.
                egui::Event::Copy if !mac && !shift => {
                    self.key(KeyCode::Char('c'), KeyModifiers::CONTROL)
                }
                egui::Event::Cut if !mac && !shift => {
                    self.key(KeyCode::Char('x'), KeyModifiers::CONTROL)
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => self.key_event(key, modifiers),
                _ => {}
            }
        }
    }

    fn key_event(&mut self, key: Key, m: egui::Modifiers) {
        let app_mod = if cfg!(target_os = "macos") {
            m.mac_cmd
        } else {
            m.ctrl && m.shift
        };
        if app_mod {
            self.shortcut(key);
            return;
        }
        let mut mods = KeyModifiers::NONE;
        if m.shift {
            mods |= KeyModifiers::SHIFT;
        }
        if m.ctrl {
            mods |= KeyModifiers::CONTROL;
        }
        if m.alt && !cfg!(target_os = "macos") {
            mods |= KeyModifiers::ALT;
        }
        let code = match key {
            Key::Enter => KeyCode::Enter,
            Key::Tab if m.shift => KeyCode::BackTab,
            Key::Tab => KeyCode::Tab,
            Key::Backspace => KeyCode::Backspace,
            Key::Escape => KeyCode::Esc,
            Key::ArrowUp => KeyCode::Up,
            Key::ArrowDown => KeyCode::Down,
            Key::ArrowLeft => KeyCode::Left,
            Key::ArrowRight => KeyCode::Right,
            Key::Home => KeyCode::Home,
            Key::End => KeyCode::End,
            Key::PageUp => KeyCode::PageUp,
            Key::PageDown => KeyCode::PageDown,
            Key::Insert => KeyCode::Insert,
            Key::Delete => KeyCode::Delete,
            Key::F1 => KeyCode::F(1),
            Key::F2 => KeyCode::F(2),
            Key::F3 => KeyCode::F(3),
            Key::F4 => KeyCode::F(4),
            Key::F5 => KeyCode::F(5),
            Key::F6 => KeyCode::F(6),
            Key::F7 => KeyCode::F(7),
            Key::F8 => KeyCode::F(8),
            Key::F9 => KeyCode::F(9),
            Key::F10 => KeyCode::F(10),
            Key::F11 => KeyCode::F(11),
            Key::F12 => KeyCode::F(12),
            // Ctrl combinations (plain text arrives as Event::Text).
            _ if m.ctrl => match key_char(key) {
                Some(c) => KeyCode::Char(c),
                None => return,
            },
            _ => return,
        };
        self.key(code, mods);
    }

    /// App shortcuts: Cmd on macOS, Ctrl+Shift on Linux. Copy, paste
    /// and zoom are handled by egui.
    fn shortcut(&mut self, key: Key) {
        let Some(c) = key_char(key) else { return };
        match c {
            's' if self.app.editor_focused() => {
                self.key(KeyCode::Char('s'), KeyModifiers::CONTROL);
            }
            // Select the whole prompt.
            'a' if !self.app.editor_focused()
                && !self.app.tab().busy()
                && self.app.overlay.is_none() =>
            {
                let input = &mut self.app.tabs[self.app.active].input;
                let len = input.text.chars().count();
                input.select(0, len);
            }
            't' | 'w' | 'k' | 'q' | 's' | 'r' | 'e' | '1'..='9' => self.app.command(c),
            '[' => self.app.command('p'),
            ']' => self.app.command('n'),
            _ => {}
        }
    }

    /// Mouse wheel over `rect` (a modal or a list): each step moves the
    /// selection one row. Accumulates fine trackpad deltas.
    pub(super) fn wheel_over(&mut self, ctx: &egui::Context, rect: Rect) {
        let (delta, pointer) = ctx.input(|i| {
            let delta: f32 = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * WHEEL_STEP,
                        egui::MouseWheelUnit::Page => delta.y * WHEEL_STEP * DOCK_ROWS as f32,
                    }),
                    _ => None,
                })
                .sum();
            (delta, i.pointer.hover_pos())
        });
        if !pointer.is_some_and(|p| rect.contains(p)) {
            return;
        }
        self.wheel += delta;
        while self.wheel.abs() >= WHEEL_STEP {
            let down = self.wheel < 0.0;
            self.wheel -= WHEEL_STEP.copysign(self.wheel);
            self.app.scroll_list(down);
        }
    }
}

fn key_char(key: Key) -> Option<char> {
    let name = key.symbol_or_name();
    let mut chars = name.chars();
    let c = chars.next()?;
    chars.next().is_none().then(|| c.to_ascii_lowercase())
}
