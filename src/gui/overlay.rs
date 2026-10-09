//! Windows over the blocks: pickers (history, snippets, profiles), forms
//! and the ⌃G prefix menu. The rest of the window dims while a modal is open.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Frame, Id, LayerId, Order, Pos2, Rect, Sense, Shadow,
    Stroke, TextFormat, Ui, Vec2, text::LayoutJob,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::app::overlay::{Form, Overlay, Picker};

use super::Gui;

/// Nominal corner radius of the modals.
const MODAL_RADIUS: f32 = 12.0;
use super::theme::{
    ACCENT, BORDER, BORDER2, BRIGHT, FG, FIELD, FIELD_BORDER, HOVER, META, PAD, PROMPT_H, RAISED,
    RED, RULE, SELECTED, SMALL_SIZE, SUBTLE, TEXT_SIZE, TEXT2, YELLOW, bold, mono,
};
use super::widgets::{paint_hints, paint_scrollbar, paint_selection, squircle};

impl Gui {
    pub(super) fn draw_picker(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let size = Vec2::new(
            (screen.width() * 0.7).clamp(420.0, 860.0),
            (screen.height() * 0.62).clamp(260.0, 560.0),
        );
        let mut clicked_row = None;
        let mut area = Rect::NOTHING;
        let Some(Overlay::Picker(p)) = self.app.overlay.as_mut() else {
            return;
        };
        overlay_area(
            ctx,
            "picker",
            Align2::CENTER_CENTER,
            Vec2::ZERO,
            size,
            |ui| {
                area = Rect::from_min_size(ui.max_rect().min, size);
                clicked_row = picker_contents(ui, p, size);
            },
        );
        self.wheel_over(ctx, area);
        if let Some(row) = clicked_row {
            if let Some(Overlay::Picker(p)) = self.app.overlay.as_mut() {
                p.selected = row;
            }
            self.key(KeyCode::Enter, KeyModifiers::NONE);
        }
    }

    pub(super) fn draw_form(&mut self, ctx: &egui::Context) {
        let Some(Overlay::Form(f)) = self.app.overlay.as_ref() else {
            return;
        };
        let size = Vec2::new(560.0, 110.0 + f.fields.len() as f32 * 58.0);
        overlay_area(ctx, "form", Align2::CENTER_CENTER, Vec2::ZERO, size, |ui| {
            form_contents(ui, f, size);
        });
    }
}

pub(super) fn dim_background(ctx: &egui::Context) {
    let rect = ctx.content_rect();
    ctx.layer_painter(LayerId::new(Order::Middle, Id::new("dim")))
        .rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(153));
}

/// Overlay window: accent border, shadow and continuous-curvature corners,
/// the way macOS rounds its windows (see `widgets::squircle`).
fn overlay_area(
    ctx: &egui::Context,
    id: &str,
    anchor: Align2,
    offset: Vec2,
    size: Vec2,
    add: impl FnOnce(&mut Ui),
) {
    egui::Area::new(Id::new(id))
        .order(Order::Foreground)
        .anchor(anchor, offset)
        .show(ctx, |ui| {
            let rect = Rect::from_min_size(ui.max_rect().min, size);
            let shadow = Shadow {
                offset: [0, 18],
                blur: 48,
                spread: 0,
                color: Color32::from_black_alpha(140),
            };
            // The shadow is blurred, so a plain rounded rect of the same
            // size is indistinguishable under it.
            ui.painter()
                .add(shadow.as_shape(rect, CornerRadius::same(MODAL_RADIUS as u8 + 6)));
            // The border goes inside the edge, as a frame's does.
            ui.painter().add(egui::Shape::convex_polygon(
                squircle(rect.shrink(0.5), MODAL_RADIUS),
                RAISED,
                Stroke::new(1.0, ACCENT),
            ));
            ui.set_min_size(size);
            ui.set_max_size(size);
            add(ui);
        });
}

