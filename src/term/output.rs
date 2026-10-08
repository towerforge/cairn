//! Block output: a line-oriented ANSI interpreter.
//!
//! Not a full emulator (`vt100` handles full-screen programs). It keeps a list
//! of styled lines and understands what normal CLI output uses: SGR colors,
//! `\r`, line erase and relative cursor moves, so progress bars (cargo,
//! docker, npm) redraw in place instead of filling the block.

use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;

/// Line limit per block; the oldest are discarded.
const MAX_LINES: usize = 20_000;

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

const BLANK: Cell = Cell {
    ch: ' ',
    style: Style::new(),
};

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Esc,
    EscSkip,
    Csi,
    Str,
    StrEsc,
}

pub struct Output {
    pub lines: Vec<Vec<Cell>>,
    pub row: usize,
    pub col: usize,
    style: Style,
    state: State,
    csi: Vec<u8>,
    utf8: Vec<u8>,
    /// A program is using the alternate screen: its output is not part of the block.
    alt: bool,
    /// Lines discarded by the limit.
    pub dropped: usize,
    /// Bumped on every change (invalidates the height cache).
    pub version: u64,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            lines: vec![Vec::new()],
            row: 0,
            col: 0,
            style: Style::new(),
            state: State::Ground,
            csi: Vec::new(),
            utf8: Vec::new(),
            alt: false,
            dropped: 0,
            version: 0,
        }
    }
}

