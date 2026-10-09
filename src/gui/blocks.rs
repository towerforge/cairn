//! Command blocks: the log in the middle of the window, one block per
//! command with its context line, the command and its output.

use eframe::egui::{
    self, Align2, CornerRadius, Label, Pos2, Rect, Stroke, TextFormat, Ui, Vec2, text::LayoutJob,
};

use crate::app::tab::{CmdBlock, CmdState, short_path};
use crate::term::output::{line_rows, segments};

use super::text::{cells_job, shorten_path};
use super::theme::{
    ACCENT, BLOCK_PAD, BORDER, BRIGHT, CURSOR, FAIL_BG, META, META_H, OUT_GAP, PAD, RED,
    SMALL_SIZE, TEXT_SIZE, bold, mono,
};
use super::widgets::paint_star;
use super::{CellSize, Gui};

impl Gui {
    pub(super) fn draw_blocks(&mut self, ui: &mut Ui, cell: CellSize) {
        let area = ui.max_rect();
        let cols = (((area.width() - 2.0 * PAD) / cell.w).floor() as usize).max(10);
        let rows = ((area.height() / cell.h).floor() as u16).max(2);
        self.app.resize_active(rows, cols as u16);

        let tab = &mut self.app.tabs[self.app.active];
        if tab.blocks.is_empty() {
            if tab.state == CmdState::Starting {
                ui.painter().text(
                    area.center(),
                    Align2::CENTER_CENTER,
                    "starting shell…",
                    mono(TEXT_SIZE),
                    META,
                );
            }
            return;
        }

        let heights: Vec<f32> = tab
            .blocks
            .iter_mut()
            .map(|b| block_height(b, cols, cell))
            .collect();
        let total: f32 = heights.iter().sum();
        let busy = tab.busy();
        let last = tab.blocks.len() - 1;
        let blink = ((ui.input(|i| i.time) * 2.0) as u64).is_multiple_of(2);

        egui::ScrollArea::vertical()
            .id_salt(("blocks", tab.id))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                // Blocks stack from the bottom, like a log.
                let offset = (viewport.height() - total).max(0.0);
                ui.set_height(total + offset);
                let origin = ui.max_rect().min;
                let view = viewport.translate(origin.to_vec2());
                let mut y = offset;
                for (i, b) in tab.blocks.iter().enumerate() {
                    let h = heights[i];
                    if y + h >= viewport.min.y && y <= viewport.max.y {
                        let rect = Rect::from_min_size(
                            origin + Vec2::new(0.0, y),
                            Vec2::new(area.width(), h),
                        );
                        let place = BlockPlace {
                            rect,
                            cols,
                            cell,
                            view,
                        };
                        draw_block(ui, b, place, busy && i == last, blink);
                    }
                    y += h;
                }
            });
    }
}

/// Block height in points: context + command + output + padding.
fn block_height(b: &mut CmdBlock, cols: usize, cell: CellSize) -> f32 {
    let rows = match b.rows_cache {
        Some((c, v, r)) if c == cols && v == b.out.version => r,
        _ => {
            let n = b.out.visible_len(b.running());
            let r = b.out.lines[..n]
                .iter()
                .map(|l| line_rows(l, cols))
                .sum::<usize>()
                + (b.out.dropped > 0) as usize;
            b.rows_cache = Some((cols, b.out.version, r));
            r
        }
    };
    let out = if rows > 0 {
        OUT_GAP + rows as f32 * cell.h
    } else {
        0.0
    };
    BLOCK_PAD + META_H + cell.h + out + BLOCK_PAD
}

/// Where and how a block is drawn.
struct BlockPlace {
    /// Block rect (full view width).
    rect: Rect,
    cols: usize,
    cell: CellSize,
    /// Visible area, in screen coordinates.
    view: Rect,
}

