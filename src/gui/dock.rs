//! Lists docked above the prompt: the `/` command list and the `Tab`
//! suggestions. Both look the same: a title, rows with `❯` on the selected
//! one and a thin scrollbar when they don't all fit.

use eframe::egui::{self, Align2, Id, Pos2, Rect, Sense, TextFormat, Ui, Vec2, text::LayoutJob};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Gui;
use super::theme::{
    ACCENT, ANSI, DOCK_HEAD_H, DOCK_ROW_H, DOCK_ROWS, FG, META, PAD, SMALL_SIZE, SUBTLE, TEXT_SIZE,
    TEXT2, bold, mono,
};
use super::widgets::paint_scrollbar;

impl Gui {
    /// `Tab` suggestions, above the prompt with the same style as the
    /// command list; folders in blue.
    pub(super) fn draw_completion(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(c) = self.app.completion.as_ref() else {
            return;
        };
        let n = c.items.len();
        let rows = n.min(DOCK_ROWS);
        let start = c.selected.map_or(0, |s| s.saturating_sub(rows - 1));
        let p = ui.painter().clone();
        let left = rect.left() + PAD;
        let info = format!("{n} {}", if n == 1 { "match" } else { "matches" });
        paint_dock_head(&p, rect, "Suggestions", &info);

        let mut clicked = None;
        for (row, (i, item)) in c
            .items
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .enumerate()
        {
            let r = Rect::from_min_size(
                Pos2::new(
                    rect.left(),
                    rect.top() + DOCK_HEAD_H + row as f32 * DOCK_ROW_H,
                ),
                Vec2::new(rect.width(), DOCK_ROW_H),
            );
            let resp = ui.interact(r, Id::new(("completion-row", i)), Sense::click());
            let selected = c.selected == Some(i);
            let cy = r.center().y;
            if selected {
                p.text(
                    Pos2::new(left, cy),
                    Align2::LEFT_CENTER,
                    "❯",
                    bold(TEXT_SIZE),
                    ACCENT,
                );
            }
            let (font, color) = if item.dir {
                (bold(TEXT_SIZE), ANSI[12])
            } else if selected {
                (mono(TEXT_SIZE), TEXT2)
            } else if resp.hovered() {
                (mono(TEXT_SIZE), FG)
            } else {
                (mono(TEXT_SIZE), SUBTLE)
            };
            let mut job = LayoutJob::single_section(
                item.label.clone(),
                TextFormat {
                    font_id: font,
                    color,
                    ..Default::default()
                },
            );
            job.wrap.max_width = r.width() - 2.0 * PAD - 18.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = p.layout_job(job);
            p.galley(Pos2::new(left + 18.0, cy - g.size().y / 2.0), g, color);
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        paint_dock_scrollbar(&p, rect, n, rows, start);
        if let Some(i) = clicked {
            self.app.pick_completion(i);
        } else {
            self.wheel_over(ui.ctx(), rect);
        }
    }

    /// Commands matching the input, right above the prompt (moon style):
    /// title, no surface of its own, `❯` on the selected one and a scrollbar
    /// on the right if they don't all fit.
    pub(super) fn draw_palette(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        list: &[&'static crate::app::SlashCommand],
    ) {
        let n = list.len();
        let rows = n.min(DOCK_ROWS);
        let sel = self.app.palette_sel.min(n - 1);
        let start = sel.saturating_sub(rows - 1);
        // What's typed after `/`: in accent; the rest is what would be completed.
        let typed = self.app.typing_command().unwrap_or("").chars().count();
        let p = ui.painter().clone();
        let left = rect.left() + PAD;

        let info = format!("{n} {}", if n == 1 { "command" } else { "commands" });
        paint_dock_head(&p, rect, "Commands", &info);

        // Description column: the same for all commands.
        let name_w = crate::app::COMMANDS
            .iter()
            .map(|c| {
                let label = format!("/{} {}", c.name, c.args);
                p.layout_no_wrap(label, mono(TEXT_SIZE), FG).size().x
            })
            .fold(0.0, f32::max);
        let desc_x = left + 18.0 + name_w + 24.0;

        let mut clicked = None;
        for (row, (i, c)) in list.iter().enumerate().skip(start).take(rows).enumerate() {
            let top = rect.top() + DOCK_HEAD_H + row as f32 * DOCK_ROW_H;
            let r = Rect::from_min_size(
                Pos2::new(rect.left(), top),
                Vec2::new(rect.width(), DOCK_ROW_H),
            );
            let resp = ui.interact(r, Id::new(("palette-row", i)), Sense::click());
            let selected = i == sel;
            let cy = r.center().y;
            if selected {
                p.text(
                    Pos2::new(left, cy),
                    Align2::LEFT_CENTER,
                    "❯",
                    bold(TEXT_SIZE),
                    ACCENT,
                );
            }
            let cut = c
                .name
                .char_indices()
                .nth(typed)
                .map_or(c.name.len(), |(b, _)| b);
            let (head, tail) = c.name.split_at(cut);
            let mut x = left + 18.0;
            x = p
                .text(
                    Pos2::new(x, cy),
                    Align2::LEFT_CENTER,
                    format!("/{head}"),
                    bold(TEXT_SIZE),
                    ACCENT,
                )
                .right();
            x = p
                .text(
                    Pos2::new(x, cy),
                    Align2::LEFT_CENTER,
                    tail,
                    mono(TEXT_SIZE),
                    if selected { SUBTLE } else { META },
                )
                .right();
            if !c.args.is_empty() {
                p.text(
                    Pos2::new(x + 8.0, cy),
                    Align2::LEFT_CENTER,
                    c.args,
                    mono(TEXT_SIZE),
                    SUBTLE,
                );
            }
            let desc_color = if selected {
                TEXT2
            } else if resp.hovered() {
                FG
            } else {
                META
            };
            p.text(
                Pos2::new(desc_x, cy),
                Align2::LEFT_CENTER,
                c.desc,
                mono(TEXT_SIZE),
                desc_color,
            );
            if resp.clicked() {
                clicked = Some(i);
            }
        }

        paint_dock_scrollbar(&p, rect, n, rows, start);

        if let Some(i) = clicked {
            self.app.palette_sel = i;
            self.key(KeyCode::Enter, KeyModifiers::NONE);
        } else {
            self.wheel_over(ui.ctx(), rect);
        }
    }
}

/// Height of a list above the prompt with `n` items.
pub(super) fn dock_height(n: usize) -> f32 {
    DOCK_HEAD_H + n.min(DOCK_ROWS) as f32 * DOCK_ROW_H + 6.0
}

/// Title of a list above the prompt, with info on the right.
fn paint_dock_head(p: &egui::Painter, rect: Rect, title: &str, info: &str) {
    let y = rect.top() + DOCK_HEAD_H / 2.0 + 2.0;
    p.text(
        Pos2::new(rect.left() + PAD, y),
        Align2::LEFT_CENTER,
        title,
        bold(SMALL_SIZE),
        TEXT2,
    );
    p.text(
        Pos2::new(rect.right() - PAD, y),
        Align2::RIGHT_CENTER,
        info,
        mono(SMALL_SIZE),
        META,
    );
}

/// Scrollbar of a list above the prompt, if not all items fit.
fn paint_dock_scrollbar(p: &egui::Painter, rect: Rect, n: usize, rows: usize, start: usize) {
    if n <= rows {
        return;
    }
    let track = Rect::from_min_size(
        Pos2::new(rect.right() - PAD / 2.0 - 2.0, rect.top() + DOCK_HEAD_H),
        Vec2::new(3.0, rows as f32 * DOCK_ROW_H),
    );
    paint_scrollbar(p, track, n, rows, start);
}
