//! PTY session: one live shell per tab.
//!
//! A thread reads the PTY and sends the bytes to the main loop. There the
//! `Scanner` separates the shell integration marks (OSC 133 / OSC 7) from the
//! rest of the output, which feeds both the `vt100` emulator (used when a
//! program takes over the full screen) and the running command's block.

use std::io::{Read, Write};

use portable_pty::{Child, MasterPty, PtySize, native_pty_system};

use crate::event::{AppEvent, EventSink};
use crate::term::shell::{self, ShellKind};

pub struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pub kind: ShellKind,
    /// Short shell name (`zsh`) for the context line.
    pub shell_label: String,
    scanner: Scanner,
    pub vt: vt100::Parser<Responder>,
    size: (u16, u16),
}

impl Session {
    pub fn spawn(
        shell_path: &str,
        cwd: Option<&str>,
        (rows, cols): (u16, u16),
        tab_id: u64,
        sink: EventSink,
    ) -> anyhow::Result<Self> {
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let (mut cmd, kind) = shell::command(shell_path)?;
        if let Some(dir) = cwd.or(std::env::var("HOME").ok().as_deref()) {
            cmd.cwd(dir);
        }
        let child = pair.slave.spawn_command(cmd)?;
        // The child keeps its copy of the slave; dropping ours lets us
        // receive EOF when the shell exits.
        drop(pair.slave);

        let writer = pair.master.take_writer()?;
        let mut reader = pair.master.try_clone_reader()?;
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if !sink(AppEvent::Pty(tab_id, buf[..n].to_vec())) {
                            return;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    // EIO on macOS/Linux when the shell closes the slave.
                    Err(_) => break,
                }
            }
            sink(AppEvent::PtyClosed(tab_id));
        });

        Ok(Self {
            writer,
            master: pair.master,
            child,
            kind,
            shell_label: shell::basename(shell_path).to_string(),
            scanner: Scanner::default(),
            // With scrollback: interactive sessions (ssh…) scroll with the wheel.
            vt: vt100::Parser::new_with_callbacks(rows, cols, 2000, Responder::default()),
            size: (rows, cols),
        })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.vt.screen_mut().set_size(rows, cols);
    }

    pub fn scan(&mut self, data: &[u8]) -> Vec<Chunk> {
        self.scanner.feed(data)
    }

    /// Feeds output to the emulator and answers any queries the program made
    /// (cursor position, device attributes).
    pub fn process_vt(&mut self, data: &[u8]) {
        self.vt.process(data);
        let replies = std::mem::take(&mut self.vt.callbacks_mut().replies);
        if !replies.is_empty() {
            self.write(&replies);
        }
    }

    /// The foreground program is using the alternate screen.
    pub fn fullscreen(&self) -> bool {
        self.vt.screen().alternate_screen()
    }

    /// Clears the screen when an interactive session (ssh, REPL…) starts, so
    /// the full terminal view shows only that session.
    pub fn clear_screen(&mut self) {
        self.vt.process(b"\x1b[0m\x1b[2J\x1b[H");
        self.vt.screen_mut().set_scrollback(0);
    }

    /// Resets the emulator if a program exited without leaving the alternate
    /// screen (e.g. killed with SIGKILL).
    pub fn reset_screen(&mut self) {
        if self.fullscreen() {
            self.vt.process(b"\x1b[?1049l");
        }
        self.vt.process(
            b"\x1b[0m\x1b[?25h\x1b[?1l\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l",
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// ── Replies to terminal queries ─────────────────────────────────────────────

#[derive(Default)]
pub struct Responder {
    replies: Vec<u8>,
}

impl vt100::Callbacks for Responder {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, first) {
            // DSR: cursor position.
            (None, 'n', 6) => {
                let (r, c) = screen.cursor_position();
                self.replies
                    .extend(format!("\x1b[{};{}R", r + 1, c + 1).bytes());
            }
            // DSR: terminal status.
            (None, 'n', 5) => self.replies.extend(b"\x1b[0n"),
            // DA1 / DA2: identify as a generic VT220 / xterm.
            (None, 'c', 0) => self.replies.extend(b"\x1b[?62;22c"),
            (Some(b'>'), 'c', 0) => self.replies.extend(b"\x1b[>0;276;0c"),
            _ => {}
        }
    }
}

// ── OSC 133 / OSC 7 mark scanner ────────────────────────────────────────────

pub enum Mark {
    /// `133;A`: the shell is waiting for a command.
    Prompt,
    /// `133;C`: command output starts.
    CommandStart,
    /// `133;D;<exit>`: the command has finished.
    CommandEnd(Option<i32>),
    /// `7;file://host/path`: current directory and machine (empty if not given).
    Cwd { host: String, path: String },
}

pub enum Chunk {
    Data(Vec<u8>),
    Mark(Mark),
}

#[derive(Default, PartialEq)]
enum ScanState {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
}

/// Extracts the shell integration OSCs from the byte stream. Everything else
/// (including other OSCs, such as the window title) passes through untouched.
/// Sequences split across reads are buffered until complete.
#[derive(Default)]
struct Scanner {
    state: ScanState,
    osc: Vec<u8>,
}

