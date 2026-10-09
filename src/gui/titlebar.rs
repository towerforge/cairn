//! Tab bar: a pill per tab, the `+` button and the ⌃G badge. Its
//! background moves the window; a double click maximizes it.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2,
    ViewportCommand,
};

use super::Gui;
use super::theme::{
    ACCENT, ACTIVE_TAB, BORDER, CLOSE_HOVER, FG, HOVER, META, ON_ACCENT, SMALL_SIZE, SUBTLE,
    TAB_CLOSE_W, TEXT_SIZE, TEXT2, TRAFFIC_LIGHTS_W, bold, mono,
};

impl Gui {
    pub(super) fn draw_titlebar(&mut self, ui: &mut Ui, fullscreen_window: bool) {
        let rect = ui.max_rect();
        // Draggable background: moves the window; double click maximizes.
        let bg = ui.interact(rect, Id::new("titlebar-drag"), Sense::click_and_drag());
        if bg.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
        } else if bg.drag_started() && !self.titlebar_drag {
            self.titlebar_drag = true;
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if !bg.dragged() {
            self.titlebar_drag = false;
        }
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, BORDER),
        );

        let inset = if cfg!(target_os = "macos") && !fullscreen_window {
            TRAFFIC_LIGHTS_W
        } else {
            12.0
        };
        let mut x = rect.left() + inset;
        let cy = rect.center().y;

        let mut clicked = None;
        let mut close = None;
        for (i, tab) in self.app.tabs.iter().enumerate() {
            let active = i == self.app.active;
            let label = tab.title();
            let font = if active {
                bold(TEXT_SIZE)
            } else {
                mono(TEXT_SIZE)
            };
            let (r, pos, galley) = pill_rect(ui, x, cy, &label, font, 14.0, 30.0);
            // Space on the right for the close × (or the busy dot).
            let r = Rect::from_min_size(r.min, Vec2::new(r.width() + TAB_CLOSE_W, r.height()));
            if r.right() > rect.right() - 60.0 {
                break;
            }
            let resp = ui.interact(r, Id::new(("tab", tab.id)), Sense::click());
            let hovered = ui.rect_contains_pointer(r);
            let fill = if active {
                ACTIVE_TAB
            } else if hovered {
                HOVER
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(r, CornerRadius::same(8), fill);
            ui.painter()
                .galley(pos, galley, if active { TEXT2 } else { SUBTLE });

            let slot = Pos2::new(r.right() - TAB_CLOSE_W / 2.0 - 4.0, cy);
            if hovered || (active && !tab.busy()) {
                let x_rect = Rect::from_center_size(slot, Vec2::splat(18.0));
                let x_resp = ui.interact(x_rect, Id::new(("tab-close", tab.id)), Sense::click());
                if x_resp.hovered() {
                    ui.painter()
                        .rect_filled(x_rect, CornerRadius::same(4), CLOSE_HOVER);
                }
                let color = if x_resp.hovered() { TEXT2 } else { META };
                let d = 3.5;
                let stroke = Stroke::new(1.4, color);
                ui.painter()
                    .line_segment([slot + Vec2::new(-d, -d), slot + Vec2::new(d, d)], stroke);
                ui.painter()
                    .line_segment([slot + Vec2::new(-d, d), slot + Vec2::new(d, -d)], stroke);
                if x_resp.on_hover_text("Close tab (⌘W)").clicked() {
                    close = Some(i);
                }
            } else if tab.busy() {
                ui.painter().circle_filled(slot, 3.0, ACCENT);
            }
            if resp.middle_clicked() {
                close = Some(i);
            } else if resp.clicked() {
                clicked = Some(i);
            }
            x = r.right() + 6.0;
        }
        if let Some(i) = close {
            self.app.close_tab(i);
        } else if let Some(i) = clicked {
            self.app.active = i;
        }

        // New tab.
        let plus = Rect::from_center_size(Pos2::new(x + 15.0, cy), Vec2::splat(30.0));
        let resp = ui.interact(plus, Id::new("new-tab"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(plus, CornerRadius::same(8), HOVER);
        }
        ui.painter().text(
            plus.center(),
            Align2::CENTER_CENTER,
            "+",
            mono(15.0),
            SUBTLE,
        );
        if resp.on_hover_text("New tab (⌘T)").clicked() {
            self.app.command('t');
        }

        // Pending ⌃G prefix: a pill on the right.
        if self.app.prefix {
            let galley = ui
                .painter()
                .layout_no_wrap("⌃G".into(), bold(SMALL_SIZE), ON_ACCENT);
            let r = Rect::from_min_size(
                Pos2::new(rect.right() - 14.0 - galley.size().x - 16.0, cy - 11.0),
                Vec2::new(galley.size().x + 16.0, 22.0),
            );
            ui.painter().rect_filled(r, CornerRadius::same(6), ACCENT);
            ui.painter()
                .galley(r.center() - galley.size() / 2.0, galley, ON_ACCENT);
        }
    }
}

/// Rect of a text pill centered on `cy`, starting at `x`.
fn pill_rect(
    ui: &Ui,
    x: f32,
    cy: f32,
    text: &str,
    font: FontId,
    pad: f32,
    h: f32,
) -> (Rect, Pos2, Arc<egui::Galley>) {
    let galley = ui.painter().layout_no_wrap(text.into(), font, FG);
    let r = Rect::from_min_size(
        Pos2::new(x, cy - h / 2.0),
        Vec2::new(galley.size().x + 2.0 * pad, h),
    );
    let pos = Pos2::new(r.left() + pad, cy - galley.size().y / 2.0);
    (r, pos, galley)
}