impl Output {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.version += 1;
        for &b in bytes {
            match self.state {
                State::Ground => self.ground(b),
                State::Esc => {
                    self.state = match b {
                        b'[' => {
                            self.csi.clear();
                            State::Csi
                        }
                        // OSC, DCS, SOS, PM, APC: strings up to BEL / ST.
                        b']' | b'P' | b'X' | b'^' | b'_' => State::Str,
                        // Character set selection: one more byte.
                        b'(' | b')' | b'*' | b'+' | b'#' | b'%' => State::EscSkip,
                        _ => State::Ground,
                    }
                }
                State::EscSkip => self.state = State::Ground,
                State::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.state = State::Ground;
                        self.csi_dispatch(b);
                    } else if self.csi.len() < 64 {
                        self.csi.push(b);
                    }
                }
                State::Str => match b {
                    0x07 => self.state = State::Ground,
                    0x1b => self.state = State::StrEsc,
                    _ => {}
                },
                State::StrEsc => self.state = State::Ground,
            }
        }
    }

    fn ground(&mut self, b: u8) {
        if !self.utf8.is_empty() || b >= 0x80 {
            self.utf8_byte(b);
            return;
        }
        match b {
            0x1b => self.state = State::Esc,
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1),
            b'\t' => {
                let next = (self.col / 8 + 1) * 8;
                while self.col < next {
                    self.put(' ');
                }
            }
            0x20..=0x7e => self.put(b as char),
            _ => {} // BEL and other controls
        }
    }

    fn utf8_byte(&mut self, b: u8) {
        if b < 0x80 {
            // Truncated sequence: discard it and process the ASCII byte.
            self.utf8.clear();
            self.put('\u{FFFD}');
            self.ground(b);
            return;
        }
        self.utf8.push(b);
        let need = match self.utf8[0] {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if self.utf8.len() >= need {
            let c = std::str::from_utf8(&self.utf8)
                .ok()
                .and_then(|s| s.chars().next())
                .unwrap_or('\u{FFFD}');
            self.utf8.clear();
            self.put(c);
        }
    }

    fn put(&mut self, c: char) {
        if self.alt {
            return;
        }
        // Zero-width characters (combining marks, variation selectors): dropped.
        if c.width().unwrap_or(0) == 0 {
            return;
        }
        let style = self.style;
        let col = self.col;
        let line = &mut self.lines[self.row];
        if col < line.len() {
            line[col] = Cell { ch: c, style };
        } else {
            line.resize(col, BLANK);
            line.push(Cell { ch: c, style });
        }
        self.col += 1;
    }

    fn newline(&mut self) {
        if self.alt {
            return;
        }
        self.row += 1;
        self.col = 0;
        self.ensure_row();
        if self.lines.len() > MAX_LINES {
            let n = self.lines.len() - MAX_LINES;
            self.lines.drain(..n);
            self.row -= n;
            self.dropped += n;
        }
    }

    fn ensure_row(&mut self) {
        while self.lines.len() <= self.row {
            self.lines.push(Vec::new());
        }
    }

    fn csi_dispatch(&mut self, final_byte: u8) {
        let raw = String::from_utf8_lossy(&self.csi).into_owned();
        if let Some(private) = raw.strip_prefix('?') {
            // Only the alternate screen matters here.
            if final_byte == b'h' || final_byte == b'l' {
                for p in private.split(';') {
                    if matches!(p, "1049" | "1047" | "47") {
                        self.alt = final_byte == b'h';
                    }
                }
            }
            return;
        }
        if self.alt || raw.starts_with(['>', '<', '=']) {
            return;
        }
        let nums: Vec<usize> = raw
            .split([';', ':'])
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        let n = nums.first().copied().unwrap_or(0).max(1);
        match final_byte {
            b'm' => self.sgr(&nums),
            b'K' => {
                let line = &mut self.lines[self.row];
                match nums.first().copied().unwrap_or(0) {
                    0 => line.truncate(self.col),
                    1 => {
                        for c in line.iter_mut().take(self.col + 1) {
                            *c = BLANK;
                        }
                    }
                    _ => line.clear(),
                }
            }
            b'J' => match nums.first().copied().unwrap_or(0) {
                0 => {
                    self.lines[self.row].truncate(self.col);
                    self.lines.truncate(self.row + 1);
                }
                _ => {
                    self.lines = vec![Vec::new()];
                    self.row = 0;
                    self.col = 0;
                }
            },
            b'G' | b'`' => self.col = n - 1,
            b'C' => self.col += n,
            b'D' => self.col = self.col.saturating_sub(n),
            b'A' => self.row = self.row.saturating_sub(n),
            b'B' => {
                self.row += n;
                self.ensure_row();
            }
            b'E' => {
                self.row += n;
                self.col = 0;
                self.ensure_row();
            }
            b'F' => {
                self.row = self.row.saturating_sub(n);
                self.col = 0;
            }
            // Absolute position: no screen of our own, only the column is honoured.
            b'H' | b'f' => self.col = nums.get(1).copied().unwrap_or(1).max(1) - 1,
            b'X' => {
                let line = &mut self.lines[self.row];
                for i in self.col..(self.col + n).min(line.len()) {
                    line[i] = BLANK;
                }
            }
            b'P' => {
                let line = &mut self.lines[self.row];
                if self.col < line.len() {
                    let end = (self.col + n).min(line.len());
                    line.drain(self.col..end);
                }
            }
            b'@' => {
                let line = &mut self.lines[self.row];
                if self.col < line.len() {
                    for _ in 0..n {
                        line.insert(self.col, BLANK);
                    }
                }
            }
            _ => {}
        }
    }

    fn sgr(&mut self, nums: &[usize]) {
        if nums.is_empty() {
            self.style = Style::new();
            return;
        }
        let mut i = 0;
        while i < nums.len() {
            let s = self.style;
            self.style = match nums[i] {
                0 => Style::new(),
                1 => s.add_modifier(Modifier::BOLD),
                2 => s.add_modifier(Modifier::DIM),
                3 => s.add_modifier(Modifier::ITALIC),
                4 => s.add_modifier(Modifier::UNDERLINED),
                5 | 6 => s.add_modifier(Modifier::SLOW_BLINK),
                7 => s.add_modifier(Modifier::REVERSED),
                8 => s.add_modifier(Modifier::HIDDEN),
                9 => s.add_modifier(Modifier::CROSSED_OUT),
                21 | 22 => s.remove_modifier(Modifier::BOLD | Modifier::DIM),
                23 => s.remove_modifier(Modifier::ITALIC),
                24 => s.remove_modifier(Modifier::UNDERLINED),
                25 => s.remove_modifier(Modifier::SLOW_BLINK),
                27 => s.remove_modifier(Modifier::REVERSED),
                28 => s.remove_modifier(Modifier::HIDDEN),
                29 => s.remove_modifier(Modifier::CROSSED_OUT),
                n @ 30..=37 => s.fg(ansi_color((n - 30) as u8)),
                n @ 40..=47 => s.bg(ansi_color((n - 40) as u8)),
                n @ 90..=97 => s.fg(ansi_color((n - 90 + 8) as u8)),
                n @ 100..=107 => s.bg(ansi_color((n - 100 + 8) as u8)),
                39 => s.fg(Color::Reset),
                49 => s.bg(Color::Reset),
                n @ (38 | 48) => {
                    let (color, used) = extended_color(&nums[i + 1..]);
                    i += used;
                    match color {
                        Some(c) if n == 38 => s.fg(c),
                        Some(c) => s.bg(c),
                        None => s,
                    }
                }
                _ => s,
            };
            i += 1;
        }
    }

    /// Lines to show: without trailing blank ones (except the cursor's if the
    /// command is still running and writing to it).
    pub fn visible_len(&self, running: bool) -> usize {
        let blank = |l: &Vec<Cell>| l.iter().all(|c| c.ch == ' ' && c.style == Style::new());
        let mut n = self.lines.len();
        while n > 0 && blank(&self.lines[n - 1]) && !(running && n - 1 == self.row && self.col > 0)
        {
            n -= 1;
        }
        n
    }

    /// Removes the first line if its text is exactly `text`.
    pub fn drop_first_line_if(&mut self, text: &str) {
        let first: String = self.lines[0].iter().map(|c| c.ch).collect();
        if self.row > 0 && first.trim_end() == text {
            self.lines.remove(0);
            self.row -= 1;
            self.version += 1;
        }
    }

    /// Plain text of the output (for copying).
    pub fn plain_text(&self) -> String {
        let n = self.visible_len(false);
        self.lines[..n]
            .iter()
            .map(|l| {
                l.iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Segments of a line wrapped at width `wrap` (cell indices).
pub fn segments(line: &[Cell], wrap: usize) -> Vec<(usize, usize)> {
    let mut segs = Vec::new();
    let (mut start, mut width) = (0, 0);
    for (i, c) in line.iter().enumerate() {
        let cw = c.ch.width().unwrap_or(1);
        if width + cw > wrap && i > start {
            segs.push((start, i));
            start = i;
            width = 0;
        }
        width += cw;
    }
    segs.push((start, line.len()));
    segs
}

pub fn line_rows(line: &[Cell], wrap: usize) -> usize {
    let width: usize = line.iter().map(|c| c.ch.width().unwrap_or(1)).sum();
    width.div_ceil(wrap).max(1)
}

/// Color from the 256 palette. The first 16 are returned as named colors so
/// they follow the theme (the window's or the terminal's).
pub fn ansi_color(i: u8) -> Color {
    const NAMED: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    NAMED.get(i as usize).copied().unwrap_or(Color::Indexed(i))
}

/// `38;5;n` or `38;2;r;g;b`. Returns the color and how many params it consumed.
fn extended_color(rest: &[usize]) -> (Option<Color>, usize) {
    match rest.first() {
        Some(5) => (rest.get(1).map(|&n| ansi_color(n as u8)), 2),
        Some(2) if rest.len() >= 4 => (
            Some(Color::Rgb(rest[1] as u8, rest[2] as u8, rest[3] as u8)),
            4,
        ),
        Some(_) => (None, 1),
        None => (None, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(o: &Output) -> Vec<String> {
        o.lines[..o.visible_len(false)]
            .iter()
            .map(|l| l.iter().map(|c| c.ch).collect())
            .collect()
    }

    #[test]
    fn carriage_return_overwrites() {
        let mut o = Output::default();
        o.feed(b"progress 10%\rprogress 100%\r\ndone\r\n");
        assert_eq!(text(&o), ["progress 100%", "done"]);
    }

    #[test]
    fn erase_line_and_move_up() {
        let mut o = Output::default();
        o.feed(b"a 1\nb 1\n\x1b[2A\x1b[2Ka 2\n\x1b[2Kb 2\n");
        assert_eq!(text(&o), ["a 2", "b 2"]);
    }

    #[test]
    fn sgr_colors() {
        let mut o = Output::default();
        o.feed(b"\x1b[1;31mx\x1b[0m\x1b[38;2;1;2;3my\x1b[38;5;200mz");
        let l = &o.lines[0];
        assert_eq!(l[0].style.fg, Some(Color::Red));
        assert!(l[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(l[1].style.fg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(l[2].style.fg, Some(Color::Indexed(200)));
    }

    #[test]
    fn split_utf8() {
        let mut o = Output::default();
        let s = "über".as_bytes();
        o.feed(&s[..2]);
        o.feed(&s[2..]);
        assert_eq!(text(&o), ["über"]);
    }

    #[test]
    fn alternate_screen_stays_out_of_block() {
        let mut o = Output::default();
        o.feed(b"before\r\n\x1b[?1049h\x1b[Hvim garbage\x1b[?1049lafter\r\n");
        assert_eq!(text(&o), ["before", "after"]);
    }
}