/// Picker contents. Returns the (visible) row clicked with the mouse.
fn picker_contents(ui: &mut Ui, p: &mut Picker, size: Vec2) -> Option<usize> {
    let rect = Rect::from_min_size(ui.max_rect().min, size);
    let painter = ui.painter().clone();

    // Header: title + filter field.
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 56.0));
    let r = painter.text(
        Pos2::new(header.left() + 18.0, header.center().y),
        Align2::LEFT_CENTER,
        p.kind.title(),
        bold(TEXT_SIZE),
        ACCENT,
    );
    let field = Rect::from_min_max(
        Pos2::new(r.right() + 14.0, header.center().y - 16.0),
        Pos2::new(header.right() - 16.0, header.center().y + 16.0),
    );
    painter.rect(
        field,
        CornerRadius::same(6),
        FIELD,
        Stroke::new(1.0, FIELD_BORDER),
        egui::StrokeKind::Inside,
    );
    let cell_w = ui.fonts_mut(|f| f.glyph_width(&mono(TEXT_SIZE), 'M'));
    let cols = ((field.width() - 20.0) / cell_w) as usize;
    let (text, cx) = p.filter.view(cols.max(4));
    let tpos = Pos2::new(field.left() + 10.0, field.center().y);
    paint_selection(
        &painter,
        &p.filter,
        cols.max(4),
        Rect::from_min_max(
            Pos2::new(tpos.x, field.center().y - 8.0),
            Pos2::new(field.right() - 10.0, field.center().y + 8.0),
        ),
        cell_w,
    );
    painter.text(tpos, Align2::LEFT_CENTER, text, mono(TEXT_SIZE), BRIGHT);
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(tpos.x + cx as f32 * cell_w, field.center().y - 8.0),
            Vec2::new(2.0, 16.0),
        ),
        CornerRadius::same(1),
        ACCENT,
    );
    painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, BORDER2));

    // Footer.
    let footer = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 38.0), rect.max);
    painter.hline(footer.x_range(), footer.top(), Stroke::new(1.0, BORDER2));
    let foot = Pos2::new(footer.left() + 18.0, footer.center().y);
    if p.confirm {
        painter.text(
            foot,
            Align2::LEFT_CENTER,
            "⌃K again to clear history",
            mono(SMALL_SIZE),
            YELLOW,
        );
    } else {
        paint_hints(&painter, foot, p.kind.hints());
    }

    // List.
    let list = Rect::from_min_max(
        Pos2::new(rect.left() + 8.0, header.bottom() + 8.0),
        Pos2::new(rect.right() - 8.0, footer.top() - 8.0),
    );
    let visible = p.visible();
    if visible.is_empty() {
        let msg = if p.items.is_empty() {
            "empty"
        } else {
            "no matches"
        };
        painter.text(
            Pos2::new(list.left() + 12.0, list.top() + 18.0),
            Align2::LEFT_CENTER,
            msg,
            mono(TEXT_SIZE),
            META,
        );
        return None;
    }
    p.selected = p.selected.min(visible.len() - 1);
    let row_h = 34.0;
    let n = ((list.height() / row_h).floor() as usize).max(1);
    let first = p.selected.saturating_sub(n - 1);
    // If not all fit, the scrollbar goes to the right of the rows.
    let overflow = visible.len() > n;
    let row_w = list.width() - if overflow { 12.0 } else { 0.0 };
    if overflow {
        let track = Rect::from_min_size(
            Pos2::new(list.right() - 3.0, list.top()),
            Vec2::new(3.0, n as f32 * row_h - 2.0),
        );
        paint_scrollbar(&painter, track, visible.len(), n, first);
    }
    let mut clicked = None;
    for (row, &idx) in visible.iter().enumerate().skip(first).take(n) {
        let it = &p.items[idx];
        let r = Rect::from_min_size(
            Pos2::new(list.left(), list.top() + (row - first) as f32 * row_h),
            Vec2::new(row_w, row_h - 2.0),
        );
        let resp = ui.interact(r, Id::new(("picker-row", row)), Sense::click());
        let selected = row == p.selected;
        if selected {
            painter.rect_filled(r, CornerRadius::same(6), SELECTED);
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(r.left() + 8.0, r.center().y - 9.0),
                    Vec2::new(3.0, 18.0),
                ),
                CornerRadius::same(2),
                ACCENT,
            );
        } else if resp.hovered() {
            painter.rect_filled(r, CornerRadius::same(6), HOVER);
        }
        let detail_w = if it.detail.is_empty() {
            0.0
        } else {
            let g = painter.layout_no_wrap(it.detail.clone(), mono(SMALL_SIZE), SUBTLE);
            let w = g.size().x;
            painter.galley(
                Pos2::new(r.right() - 12.0 - w, r.center().y - g.size().y / 2.0),
                g,
                SUBTLE,
            );
            w + 24.0
        };
        let mut job = LayoutJob::single_section(
            it.label.clone(),
            TextFormat {
                font_id: if selected {
                    bold(TEXT_SIZE)
                } else {
                    mono(TEXT_SIZE)
                },
                color: if selected { BRIGHT } else { FG },
                ..Default::default()
            },
        );
        job.wrap.max_width = (r.width() - 24.0 - detail_w).max(40.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = painter.layout_job(job);
        painter.galley(
            Pos2::new(r.left() + 20.0, r.center().y - g.size().y / 2.0),
            g,
            FG,
        );
        if resp.clicked() {
            clicked = Some(row);
        }
    }
    clicked
}

