//! Block output: a line-oriented ANSI interpreter.
//!
//! Not a full emulator (`vt100` handles full-screen programs). It keeps a list
//! of styled lines and understands what normal CLI output uses: SGR colors,
//! `\r`, erasing and cursor moves. The last `rows` lines (the PTY height) are
//! the screen the program addresses; the lines above are scrollback. That is
//! enough for anything that redraws in place to replace its old output instead
//! of piling up in the block: progress bars (cargo, docker, npm), tables
//! refreshed with "clear screen and home" (docker stats, top), a status line
//! kept at the bottom with a scroll region (apt), or a screen cleared with
//! "home and erase below" (vite).

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
    /// Screen height, as the program sees it: the last `rows` lines.
    rows: usize,
    style: Style,
    state: State,
    csi: Vec<u8>,
    utf8: Vec<u8>,
    /// A program is using the alternate screen: its output is not part of the block.
    alt: bool,
    /// Scroll region (first and last screen row, inclusive) when it is not
    /// the whole screen.
    region: Option<(usize, usize)>,
    /// Cursor saved with `ESC 7` / `CSI s`: screen row, column and style.
    saved: Option<(usize, usize, Style)>,
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
            rows: 24,
            style: Style::new(),
            state: State::Ground,
            csi: Vec::new(),
            utf8: Vec::new(),
            alt: false,
            region: None,
            saved: None,
            dropped: 0,
            version: 0,
        }
    }
}

