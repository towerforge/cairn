//! Translation of crossterm keyboard and mouse events into the bytes a
//! program inside the PTY expects (xterm encoding).

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

pub fn encode_key(key: KeyEvent, app_cursor: bool) -> Vec<u8> {
    let m = key.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);
    // xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    let modp = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;

    let mut out = Vec::new();
    let cursor_key = |c: u8| -> Vec<u8> {
        if modp > 1 {
            format!("\x1b[1;{modp}{}", c as char).into_bytes()
        } else if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modp > 1 {
            format!("\x1b[{n};{modp}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    match key.code {
        KeyCode::Char(c) => {
            if alt {
                out.push(0x1b);
            }
            if ctrl {
                let b = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    ' ' | '@' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return out,
                };
                out.push(b);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        KeyCode::Enter => {
            if alt {
                out.push(0x1b);
            }
            out.push(b'\r');
        }
        KeyCode::Tab => out.push(b'\t'),
        KeyCode::BackTab => out.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => {
            if alt {
                out.push(0x1b);
            }
            out.push(if ctrl { 0x08 } else { 0x7f });
        }
        KeyCode::Esc => out.push(0x1b),
        KeyCode::Up => out = cursor_key(b'A'),
        KeyCode::Down => out = cursor_key(b'B'),
        KeyCode::Right => out = cursor_key(b'C'),
        KeyCode::Left => out = cursor_key(b'D'),
        KeyCode::Home => out = cursor_key(b'H'),
        KeyCode::End => out = cursor_key(b'F'),
        KeyCode::Insert => out = tilde(2),
        KeyCode::Delete => out = tilde(3),
        KeyCode::PageUp => out = tilde(5),
        KeyCode::PageDown => out = tilde(6),
        KeyCode::F(n) => {
            out = match n {
                1..=4 if modp == 1 => vec![0x1b, b'O', b'P' + (n - 1)],
                1..=4 => format!("\x1b[1;{modp}{}", (b'P' + (n - 1)) as char).into_bytes(),
                5 => tilde(15),
                6 => tilde(17),
                7 => tilde(18),
                8 => tilde(19),
                9 => tilde(20),
                10 => tilde(21),
                11 => tilde(23),
                12 => tilde(24),
                _ => Vec::new(),
            }
        }
        _ => {}
    }
    out
}

/// Encodes a mouse event in SGR format (`ESC[<b;x;yM`), with coordinates
/// relative to the program's area (0-based). `motion` tells whether the
/// program also asked for motion events with no button pressed.
pub fn encode_mouse(ev: &MouseEvent, col: u16, row: u16, motion: bool) -> Option<Vec<u8>> {
    let btn = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (code, release) = match ev.kind {
        MouseEventKind::Down(b) => (btn(b), false),
        MouseEventKind::Up(b) => (btn(b), true),
        MouseEventKind::Drag(b) => (btn(b) + 32, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::Moved if motion => (35, false),
        _ => return None,
    };
    let mut code = code;
    if ev.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if ev.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }
    let end = if release { 'm' } else { 'M' };
    Some(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes())
}