const MAX_OSC: usize = 8192;

impl Scanner {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Chunk> {
        let mut out = Vec::new();
        let mut data = Vec::with_capacity(bytes.len());
        for &b in bytes {
            match self.state {
                ScanState::Ground => {
                    if b == 0x1b {
                        self.state = ScanState::Esc;
                    } else {
                        data.push(b);
                    }
                }
                ScanState::Esc => {
                    if b == b']' {
                        self.state = ScanState::Osc;
                        self.osc.clear();
                    } else {
                        data.push(0x1b);
                        if b == 0x1b {
                            continue; // stay in Esc
                        }
                        data.push(b);
                        self.state = ScanState::Ground;
                    }
                }
                ScanState::Osc => match b {
                    0x07 => self.finish(&mut out, &mut data, b"\x07"),
                    0x1b => self.state = ScanState::OscEsc,
                    _ => {
                        self.osc.push(b);
                        if self.osc.len() > MAX_OSC {
                            data.extend_from_slice(b"\x1b]");
                            data.append(&mut self.osc);
                            self.state = ScanState::Ground;
                        }
                    }
                },
                ScanState::OscEsc => {
                    if b == b'\\' {
                        self.finish(&mut out, &mut data, b"\x1b\\");
                    } else {
                        // OSC interrupted by another sequence: pass it through as is.
                        data.extend_from_slice(b"\x1b]");
                        data.append(&mut self.osc);
                        data.push(0x1b);
                        data.push(b);
                        self.state = ScanState::Ground;
                    }
                }
            }
        }
        if !data.is_empty() {
            out.push(Chunk::Data(data));
        }
        out
    }

    fn finish(&mut self, out: &mut Vec<Chunk>, data: &mut Vec<u8>, terminator: &[u8]) {
        self.state = ScanState::Ground;
        match parse_mark(&self.osc) {
            Some(mark) => {
                if !data.is_empty() {
                    out.push(Chunk::Data(std::mem::take(data)));
                }
                out.push(Chunk::Mark(mark));
            }
            None if self.osc.starts_with(b"133;") => {} // B, P… not of interest
            None => {
                data.extend_from_slice(b"\x1b]");
                data.extend_from_slice(&self.osc);
                data.extend_from_slice(terminator);
            }
        }
        self.osc.clear();
    }
}

fn parse_mark(osc: &[u8]) -> Option<Mark> {
    let s = std::str::from_utf8(osc).ok()?;
    if let Some(rest) = s.strip_prefix("133;") {
        let mut parts = rest.split(';');
        return match parts.next()? {
            "A" => Some(Mark::Prompt),
            "C" => Some(Mark::CommandStart),
            "D" => Some(Mark::CommandEnd(
                parts.next().and_then(|c| c.trim().parse().ok()),
            )),
            _ => None,
        };
    }
    if let Some(url) = s.strip_prefix("7;") {
        let rest = url.strip_prefix("file://").unwrap_or(url);
        // The host runs up to the first slash; the path starts there.
        let slash = rest.find('/')?;
        return Some(Mark::Cwd {
            host: percent_decode(&rest[..slash]),
            path: percent_decode(&rest[slash..]),
        });
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(chunks: &[Chunk]) -> Vec<String> {
        chunks
            .iter()
            .map(|c| match c {
                Chunk::Data(d) => format!("data:{}", String::from_utf8_lossy(d)),
                Chunk::Mark(Mark::Prompt) => "A".into(),
                Chunk::Mark(Mark::CommandStart) => "C".into(),
                Chunk::Mark(Mark::CommandEnd(e)) => format!("D{e:?}"),
                Chunk::Mark(Mark::Cwd { host, path }) => format!("cwd:{host}|{path}"),
            })
            .collect()
    }

    #[test]
    fn splits_marks_from_output() {
        let mut s = Scanner::default();
        let out = s.feed(b"hello\x1b]133;C\x07output\x1b[31mred\x1b]133;D;2\x07");
        assert_eq!(
            marks(&out),
            ["data:hello", "C", "data:output\x1b[31mred", "DSome(2)"]
        );
    }

    #[test]
    fn mark_split_across_reads() {
        let mut s = Scanner::default();
        assert!(marks(&s.feed(b"ab\x1b]13")) == ["data:ab"]);
        assert_eq!(marks(&s.feed(b"3;D;0\x1b")), Vec::<String>::new());
        assert_eq!(marks(&s.feed(b"\\x")), ["DSome(0)", "data:x"]);
    }

    #[test]
    fn other_osc_pass_through() {
        let mut s = Scanner::default();
        let out = s.feed(b"\x1b]0;title\x07z");
        assert_eq!(marks(&out), ["data:\x1b]0;title\x07z"]);
    }

    #[test]
    fn cwd_with_host_and_escapes() {
        let mut s = Scanner::default();
        let out = s.feed(b"\x1b]7;file://mac.local/Users/j/my%20dir\x07");
        assert_eq!(marks(&out), ["cwd:mac.local|/Users/j/my dir"]);
    }
}