fn form_contents(ui: &mut Ui, f: &Form, size: Vec2) {
    let rect = Rect::from_min_size(ui.max_rect().min, size);
    let p = ui.painter().clone();
    p.text(
        Pos2::new(rect.left() + 18.0, rect.top() + 26.0),
        Align2::LEFT_CENTER,
        f.title(),
        bold(TEXT_SIZE),
        ACCENT,
    );
    p.hline(rect.x_range(), rect.top() + 50.0, Stroke::new(1.0, BORDER));
    let cell_w = ui.fonts_mut(|fo| fo.glyph_width(&mono(TEXT_SIZE), 'M'));
    for (i, (label, input)) in f.fields.iter().enumerate() {
        let y = rect.top() + 64.0 + i as f32 * 58.0;
        let focused = i == f.focus;
        p.text(
            Pos2::new(rect.left() + 18.0, y + 8.0),
            Align2::LEFT_CENTER,
            *label,
            mono(SMALL_SIZE),
            if focused { ACCENT } else { SUBTLE },
        );
        let field = Rect::from_min_size(
            Pos2::new(rect.left() + 18.0, y + 20.0),
            Vec2::new(rect.width() - 36.0, 30.0),
        );
        p.rect(
            field,
            CornerRadius::same(6),
            FIELD,
            Stroke::new(1.0, if focused { ACCENT } else { RULE }),
            egui::StrokeKind::Inside,
        );
        let cols = ((field.width() - 20.0) / cell_w) as usize;
        let (text, cx) = input.view(cols.max(4));
        let tpos = Pos2::new(field.left() + 10.0, field.center().y);
        paint_selection(
            &p,
            input,
            cols.max(4),
            Rect::from_min_max(
                Pos2::new(tpos.x, field.center().y - 8.0),
                Pos2::new(field.right() - 10.0, field.center().y + 8.0),
            ),
            cell_w,
        );
        p.text(tpos, Align2::LEFT_CENTER, text, mono(TEXT_SIZE), BRIGHT);
        if focused {
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(tpos.x + cx as f32 * cell_w, field.center().y - 8.0),
                    Vec2::new(2.0, 16.0),
                ),
                CornerRadius::same(1),
                ACCENT,
            );
        }
    }
    let foot = Pos2::new(rect.left() + 18.0, rect.bottom() - 20.0);
    match &f.error {
        Some(e) => {
            p.text(foot, Align2::LEFT_CENTER, e, mono(SMALL_SIZE), RED);
        }
        None => paint_hints(
            &p,
            foot,
            &[("↵", "next/save"), ("Tab", "switch"), ("Esc", "cancel")],
        ),
    }
}

pub(super) fn draw_prefix_menu(ctx: &egui::Context) {
    const KEYS: [(&str, &str); 13] = [
        ("t", "new tab"),
        ("h", "history"),
        ("w", "close tab"),
        ("s", "snippets"),
        ("n / p", "next/prev tab"),
        ("r", "profiles"),
        ("1–9", "go to tab"),
        ("e", "show/hide editor"),
        ("Tab", "focus terminal/editor"),
        ("k", "clear blocks"),
        ("y", "copy last output"),
        ("g", "send ^G to program"),
        ("q", "quit"),
    ];
    let rows = KEYS.len().div_ceil(2) as f32;
    let size = Vec2::new(600.0, 52.0 + rows * 26.0);
    let frame = Frame::new()
        .fill(RAISED)
        .stroke(Stroke::new(1.0, RULE))
        .shadow(Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(160),
        });
    egui::Area::new(Id::new("prefix"))
        .order(Order::Foreground)
        .anchor(Align2::LEFT_BOTTOM, Vec2::new(PAD, -(PROMPT_H + 18.0)))
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                let rect = Rect::from_min_size(ui.max_rect().min, size);
                let p = ui.painter();
                let r = p.text(
                    Pos2::new(rect.left() + 14.0, rect.top() + 20.0),
                    Align2::LEFT_CENTER,
                    "⌃G",
                    bold(SMALL_SIZE),
                    TEXT2,
                );
                p.text(
                    Pos2::new(r.right() + 10.0, rect.top() + 20.0),
                    Align2::LEFT_CENTER,
                    "press a key",
                    mono(SMALL_SIZE),
                    META,
                );
                p.hline(rect.x_range(), rect.top() + 40.0, Stroke::new(1.0, BORDER2));
                for (i, (k, d)) in KEYS.iter().enumerate() {
                    let col = (i % 2) as f32;
                    let x = rect.left() + 14.0 + col * (rect.width() / 2.0);
                    let y = rect.top() + 58.0 + (i / 2) as f32 * 26.0;
                    p.text(
                        Pos2::new(x + 44.0, y),
                        Align2::RIGHT_CENTER,
                        *k,
                        bold(TEXT_SIZE),
                        ACCENT,
                    );
                    p.text(
                        Pos2::new(x + 58.0, y),
                        Align2::LEFT_CENTER,
                        *d,
                        mono(TEXT_SIZE),
                        FG,
                    );
                }
            });
        });
}