impl Output {
    /// The PTY height: what the program believes the screen is.
    pub fn set_rows(&mut self, rows: usize) {
        let rows = rows.max(1);
        if rows == self.rows {
            return;
        }
        self.rows = rows;
        if self.region.is_some_and(|(_, bottom)| bottom >= rows) {
            self.region = None;
        }
        self.row = self.row.max(self.top());
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.version += 1;
        for &b in bytes {
            match self.state {
                State::Ground => self.ground(b),
                State::Esc => {
                    self.state = State::Ground;
                    match b {
                        b'[' => {
                            self.csi.clear();
                            self.state = State::Csi;
                        }
                        // OSC, DCS, SOS, PM, APC: strings up to BEL / ST.
                        b']' | b'P' | b'X' | b'^' | b'_' => self.state = State::Str,
                        // Character set selection: one more byte.
                        b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.state = State::EscSkip,
                        b'7' => self.save_cursor(),
                        b'8' => self.restore_cursor(true),
                        b'D' => self.index(),
                        b'E' => self.newline(),
                        b'M' => self.reverse_index(),
                        b'c' => self.reset(),
                        _ => {}
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

    // ── The screen ───────────────────────────────────────────────────────────
    //
    // The screen is the last `rows` lines. Moving the cursor never scrolls
    // (it is clamped to the screen); only a line feed at the bottom of the
    // scroll region does, and then the line that leaves the screen stays
    // above it as scrollback.

    /// First line of the screen.
    fn top(&self) -> usize {
        self.lines.len().saturating_sub(self.rows)
    }

    /// Last line of the screen (it may not exist yet).
    fn bottom(&self) -> usize {
        self.top() + self.rows - 1
    }

    /// Scroll region as line indexes: the whole screen unless one was set.
    fn region(&self) -> (usize, usize) {
        let top = self.top();
        match self.region {
            Some((a, b)) => (top + a, top + b),
            None => (top, top + self.rows - 1),
        }
    }

    fn ensure_row(&mut self) {
        self.ensure_line(self.row);
    }

    fn ensure_line(&mut self, i: usize) {
        while self.lines.len() <= i {
            self.lines.push(Vec::new());
        }
    }

    /// Keeps the block under `MAX_LINES`, dropping the oldest scrollback.
    fn trim(&mut self) {
        if self.lines.len() > MAX_LINES {
            let n = self.lines.len() - MAX_LINES;
            self.lines.drain(..n);
            self.row = self.row.saturating_sub(n);
            self.dropped += n;
        }
    }

    /// Screen position `(row, col)`, zero-based.
    fn goto(&mut self, row: usize, col: usize) {
        self.row = self.top() + row.min(self.rows - 1);
        self.col = col;
        self.ensure_row();
    }

    /// Up `n` rows, stopping at the scroll region (if inside it) or the screen.
    fn cursor_up(&mut self, n: usize) {
        let (rtop, _) = self.region();
        let limit = if self.row >= rtop { rtop } else { self.top() };
        self.row = self.row.saturating_sub(n).max(limit);
    }

    /// Down `n` rows, stopping at the scroll region (if inside it) or the screen.
    fn cursor_down(&mut self, n: usize) {
        let (_, rbot) = self.region();
        let limit = if self.row <= rbot {
            rbot
        } else {
            self.bottom()
        };
        self.row = (self.row + n).min(limit);
        self.ensure_row();
    }

    /// Line feed: down one row; at the bottom of the region, scroll it.
    fn index(&mut self) {
        if self.alt {
            return;
        }
        let (rtop, rbot) = self.region();
        if self.row == rbot {
            self.scroll_up(rtop, rbot, 1, true);
        } else if self.row < self.bottom() {
            self.row += 1;
            self.ensure_row();
        }
    }

    fn newline(&mut self) {
        self.index();
        self.col = 0;
    }

    /// Up one row; at the top of the region, scroll it the other way.
    fn reverse_index(&mut self) {
        if self.alt {
            return;
        }
        let (rtop, rbot) = self.region();
        if self.row == rtop {
            self.scroll_down(rtop, rbot, 1);
        } else if self.row > self.top() {
            self.row -= 1;
        }
    }

    /// Moves the lines of `rtop..=rbot` up by `n`: the top ones leave, blank
    /// ones come in at the bottom. With `keep`, lines leaving a region that
    /// starts at the top of the screen are kept above it as scrollback (as a
    /// terminal does); otherwise they are lost.
    fn scroll_up(&mut self, rtop: usize, rbot: usize, n: usize, keep: bool) {
        let n = n.min(rbot + 1 - rtop);
        self.ensure_line(rbot);
        if keep && rtop == self.top() {
            for _ in 0..n {
                self.lines.insert(rbot + 1, Vec::new());
            }
            // The screen moved down `n` lines; the cursor keeps its screen row.
            self.row += n;
            self.trim();
        } else {
            for _ in 0..n {
                self.lines.remove(rtop);
                self.lines.insert(rbot, Vec::new());
            }
        }
    }

    /// Moves the lines of `rtop..=rbot` down by `n`: blank ones come in at
    /// the top, the bottom ones are lost.
    fn scroll_down(&mut self, rtop: usize, rbot: usize, n: usize) {
        let n = n.min(rbot + 1 - rtop);
        self.ensure_line(rbot);
        for _ in 0..n {
            self.lines.remove(rbot);
            self.lines.insert(rtop, Vec::new());
        }
    }

    fn save_cursor(&mut self) {
        if !self.alt {
            self.saved = Some((self.row - self.top(), self.col, self.style));
        }
    }

    fn restore_cursor(&mut self, with_style: bool) {
        if self.alt {
            return;
        }
        let Some((row, col, style)) = self.saved else {
            self.goto(0, 0);
            return;
        };
        self.goto(row, col);
        if with_style {
            self.style = style;
        }
    }

    /// The block starts over: what the program wants is a blank screen.
    fn wipe(&mut self) {
        self.lines = vec![Vec::new()];
        self.row = 0;
        self.col = 0;
    }

    /// `ESC c`: full reset.
    fn reset(&mut self) {
        if self.alt {
            return;
        }
        self.wipe();
        self.style = Style::new();
        self.region = None;
        self.saved = None;
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
        let first = nums.first().copied().unwrap_or(0);
        let n = first.max(1);
        let second = nums.get(1).copied().unwrap_or(0);
        match final_byte {
            b'm' => self.sgr(&nums),
            b'K' => {
                let line = &mut self.lines[self.row];
                match first {
                    0 => line.truncate(self.col),
                    1 => {
                        for c in line.iter_mut().take(self.col + 1) {
                            *c = BLANK;
                        }
                    }
                    _ => line.clear(),
                }
            }
            b'J' => match first {
                // Erase below from the home position: the whole screen.
                0 if self.row == self.top() && self.col == 0 => self.wipe(),
                0 => {
                    self.lines[self.row].truncate(self.col);
                    self.lines.truncate(self.row + 1);
                }
                1 => {
                    let top = self.top();
                    for line in &mut self.lines[top..self.row] {
                        line.clear();
                    }
                    for c in self.lines[self.row].iter_mut().take(self.col + 1) {
                        *c = BLANK;
                    }
                }
                _ => self.wipe(),
            },
            b'G' | b'`' => self.col = n - 1,
            b'C' => self.col += n,
            b'D' => self.col = self.col.saturating_sub(n),
            b'A' => self.cursor_up(n),
            b'B' => self.cursor_down(n),
            b'E' => {
                self.cursor_down(n);
                self.col = 0;
            }
            b'F' => {
                self.cursor_up(n);
                self.col = 0;
            }
            b'H' | b'f' => self.goto(n - 1, second.max(1) - 1),
            b'd' => self.goto(n - 1, self.col),
            // Insert / delete lines at the cursor, within the scroll region.
            b'L' | b'M' => {
                let (rtop, rbot) = self.region();
                if (rtop..=rbot).contains(&self.row) {
                    if final_byte == b'L' {
                        self.scroll_down(self.row, rbot, n);
                    } else {
                        self.scroll_up(self.row, rbot, n, false);
                    }
                }
            }
            b'S' => {
                let (rtop, rbot) = self.region();
                self.scroll_up(rtop, rbot, n, true);
            }
            b'T' => {
                let (rtop, rbot) = self.region();
                self.scroll_down(rtop, rbot, n);
            }
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
            // Scroll region (1-based, inclusive); the cursor goes home.
            b'r' => {
                let top = n;
                let bottom = if second == 0 { self.rows } else { second };
                if top < bottom && bottom <= self.rows {
                    self.region = (top > 1 || bottom < self.rows).then_some((top - 1, bottom - 1));
                    self.goto(0, 0);
                }
            }
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(false),
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

    #[test]
    fn clear_screen_and_home_redraws_in_place() {
        // docker stats, top: every refresh starts with ESC[2J ESC[H.
        let mut o = Output::default();
        o.feed(b"NAME  CPU\r\nweb   1%\r\n");
        o.feed(b"\x1b[2J\x1b[HNAME  CPU\r\nweb   2%\r\n");
        o.feed(b"\x1b[2J\x1b[HNAME  CPU\r\nweb   3%\r\n");
        assert_eq!(text(&o), ["NAME  CPU", "web   3%"]);
    }

    #[test]
    fn docker_stats_refresh_redraws_in_place() {
        // What `docker stats` really sends every refresh (captured through a
        // pty): home, every line rewritten and erased to its end, then erase
        // below. No clear screen at all.
        let mut o = Output::default();
        let refresh = |o: &mut Output, cpu: &str| {
            o.feed(b"\x1b[HCONTAINER   CPU %\x1b[K\r\n");
            o.feed(format!("web         {cpu}\x1b[K\r\n").as_bytes());
            o.feed(b"db          0.1%\x1b[K\r\n\x1b[J");
        };
        for cpu in ["1.5%", "1.6%", "1.4%"] {
            refresh(&mut o, cpu);
        }
        assert_eq!(
            text(&o),
            ["CONTAINER   CPU %", "web         1.4%", "db          0.1%"]
        );
        // A container goes away: the stale line is erased below.
        o.feed(b"\x1b[HCONTAINER   CPU %\x1b[K\r\nweb         1.3%\x1b[K\r\n\x1b[J");
        assert_eq!(text(&o), ["CONTAINER   CPU %", "web         1.3%"]);
    }

    #[test]
    fn home_and_overwrite_redraws_in_place() {
        // Home, rewrite every line and erase its tail: no clear at all.
        let mut o = Output::default();
        o.feed(b"CPU 10%\r\nMEM 20%\r\n");
        o.feed(b"\x1b[1;1HCPU 11%\x1b[K\r\nMEM 21%\x1b[K\r\n");
        o.feed(b"\x1b[1;1HCPU 9%\x1b[K\r\nMEM 22%\x1b[K\r\n");
        assert_eq!(text(&o), ["CPU 9%", "MEM 22%"]);
    }

    #[test]
    fn absolute_rows_address_the_screen_not_the_block() {
        let mut o = Output::default();
        o.set_rows(3);
        o.feed(b"l1\r\nl2\r\nl3\r\nl4\r\nl5\r\n");
        // Row 1 of the screen is the fourth line of the block.
        o.feed(b"\x1b[1;1HX");
        assert_eq!(text(&o), ["l1", "l2", "l3", "X4", "l5"]);
        // Row 3 of the screen is the line after it; row 10 is clamped there too.
        o.feed(b"\x1b[10;1HY");
        assert_eq!(text(&o), ["l1", "l2", "l3", "X4", "l5", "Y"]);
    }

    #[test]
    fn cursor_moves_stop_at_the_screen_edges() {
        let mut o = Output::default();
        o.set_rows(2);
        o.feed(b"a\r\nb\r\nc\r\nd");
        o.feed(b"\x1b[10A\rX");
        assert_eq!(text(&o), ["a", "b", "X", "d"]);
        o.feed(b"\x1b[10B\rY");
        assert_eq!(text(&o), ["a", "b", "X", "Y"]);
    }

    #[test]
    fn home_and_erase_below_clears_the_screen() {
        // vite: pads with newlines, goes home and erases below.
        let mut o = Output::default();
        o.set_rows(2);
        o.feed(b"a\r\nb\r\nc\r\n\x1b[1;1H\x1b[0Jnew\r\n");
        assert_eq!(text(&o), ["new"]);
    }

    #[test]
    fn scroll_region_keeps_a_status_line_at_the_bottom() {
        // apt: output scrolls in the top rows, the progress bar stays below.
        let mut o = Output::default();
        o.set_rows(4);
        o.feed(b"\x1b[1;3r\x1b[4;1HBAR\x1b[1;1Hl1\r\nl2\r\nl3\r\nl4\r\n");
        assert_eq!(text(&o), ["l1", "l2", "l3", "l4", "", "BAR"]);
        // Resetting the region: the next line feed at the bottom scrolls all.
        o.feed(b"\x1b[r\x1b[4;1HBAR 2\r\nend");
        assert_eq!(text(&o), ["l1", "l2", "l3", "l4", "", "BAR 2", "end"]);
    }

    #[test]
    fn save_and_restore_cursor() {
        let mut o = Output::default();
        o.feed(b"abc\x1b7\r\nxyz\x1b8X");
        assert_eq!(text(&o), ["abcX", "xyz"]);
        o.feed(b"\x1b[s\r\nq\x1b[uY");
        assert_eq!(text(&o), ["abcXY", "qyz"]);
    }

    #[test]
    fn insert_and_delete_lines() {
        let mut o = Output::default();
        o.set_rows(5);
        o.feed(b"a\r\nb\r\nc\x1b[1;1H\x1b[M");
        assert_eq!(text(&o), ["b", "c"]);
        o.feed(b"\x1b[L");
        assert_eq!(text(&o), ["", "b", "c"]);
    }

    #[test]
    fn index_and_reverse_index() {
        let mut o = Output::default();
        o.set_rows(2);
        o.feed(b"a\r\nb\x1bDc");
        assert_eq!(text(&o), ["a", "b", " c"]);
        // Up to the top of the screen, then once more: the screen scrolls
        // down and its last line (` c`) is lost, as in a terminal.
        o.feed(b"\x1bM\x1bMZ");
        assert_eq!(text(&o), ["a", "  Z", "b"]);
    }

    #[test]
    fn shrinking_the_screen_keeps_the_cursor_on_it() {
        let mut o = Output::default();
        o.set_rows(5);
        o.feed(b"a\r\nb\r\nc\r\nd\x1b[3A");
        o.set_rows(2);
        o.feed(b"\rX");
        assert_eq!(text(&o), ["a", "b", "X", "d"]);
    }
}
