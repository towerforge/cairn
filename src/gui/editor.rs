//! Side editor: a header with the file name and the Code/Preview toggle,
//! the text with line numbers and cursor (or the Markdown preview) and a
//! footer with the shortcuts.

use eframe::egui::{
    self, Align2, CornerRadius, Id, Label, Pos2, Rect, Sense, Stroke, TextFormat, Ui, Vec2,
    text::LayoutJob,
};
use ratatui::style::{Color, Modifier};

use crate::app::tab::Focus;
use crate::edit::markdown;

use super::text::text_format;
use super::theme::{
    ACCENT, ACTIVE_TAB, BG, BORDER, BRIGHT, FG, META, SMALL_SIZE, SUBTLE, TEXT_SIZE, TITLEBAR,
    YELLOW, bold, mono,
};
use super::widgets::paint_hints;
use super::{CellSize, Gui};

impl Gui {
    pub(super) fn draw_editor(&mut self, ui: &mut Ui, cell: CellSize) {
        let overlay_open = self.app.overlay.is_some();
        let tab = &mut self.app.tabs[self.app.active];
        let focused = tab.focus == Focus::Editor;
        let outer = ui.max_rect();
        if ui
            .interact(outer, Id::new("editor-focus"), Sense::click())
            .clicked()
        {
            tab.focus = Focus::Editor;
        }
        let Some(editor) = tab.editor.as_mut() else {
            return;
        };

        let rect = outer;
        let p = ui.painter().clone();
        p.vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, BORDER));
        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 34.0));
        let footer = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 28.0), rect.max);
        let body = Rect::from_min_max(
            Pos2::new(rect.left(), header.bottom()),
            Pos2::new(rect.right(), footer.top()),
        );
        p.rect_filled(header, CornerRadius::ZERO, TITLEBAR);
        p.hline(
            header.x_range(),
            header.bottom() - 0.5,
            Stroke::new(1.0, if focused { ACCENT } else { BORDER }),
        );
        p.hline(
            footer.x_range(),
            footer.top() + 0.5,
            Stroke::new(1.0, BORDER),
        );

        let name = editor
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let r = p.text(
            Pos2::new(header.left() + 12.0, header.center().y),
            Align2::LEFT_CENTER,
            format!("✎ {name}"),
            bold(TEXT_SIZE),
            BRIGHT,
        );
        if editor.dirty {
            p.circle_filled(Pos2::new(r.right() + 10.0, header.center().y), 3.5, YELLOW);
        }

        // Code / Preview toggle.
        if editor.is_markdown {
            let seg_w: f32 = ["Preview", "Code"]
                .iter()
                .map(|l| {
                    p.layout_no_wrap((*l).into(), mono(SMALL_SIZE), SUBTLE)
                        .size()
                        .x
                        + 20.0
                })
                .sum::<f32>()
                + 6.0;
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(header.right() - 10.0 - seg_w, header.center().y - 13.0),
                    Vec2::new(seg_w + 4.0, 26.0),
                ),
                CornerRadius::same(6),
                BG,
            );
            let mut x = header.right() - 10.0;
            for (label, preview) in [("Preview", true), ("Code", false)] {
                let g = p.layout_no_wrap(label.into(), mono(SMALL_SIZE), SUBTLE);
                let w = g.size().x + 20.0;
                let r = Rect::from_min_size(
                    Pos2::new(x - w, header.center().y - 10.0),
                    Vec2::new(w, 20.0),
                );
                let on = editor.preview == preview;
                if on {
                    p.rect_filled(r, CornerRadius::same(4), ACTIVE_TAB);
                }
                p.galley(
                    r.center() - g.size() / 2.0,
                    g,
                    if on { BRIGHT } else { SUBTLE },
                );
                if ui
                    .interact(r, Id::new(("seg", label)), Sense::click())
                    .clicked()
                {
                    editor.preview = preview;
                }
                x -= w + 2.0;
            }
        }

        // Shortcut footer.
        let mut hints = vec![("⌘S", "save")];
        if editor.is_markdown {
            hints.push(("⌃P", if editor.preview { "code" } else { "preview" }));
        }
        hints.push(("⌃X", "close"));
        hints.push(("Esc", "terminal"));
        let foot = Pos2::new(footer.left() + 12.0, footer.center().y);
        match &editor.status {
            Some(s) => {
                p.text(foot, Align2::LEFT_CENTER, s, mono(SMALL_SIZE), YELLOW);
            }
            None => paint_hints(&p, foot, &hints),
        }

        if editor.preview {
            let inner = body.shrink2(Vec2::new(18.0, 12.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            egui::ScrollArea::vertical()
                .id_salt("md-preview")
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    for line in markdown::render(&editor.content()) {
                        ui.add(Label::new(markdown_job(&line, inner.width())).selectable(true));
                    }
                });
            return;
        }

        // Code: line numbers + text + cursor.
        let num_w = editor.lines.len().to_string().len().max(3) as f32 * cell.w + 16.0;
        let text_x = body.left() + 10.0 + num_w;
        let rows = ((body.height() - 16.0) / cell.h).floor().max(1.0) as usize;
        let cols = ((body.right() - 12.0 - text_x) / cell.w).floor().max(4.0) as usize;
        editor.scroll_into_view(rows, cols);
        for (i, line) in editor.lines.iter().enumerate().skip(editor.top).take(rows) {
            let y = body.top() + 8.0 + (i - editor.top) as f32 * cell.h + cell.h / 2.0;
            p.text(
                Pos2::new(text_x - 14.0, y),
                Align2::RIGHT_CENTER,
                (i + 1).to_string(),
                mono(SMALL_SIZE),
                if i == editor.cy { ACCENT } else { META },
            );
            let shown: String = line.chars().skip(editor.left).take(cols).collect();
            p.text(
                Pos2::new(text_x, y),
                Align2::LEFT_CENTER,
                shown,
                mono(TEXT_SIZE),
                FG,
            );
        }
        if focused && !overlay_open {
            let cx = editor.cursor_col().saturating_sub(editor.left) as f32;
            let cy = (editor.cy - editor.top) as f32;
            let c = Rect::from_min_size(
                Pos2::new(text_x + cx * cell.w, body.top() + 8.0 + cy * cell.h + 1.0),
                Vec2::new(2.0, cell.h - 2.0),
            );
            p.rect_filled(c, CornerRadius::same(1), ACCENT);
        }
    }
}

/// Markdown preview line: headings larger.
fn markdown_job(line: &ratatui::text::Line, max_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let heading =
        line.style.fg == Some(Color::Cyan) && line.style.add_modifier.contains(Modifier::BOLD);
    let size = match (
        heading,
        line.style.add_modifier.contains(Modifier::UNDERLINED),
    ) {
        (true, true) => 22.0,
        (true, false) => 17.0,
        _ => TEXT_SIZE,
    };
    for span in &line.spans {
        let mut f = text_format(line.style.patch(span.style));
        f.font_id.size = size;
        if heading {
            f.underline = Stroke::NONE;
            f.color = ACCENT;
        }
        f.line_height = Some(size * 1.55);
        job.append(&span.content, 0.0, f);
    }
    if job.sections.is_empty() {
        job.append(
            " ",
            0.0,
            TextFormat {
                font_id: mono(TEXT_SIZE),
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = max_width;
    job
}