/// Block context line, left part: `~/path git:(branch) 3 • +48 -12`.
fn meta_job(b: &CmdBlock, max_width: f32) -> LayoutJob {
    let mut text = match &b.host {
        Some(host) => format!("{host}:{}", shorten_path(&b.cwd, 60)),
        None => shorten_path(&short_path(&b.cwd), 60),
    };
    if let Some(br) = &b.branch {
        text.push_str(&format!(" git:({br})"));
    }
    if let Some(g) = b.git.filter(|g| g.files > 0) {
        text.push_str(&format!(
            " {} • +{} -{}",
            g.files, g.insertions, g.deletions
        ));
    }
    let mut job = LayoutJob::single_section(
        text,
        TextFormat {
            font_id: mono(SMALL_SIZE),
            color: META,
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width.max(40.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    job
}

/// Right end of the context line: the time so far while it runs (in the
/// accent colour), or the exit code if it failed and the duration.
fn time_job(b: &CmdBlock) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color| TextFormat {
        font_id: mono(SMALL_SIZE),
        color,
        ..Default::default()
    };
    match b.end {
        None => job.append(&fmt_duration(b.started.elapsed()), 0.0, fmt(ACCENT)),
        Some((code, d)) => {
            if let Some(c) = code.filter(|c| *c != 0) {
                job.append(&format!("exit {c} ·"), 0.0, fmt(RED));
            }
            let gap = if job.sections.is_empty() { 0.0 } else { 6.0 };
            job.append(&fmt_duration(d), gap, fmt(META));
        }
    }
    job
}

fn draw_block(ui: &mut Ui, b: &CmdBlock, place: BlockPlace, want_cursor: bool, blink: bool) {
    let BlockPlace {
        rect,
        cols,
        cell,
        view,
    } = place;
    let p = ui.painter().clone();
    if b.failed() {
        p.rect_filled(rect, CornerRadius::ZERO, FAIL_BG);
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
            CornerRadius::ZERO,
            RED,
        );
    }
    p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, BORDER));

    // Context on the left; time on the right (with the star while running).
    let x = rect.left() + PAD;
    let meta_y = rect.top() + BLOCK_PAD;
    let cy = meta_y + META_H / 2.0;
    let time = p.layout_job(time_job(b));
    let time_x = rect.right() - PAD - time.size().x;
    let time_y = cy - time.size().y / 2.0;
    p.galley(Pos2::new(time_x, time_y), time, META);
    let mut left_edge = time_x;
    if b.running() {
        let at = Pos2::new(time_x - 10.0, cy);
        paint_star(&p, at, SMALL_SIZE, ACCENT, ui.input(|i| i.time));
        left_edge = at.x - 8.0;
    }
    let g = p.layout_job(meta_job(b, left_edge - 16.0 - x));
    p.galley(Pos2::new(x, cy - g.size().y / 2.0), g, META);

    // Command.
    let cmd_y = meta_y + META_H;
    let mut job = LayoutJob::single_section(
        b.cmd.clone(),
        TextFormat {
            font_id: bold(TEXT_SIZE),
            color: BRIGHT,
            ..Default::default()
        },
    );
    job.wrap.max_width = rect.width() - 2.0 * PAD;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    put_left(
        ui,
        Rect::from_min_size(
            Pos2::new(x, cmd_y),
            Vec2::new(rect.width() - 2.0 * PAD, cell.h),
        ),
        Label::new(job).selectable(true),
    );

    // Output.
    let n = b.out.visible_len(b.running());
    let out_top = cmd_y + cell.h + OUT_GAP;
    let row_y = |row: usize| out_top + row as f32 * cell.h;
    let visible = |y: f32| y + cell.h >= view.min.y && y <= view.max.y;
    let mut row = 0usize;
    if b.out.dropped > 0 {
        if visible(row_y(0)) {
            p.text(
                Pos2::new(x, row_y(0) + cell.h / 2.0),
                Align2::LEFT_CENTER,
                format!("… {} earlier lines omitted", b.out.dropped),
                mono(SMALL_SIZE),
                META,
            );
        }
        row += 1;
    }
    for (li, line) in b.out.lines[..n].iter().enumerate() {
        let lr = line_rows(line, cols);
        let y0 = row_y(row);
        if y0 > view.max.y {
            break;
        }
        if y0 + lr as f32 * cell.h < view.min.y {
            row += lr;
            continue;
        }
        for (s, e) in segments(line, cols) {
            let y = row_y(row);
            if visible(y) {
                if e > s {
                    put_left(
                        ui,
                        Rect::from_min_size(
                            Pos2::new(x, y),
                            Vec2::new(rect.width() - 2.0 * PAD, cell.h),
                        ),
                        Label::new(cells_job(&line[s..e])).selectable(true).extend(),
                    );
                }
                if want_cursor && li == b.out.row && blink {
                    let col = b.out.col;
                    let in_seg = (col >= s && col < e) || (col >= e && e == line.len());
                    if in_seg {
                        let cx = (col.min(e) - s) + col.saturating_sub(line.len());
                        let c = Rect::from_min_size(
                            Pos2::new(x + cx as f32 * cell.w, y + 1.0),
                            Vec2::new(cell.w, cell.h - 2.0),
                        );
                        p.rect_filled(c, CornerRadius::same(1), CURSOR);
                    }
                }
            }
            row += 1;
        }
    }
}

/// Places a widget in `rect`, left-aligned and vertically centered.
fn put_left(ui: &mut Ui, rect: Rect, widget: Label) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.add(widget),
    );
}

pub(super) fn fmt_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", d.as_secs_f32())
    } else {
        format!("{}m{:02}s", ms / 60_000, (ms / 1000) % 60)
    }
}
