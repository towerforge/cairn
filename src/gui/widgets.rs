//! Painting primitives shared by several panels: keycap hints,
//! scrollbars, chips and their stroke icons, and the selection of a line
//! input.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Pos2, Rect, Stroke, TextFormat, Vec2, text::LayoutJob,
};
use unicode_width::UnicodeWidthChar;

use crate::app::keycaps;
use crate::edit::input::LineInput;

use super::theme::{
    BORDER2, CHIP_BG, CHIP_TEXT, DIM, GREEN, KEYCAP, META, RED, RULE, SMALL_SIZE, TEXT_SELECTION,
    TEXT2, bold, mono,
};

/// Shortcuts: each key in its box and the dimmed action next to it.
pub(super) fn paint_hints(p: &egui::Painter, mut pos: Pos2, hints: &[(&str, &str)]) {
    for (key, action) in hints {
        for cap in keycaps(key) {
            let g = p.layout_no_wrap(cap, mono(SMALL_SIZE), TEXT2);
            let w = (g.size().x + 10.0).max(18.0);
            let r = Rect::from_center_size(Pos2::new(pos.x + w / 2.0, pos.y), Vec2::new(w, 18.0));
            p.rect_filled(r, CornerRadius::same(4), KEYCAP);
            p.galley(r.center() - g.size() / 2.0, g, TEXT2);
            pos.x = r.right() + 4.0;
        }
        let r = p.text(
            Pos2::new(pos.x + 3.0, pos.y),
            Align2::LEFT_CENTER,
            *action,
            mono(SMALL_SIZE),
            META,
        );
        pos.x = r.right() + 16.0;
    }
}

/// Thin scrollbar in `track`: `rows` of `n` rows visible,
/// starting at row `start`.
pub(super) fn paint_scrollbar(p: &egui::Painter, track: Rect, n: usize, rows: usize, start: usize) {
    p.rect_filled(track, CornerRadius::same(2), BORDER2);
    let h = (track.height() * rows as f32 / n as f32).max(12.0);
    let y = track.top() + (track.height() - h) * start as f32 / (n - rows).max(1) as f32;
    let thumb = Rect::from_min_size(Pos2::new(track.left(), y), Vec2::new(track.width(), h));
    p.rect_filled(thumb, CornerRadius::same(2), META);
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Folder,
    Branch,
    File,
    Remote,
}

