//! Fullscreen programs (vim, htop, less…): the vt100 grid painted run by
//! run, mouse selection and copy, and the mouse forwarded to the program.
//! While a session has not printed anything yet (an ssh that cannot reach
//! its server, say), the empty view says what it is waiting for.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, TextFormat, Ui, Vec2,
    text::LayoutJob,
};
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::MouseProtocolMode;

use crate::app::tab::Tab;
use crate::term::keys;

use super::blocks::fmt_duration;
use super::text::xterm256;
use super::theme::{
    ACCENT, BG, CURSOR, FG, META, PAD, RAISED, SELECTION, SMALL_SIZE, TEXT_SIZE, TEXT2, YELLOW,
    bold, mono,
};
use super::widgets::paint_star;
use super::{CellSize, Gui};

/// Selection in the fullscreen terminal grid, in `(row, column)` cells
/// of the visible screen.
#[derive(Clone, Copy)]
pub(super) struct GridSel {
    tab: u64,
    anchor: (u16, u16),
    head: (u16, u16),
}

impl GridSel {
    /// Ordered ends: start inclusive, end exclusive.
    fn range(&self) -> ((u16, u16), (u16, u16)) {
        let (a, b) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        (a, (b.0, b.1 + 1))
    }
}

impl Gui {
    pub(super) fn draw_fullscreen(&mut self, ui: &mut Ui, cell: CellSize) {
        // No bar: the program fills the whole area below the tabs.
        let area = ui.max_rect();
        let live = Rect::from_min_max(
            Pos2::new(area.left() + PAD / 2.0, area.top() + 6.0),
            Pos2::new(area.right() - PAD / 2.0, area.bottom()),
        );
        let cols = ((live.width() / cell.w).floor() as u16).max(10);
        let rows = ((live.height() / cell.h).floor() as u16).max(2);
        self.app.resize_active(rows, cols);

        let tab = &mut self.app.tabs[self.app.active];
        let p = ui.painter().clone();
        let scrolled = tab.session.vt.screen().scrollback();

        // Mouse: to the program if it asked for it; otherwise the wheel sends arrows.
        // With Shift (or Option on macOS) it selects even if the program asked.
        let force = ui.input(|i| i.modifiers.shift || i.modifiers.alt);
        let select =
            force || tab.session.vt.screen().mouse_protocol_mode() == MouseProtocolMode::None;
        forward_mouse(ui, tab, live, cell, !select);

        let screen = tab.session.vt.screen();
        let (vrows, vcols) = screen.size();

        // Mouse selection: drag selects, a click clears it.
        if self.grid_sel.is_some_and(|s| s.tab != tab.id) {
            self.grid_sel = None;
        }
        let resp = ui.interact(live, Id::new("fullscreen-select"), Sense::drag());
        if select {
            let to_cell = |pos: Pos2| {
                let r = ((pos.y - live.top()) / cell.h).clamp(0.0, (vrows - 1) as f32);
                let c = ((pos.x - live.left()) / cell.w).clamp(0.0, (vcols - 1) as f32);
                (r as u16, c as u16)
            };
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() {
                self.grid_sel = None;
            }
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(at) = ui.input(|i| i.pointer.press_origin()).map(to_cell) {
                    self.grid_sel = Some(GridSel {
                        tab: tab.id,
                        anchor: at,
                        head: at,
                    });
                }
            } else if resp.dragged_by(egui::PointerButton::Primary)
                && let (Some(s), Some(pos)) = (&mut self.grid_sel, resp.interact_pointer_pos())
            {
                s.head = to_cell(pos);
            }
        }
        for r in 0..vrows {
            let y = live.top() + r as f32 * cell.h;
            let mut c = 0;
            while c < vcols {
                let Some(first) = screen.cell(r, c) else {
                    break;
                };
                // Group consecutive cells with the same style.
                let key = vt_style_key(first);
                let start = c;
                let mut text = String::new();
                while c < vcols {
                    let Some(cl) = screen.cell(r, c) else { break };
                    if vt_style_key(cl) != key {
                        break;
                    }
                    if !cl.is_wide_continuation() {
                        let s = cl.contents();
                        text.push_str(if s.is_empty() { " " } else { s });
                    }
                    c += 1;
                }
                let (fg, bg, b, it, ul) = vt_colors(first);
                let x = live.left() + start as f32 * cell.w;
                let run = Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new((c - start) as f32 * cell.w, cell.h),
                );
                if bg != BG {
                    p.rect_filled(run, CornerRadius::ZERO, bg);
                }
                if !text.trim().is_empty() {
                    let job = LayoutJob::single_section(
                        text,
                        TextFormat {
                            font_id: if b { bold(TEXT_SIZE) } else { mono(TEXT_SIZE) },
                            color: fg,
                            italics: it,
                            underline: if ul {
                                Stroke::new(1.0, fg)
                            } else {
                                Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                    let g = p.layout_job(job);
                    p.galley(Pos2::new(x, y + 1.5), g, fg);
                }
            }
        }
        if !screen.hide_cursor() && scrolled == 0 {
            let (cr, cc) = screen.cursor_position();
            let r = Rect::from_min_size(
                Pos2::new(
                    live.left() + cc as f32 * cell.w,
                    live.top() + cr as f32 * cell.h,
                ),
                Vec2::new(cell.w, cell.h),
            );
            p.rect_filled(r, CornerRadius::ZERO, CURSOR);
        }

        if let Some(sel) = self.grid_sel {
            let ((r0, c0), (r1, c1)) = sel.range();
            for r in r0..=r1.min(vrows - 1) {
                let from = if r == r0 { c0 } else { 0 };
                let to = if r == r1 { c1 } else { vcols }.min(vcols);
                if to > from {
                    let rect = Rect::from_min_size(
                        Pos2::new(
                            live.left() + from as f32 * cell.w,
                            live.top() + r as f32 * cell.h,
                        ),
                        Vec2::new((to - from) as f32 * cell.w, cell.h),
                    );
                    p.rect_filled(rect, CornerRadius::ZERO, SELECTION);
                }
            }
        }

        if let Some(waited) = waiting_for_output(tab) {
            let cmd = tab.blocks.last().map_or("", |b| b.cmd.as_str());
            paint_waiting(ui, area, cmd, waited);
        }

        // Only while viewing scrollback: a notice at the top right.
        if scrolled > 0 {
            let galley = p.layout_no_wrap(
                format!("history ↑{scrolled} · type to return"),
                mono(SMALL_SIZE),
                YELLOW,
            );
            let r = Rect::from_min_size(
                Pos2::new(
                    area.right() - PAD - galley.size().x - 20.0,
                    area.top() + 10.0,
                ),
                galley.size() + Vec2::new(20.0, 10.0),
            );
            p.rect_filled(r, CornerRadius::same(6), RAISED);
            p.galley(r.center() - galley.size() / 2.0, galley, YELLOW);
        }
    }

    /// Copies the fullscreen terminal selection, if any.
    pub(super) fn copy_grid_sel(&mut self) -> bool {
        let tab = self.app.tab();
        let Some(sel) = self
            .grid_sel
            .filter(|s| s.tab == tab.id && tab.fullscreen())
        else {
            return false;
        };
        let ((r0, c0), (r1, c1)) = sel.range();
        let text = tab.session.vt.screen().contents_between(r0, c0, r1, c1);
        if !text.is_empty()
            && let Err(e) = self.app.set_clipboard(&text)
        {
            self.app.flash(format!("could not copy: {e}"));
        }
        true
    }
}

type StyleKey = (vt100::Color, vt100::Color, bool, bool, bool, bool, bool);

fn vt_style_key(c: &vt100::Cell) -> StyleKey {
    (
        c.fgcolor(),
        c.bgcolor(),
        c.bold(),
        c.italic(),
        c.underline(),
        c.inverse(),
        c.dim(),
    )
}

/// Colors and attributes of a vt100 cell: (fg, bg, bold,
/// italic, underline).
fn vt_colors(c: &vt100::Cell) -> (Color32, Color32, bool, bool, bool) {
    let conv = |col: vt100::Color, default: Color32| match col {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => xterm256(i),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    };
    let mut fg = conv(c.fgcolor(), FG);
    let mut bg = conv(c.bgcolor(), BG);
    if c.inverse() {
        std::mem::swap(&mut fg, &mut bg);
    }
    if c.dim() {
        fg = fg.gamma_multiply(0.6);
    }
    (fg, bg, c.bold(), c.italic(), c.underline())
}

/// Mouse over a fullscreen program: sent to it if it asked for it;
/// otherwise the wheel is translated into arrows (for less, man…).
/// Forwards the mouse to the program. Without `buttons` only the wheel:
/// clicks and drags are for selecting.
fn forward_mouse(ui: &Ui, tab: &mut Tab, live: Rect, cell: CellSize, buttons: bool) {
    let mode = tab.session.vt.screen().mouse_protocol_mode();
    let app_cursor = tab.session.vt.screen().application_cursor();
    let (events, pointer, primary_down) = ui.ctx().input(|i| {
        (
            i.events.clone(),
            i.pointer.hover_pos(),
            i.pointer.primary_down(),
        )
    });
    let to_cell = |pos: Pos2| -> Option<(u16, u16)> {
        live.contains(pos).then(|| {
            (
                ((pos.x - live.left()) / cell.w) as u16,
                ((pos.y - live.top()) / cell.h) as u16,
            )
        })
    };
    let send = |tab: &mut Tab, kind: MouseEventKind, (c, r): (u16, u16), motion: bool| {
        let ev = MouseEvent {
            kind,
            column: c,
            row: r,
            modifiers: KeyModifiers::NONE,
        };
        if let Some(bytes) = keys::encode_mouse(&ev, c, r, motion) {
            tab.session.write(&bytes);
        }
    };
    for ev in events {
        match ev {
            egui::Event::MouseWheel { delta, .. } => {
                let Some(at) = pointer.and_then(to_cell) else {
                    continue;
                };
                let up = delta.y > 0.0;
                if mode == MouseProtocolMode::None && !tab.session.fullscreen() {
                    // Interactive session (ssh…): the wheel scrolls its history.
                    let screen = tab.session.vt.screen_mut();
                    let cur = screen.scrollback();
                    screen.set_scrollback(if up { cur + 3 } else { cur.saturating_sub(3) });
                } else if mode == MouseProtocolMode::None {
                    let arrow: &[u8] = match (up, app_cursor) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    tab.session.write(arrow);
                } else {
                    let kind = if up {
                        MouseEventKind::ScrollUp
                    } else {
                        MouseEventKind::ScrollDown
                    };
                    send(tab, kind, at, false);
                }
            }
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                ..
            } if buttons && mode != MouseProtocolMode::None => {
                let Some(at) = to_cell(pos) else { continue };
                let button = match button {
                    egui::PointerButton::Primary => MouseButton::Left,
                    egui::PointerButton::Secondary => MouseButton::Right,
                    egui::PointerButton::Middle => MouseButton::Middle,
                    _ => continue,
                };
                if !pressed && mode == MouseProtocolMode::Press {
                    continue;
                }
                let kind = if pressed {
                    MouseEventKind::Down(button)
                } else {
                    MouseEventKind::Up(button)
                };
                send(tab, kind, at, false);
            }
            egui::Event::PointerMoved(pos)
                if buttons
                    && matches!(
                        mode,
                        MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
                    ) =>
            {
                let Some(at) = to_cell(pos) else { continue };
                let any = mode == MouseProtocolMode::AnyMotion;
                if primary_down {
                    send(tab, MouseEventKind::Drag(MouseButton::Left), at, any);
                } else if any {
                    send(tab, MouseEventKind::Moved, at, true);
                }
            }
            _ => {}
        }
    }
}

