//! Terminal text as egui text: ANSI colors and ratatui styles turned into
//! `TextFormat`s and `LayoutJob`s, plus the path shortener the panels share.

use eframe::egui::{Color32, Stroke, TextFormat, text::LayoutJob};
use ratatui::style::{Color, Modifier, Style};

use crate::term::output::Cell;

use super::theme::{ANSI, BG, FG, TEXT_SIZE, bold, mono};

pub(super) fn color32(c: Color, default: Color32) -> Color32 {
    match c {
        Color::Reset => default,
        Color::Black => ANSI[0],
        Color::Red => ANSI[1],
        Color::Green => ANSI[2],
        Color::Yellow => ANSI[3],
        Color::Blue => ANSI[4],
        Color::Magenta => ANSI[5],
        Color::Cyan => ANSI[6],
        Color::Gray => ANSI[7],
        Color::DarkGray => ANSI[8],
        Color::LightRed => ANSI[9],
        Color::LightGreen => ANSI[10],
        Color::LightYellow => ANSI[11],
        Color::LightBlue => ANSI[12],
        Color::LightMagenta => ANSI[13],
        Color::LightCyan => ANSI[14],
        Color::White => ANSI[15],
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        Color::Indexed(i) => xterm256(i),
    }
}

pub(super) fn xterm256(i: u8) -> Color32 {
    match i {
        0..=15 => ANSI[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

/// egui format for a ratatui style (block output).
pub(super) fn text_format(style: Style) -> TextFormat {
    let m = style.add_modifier;
    let mut fg = color32(style.fg.unwrap_or(Color::Reset), FG);
    let mut bg = style
        .bg
        .map_or(Color32::TRANSPARENT, |c| color32(c, Color32::TRANSPARENT));
    if m.contains(Modifier::REVERSED) {
        let back = if bg == Color32::TRANSPARENT { BG } else { bg };
        bg = fg;
        fg = back;
    }
    if m.contains(Modifier::DIM) {
        fg = fg.gamma_multiply(0.6);
    }
    if m.contains(Modifier::HIDDEN) {
        fg = Color32::TRANSPARENT;
    }
    let line = |on: bool| {
        if on {
            Stroke::new(1.0, fg)
        } else {
            Stroke::NONE
        }
    };
    TextFormat {
        font_id: if m.contains(Modifier::BOLD) {
            bold(TEXT_SIZE)
        } else {
            mono(TEXT_SIZE)
        },
        color: fg,
        background: bg,
        italics: m.contains(Modifier::ITALIC),
        underline: line(m.contains(Modifier::UNDERLINED)),
        strikethrough: line(m.contains(Modifier::CROSSED_OUT)),
        ..Default::default()
    }
}

pub(super) fn cells_job(cells: &[Cell]) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut style = cells.first().map(|c| c.style).unwrap_or_default();
    for c in cells {
        if c.style != style {
            job.append(&run, 0.0, text_format(style));
            run.clear();
            style = c.style;
        }
        run.push(c.ch);
    }
    if !run.is_empty() {
        job.append(&run, 0.0, text_format(style));
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

/// Shortens a path from the left: `…/scratchpad/demo-repo`.
pub(super) fn shorten_path(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
        return path.to_string();
    }
    let mut out = String::new();
    for seg in path.rsplit('/') {
        let candidate = if out.is_empty() {
            seg.to_string()
        } else {
            format!("{seg}/{out}")
        };
        if candidate.chars().count() + 2 > max_chars {
            break;
        }
        out = candidate;
    }
    if out.is_empty() {
        let tail: String = path
            .chars()
            .rev()
            .take(max_chars.saturating_sub(1))
            .collect();
        return format!("…{}", tail.chars().rev().collect::<String>());
    }
    format!("…/{out}")
}