/// 12 px stroke icon (same as the mockup), with its corner at `o`.
pub(super) fn paint_icon(p: &egui::Painter, o: Pos2, icon: Icon, color: Color32) {
    let k = 12.0 / 16.0;
    let pt = |x: f32, y: f32| o + Vec2::new(x * k, y * k);
    let stroke = Stroke::new(1.2, color);
    match icon {
        Icon::Folder => {
            let pts = vec![
                pt(2.0, 3.5),
                pt(6.5, 3.5),
                pt(8.0, 5.5),
                pt(14.0, 5.5),
                pt(14.0, 12.5),
                pt(2.0, 12.5),
            ];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::Branch => {
            for (cx, cy) in [(5.0, 3.5), (5.0, 12.5), (11.0, 5.0)] {
                p.circle_stroke(pt(cx, cy), 1.5 * k, stroke);
            }
            p.line_segment([pt(5.0, 5.0), pt(5.0, 11.0)], stroke);
            p.add(egui::Shape::line(
                vec![pt(11.0, 6.5), pt(10.0, 9.0), pt(5.5, 11.0)],
                stroke,
            ));
        }
        Icon::Remote => {
            let pts = vec![pt(2.0, 3.0), pt(14.0, 3.0), pt(14.0, 11.0), pt(2.0, 11.0)];
            p.add(egui::Shape::closed_line(pts, stroke));
            p.line_segment([pt(8.0, 11.0), pt(8.0, 13.5)], stroke);
            p.line_segment([pt(5.0, 13.5), pt(11.0, 13.5)], stroke);
        }
        Icon::File => {
            let pts = vec![
                pt(4.0, 2.0),
                pt(9.5, 2.0),
                pt(12.5, 5.0),
                pt(12.5, 14.0),
                pt(4.0, 14.0),
            ];
            p.add(egui::Shape::closed_line(pts, stroke));
            p.add(egui::Shape::line(
                vec![pt(9.5, 2.0), pt(9.5, 5.0), pt(12.5, 5.0)],
                stroke,
            ));
        }
    }
}

/// Bordered chip: icon + text. Returns its right edge.
pub(super) fn chip(
    p: &egui::Painter,
    x: f32,
    cy: f32,
    icon: Icon,
    text: &str,
    color: Color32,
    strong: bool,
) -> f32 {
    let font = if strong {
        bold(SMALL_SIZE)
    } else {
        mono(SMALL_SIZE)
    };
    let g = p.layout_no_wrap(text.to_string(), font, color);
    let w = 7.0 + 12.0 + 6.0 + g.size().x + 7.0;
    let r = Rect::from_min_size(Pos2::new(x, cy - 11.0), Vec2::new(w, 22.0));
    p.rect(
        r,
        CornerRadius::same(5),
        CHIP_BG,
        Stroke::new(1.0, RULE),
        egui::StrokeKind::Inside,
    );
    paint_icon(p, Pos2::new(x + 7.0, cy - 6.0), icon, color);
    p.galley(Pos2::new(x + 25.0, cy - g.size().y / 2.0), g, color);
    r.right()
}

/// Changes chip: `3 • +48 -12` with numbers in green and red.
pub(super) fn diff_chip(p: &egui::Painter, x: f32, cy: f32, g: crate::git::DiffStat) -> f32 {
    let mut job = LayoutJob::default();
    let f = |color, strong: bool| TextFormat {
        font_id: if strong {
            bold(SMALL_SIZE)
        } else {
            mono(SMALL_SIZE)
        },
        color,
        ..Default::default()
    };
    job.append(&g.files.to_string(), 0.0, f(CHIP_TEXT, false));
    job.append("•", 6.0, f(DIM, false));
    job.append(&format!("+{}", g.insertions), 6.0, f(GREEN, true));
    job.append(&format!("-{}", g.deletions), 6.0, f(RED, true));
    let gal = p.layout_job(job);
    let w = 7.0 + 12.0 + 6.0 + gal.size().x + 7.0;
    let r = Rect::from_min_size(Pos2::new(x, cy - 11.0), Vec2::new(w, 22.0));
    p.rect(
        r,
        CornerRadius::same(5),
        CHIP_BG,
        Stroke::new(1.0, RULE),
        egui::StrokeKind::Inside,
    );
    paint_icon(p, Pos2::new(x + 7.0, cy - 6.0), Icon::File, CHIP_TEXT);
    p.galley(Pos2::new(x + 25.0, cy - gal.size().y / 2.0), gal, CHIP_TEXT);
    r.right()
}

/// Display columns of `chars` (wide chars count double), as a line input
/// lays them out.
pub(super) fn cols_of(chars: &[char]) -> f32 {
    chars.iter().map(|c| c.width().unwrap_or(0)).sum::<usize>() as f32
}

/// Selection of a line input, painted behind its text. `line` is where the
/// visible text goes (left edge, height and right limit); `cols` is the
/// width the text was laid out for.
pub(super) fn paint_selection(
    p: &egui::Painter,
    input: &LineInput,
    cols: usize,
    line: Rect,
    cell_w: f32,
) {
    let Some((a, b)) = input.selection() else {
        return;
    };
    let chars: Vec<char> = input.text.chars().collect();
    let start = input.first_visible(cols);
    let (a, b) = (a.max(start), b.min(chars.len()));
    if b <= a {
        return;
    }
    let x0 = line.left() + cols_of(&chars[start..a]) * cell_w;
    let x1 = (line.left() + cols_of(&chars[start..b]) * cell_w).min(line.right());
    if x1 > x0 {
        p.rect_filled(
            Rect::from_min_max(Pos2::new(x0, line.top()), Pos2::new(x1, line.bottom())),
            CornerRadius::same(2),
            TEXT_SELECTION,
        );
    }
}

/// Exponent of the superellipse used for the corners. Above 2 the curvature
/// falls to zero where the corner meets the straight edge, so the two join
/// with continuous curvature (G2) instead of the sudden jump of a circular arc.
const SQUIRCLE_N: f32 = 5.0;
/// A continuous corner starts further from the vertex than a circular one of
/// the same nominal radius, so that it looks about as round.
const SQUIRCLE_REACH: f32 = 1.6;
/// Points per corner.
const SQUIRCLE_STEPS: usize = 16;

/// Outline of `rect` with continuous-curvature corners ("squircle") of
/// nominal radius `radius`, clockwise from the top-left corner. Convex, so
/// it can be filled as a convex polygon.
pub(super) fn squircle(rect: Rect, radius: f32) -> Vec<Pos2> {
    let e = (radius * SQUIRCLE_REACH)
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    // A quarter of the superellipse |x|^n + |y|^n = 1, from (1, 0) to (0, 1).
    let quarter: Vec<Vec2> = (0..=SQUIRCLE_STEPS)
        .map(|i| {
            let t = i as f32 / SQUIRCLE_STEPS as f32 * std::f32::consts::FRAC_PI_2;
            let k = 2.0 / SQUIRCLE_N;
            // cos(π/2) comes out a hair below zero, and a fractional power
            // of a negative number is NaN.
            Vec2::new(t.cos().max(0.0).powf(k), t.sin().max(0.0).powf(k))
        })
        .collect();
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    // Corner centre, signs of the offset and whether to walk the quarter
    // backwards, so the outline goes round clockwise.
    let corners = [
        (Pos2::new(l + e, t + e), Vec2::new(-1.0, -1.0), false),
        (Pos2::new(r - e, t + e), Vec2::new(1.0, -1.0), true),
        (Pos2::new(r - e, b - e), Vec2::new(1.0, 1.0), false),
        (Pos2::new(l + e, b - e), Vec2::new(-1.0, 1.0), true),
    ];
    let mut points = Vec::with_capacity(4 * (SQUIRCLE_STEPS + 1));
    for (c, s, back) in corners {
        let at = |q: &Vec2| c + Vec2::new(s.x * q.x, s.y * q.y) * e;
        if back {
            points.extend(quarter.iter().rev().map(at));
        } else {
            points.extend(quarter.iter().map(at));
        }
    }
    points
}

/// Activity indicator: a star that grows and shrinks.
const STAR: [&str; 10] = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];
/// Time each frame of the star stays on screen.
const STAR_FRAME_SECS: f64 = 0.12;