/// Before saying anything: most sessions print within this time, and a
/// notice that flashes for a moment is noise.
const WAIT_NOTICE_AFTER: std::time::Duration = std::time::Duration::from_millis(700);

/// How long the running command has gone without putting anything on the
/// screen, once that is longer than `WAIT_NOTICE_AFTER`.
fn waiting_for_output(tab: &Tab) -> Option<std::time::Duration> {
    let block = tab.blocks.last().filter(|b| b.running())?;
    let waited = block.started.elapsed();
    (waited >= WAIT_NOTICE_AFTER && screen_is_blank(tab.session.vt.screen())).then_some(waited)
}

/// Nothing visible on the screen: no text, cursor still at the top left.
fn screen_is_blank(screen: &vt100::Screen) -> bool {
    screen.cursor_position() == (0, 0) && screen.contents().trim().is_empty()
}

/// Centered in the empty view: the star with the command, how long it has
/// been waiting, and how to give up.
fn paint_waiting(ui: &mut Ui, area: Rect, cmd: &str, waited: std::time::Duration) {
    let p = ui.painter().clone();
    let c = area.center();
    let mut job = LayoutJob::single_section(
        cmd.to_string(),
        TextFormat {
            font_id: bold(TEXT_SIZE),
            color: TEXT2,
            ..Default::default()
        },
    );
    job.wrap.max_width = (area.width() - 2.0 * PAD - 24.0).max(40.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = p.layout_job(job);
    let w = 18.0 + g.size().x;
    let x = c.x - w / 2.0;
    paint_star(
        &p,
        Pos2::new(x + 6.0, c.y - 14.0),
        TEXT_SIZE,
        ACCENT,
        ui.input(|i| i.time),
    );
    p.galley(Pos2::new(x + 18.0, c.y - 14.0 - g.size().y / 2.0), g, TEXT2);
    p.text(
        Pos2::new(c.x, c.y + 10.0),
        Align2::CENTER_CENTER,
        format!("no output yet · {} · ⌃C to cancel", fmt_duration(waited)),
        mono(SMALL_SIZE),
        META,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_is_blank_until_something_shows_up() {
        let mut vt = vt100::Parser::new(24, 80, 0);
        assert!(screen_is_blank(vt.screen()));
        // Clearing and homing (what Cairn does when a session starts).
        vt.process(b"\x1b[2J\x1b[H");
        assert!(screen_is_blank(vt.screen()));
        // A password prompt, or even a bare newline from the program.
        vt.process(b"root@host's password: ");
        assert!(!screen_is_blank(vt.screen()));
        let mut vt = vt100::Parser::new(24, 80, 0);
        vt.process(b"\r\n");
        assert!(!screen_is_blank(vt.screen()));
    }
}