/// Frame of the star at `time` seconds (egui's clock), so every indicator on
/// screen beats in step.
pub(super) fn star_frame(time: f64) -> &'static str {
    STAR[(time / STAR_FRAME_SECS).max(0.0) as usize % STAR.len()]
}

/// Paints the star centred on `center`, with glyphs of `size` points.
pub(super) fn paint_star(p: &egui::Painter, center: Pos2, size: f32, color: Color32, time: f64) {
    p.text(
        center,
        Align2::CENTER_CENTER,
        star_frame(time),
        mono(size),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_squircle_stays_in_its_rect_and_touches_every_edge() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(300.0, 120.0));
        let pts = squircle(rect, 12.0);
        let eps = 1e-3;
        assert!(pts.iter().all(|p| rect.expand(eps).contains(*p)));
        for touches in [
            |p: &Pos2, r: &Rect| (p.x - r.left()).abs() < 1e-3,
            |p: &Pos2, r: &Rect| (p.x - r.right()).abs() < 1e-3,
            |p: &Pos2, r: &Rect| (p.y - r.top()).abs() < 1e-3,
            |p: &Pos2, r: &Rect| (p.y - r.bottom()).abs() < 1e-3,
        ] {
            assert!(pts.iter().any(|p| touches(p, &rect)));
        }
    }

    #[test]
    fn a_squircle_is_convex_and_clockwise() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 80.0));
        let pts = squircle(rect, 12.0);
        let n = pts.len();
        for i in 0..n {
            let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
            let cross = (b - a).x * (c - b).y - (b - a).y * (c - b).x;
            // Clockwise on screen (y down) turns right: cross >= 0.
            assert!(cross >= -1e-3, "turns the wrong way at {i}");
        }
    }

    #[test]
    fn the_corner_is_flatter_than_a_circle_where_it_meets_the_edge() {
        // Right after leaving the top edge, a circular corner of the same
        // reach has already dropped more: the squircle eases into the curve.
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 400.0));
        let pts = squircle(rect, 20.0);
        let e = 20.0 * SQUIRCLE_REACH;
        let p = pts[SQUIRCLE_STEPS + 1 + 2]; // top-right corner, near the top edge
        let dx = p.x - (400.0 - e);
        let circle_drop = e - (e * e - dx * dx).sqrt();
        assert!(p.y < circle_drop, "{} vs {circle_drop}", p.y);
    }

    #[test]
    fn the_star_grows_shrinks_and_starts_over() {
        assert_eq!(star_frame(0.0), "·");
        assert_eq!(star_frame(0.13), "✢");
        assert_eq!(star_frame(5.0 * STAR_FRAME_SECS + 0.01), "✽");
        assert_eq!(star_frame(10.0 * STAR_FRAME_SECS + 0.01), "·");
        assert_eq!(star_frame(-1.0), "·");
    }

    #[test]
    fn a_tiny_rect_is_not_turned_inside_out() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(10.0, 6.0));
        assert!(
            squircle(rect, 12.0)
                .iter()
                .all(|p| rect.expand(1e-3).contains(*p))
        );
    }
}
