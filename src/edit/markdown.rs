//! Markdown → styled lines for the editor preview.
//!
//! Deliberately small: headings, bold, italic, inline and block code, links,
//! quotes, lists and horizontal rules.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn render(src: &str) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in src.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            let lang = trimmed.trim_start_matches('`').trim();
            if in_code && !lang.is_empty() {
                out.push(Line::styled(
                    format!("  {lang}"),
                    Style::new().fg(Color::DarkGray),
                ));
            }
            continue;
        }
        if in_code {
            out.push(Line::styled(
                format!("  {raw}"),
                Style::new().fg(Color::Yellow),
            ));
            continue;
        }
        if trimmed.is_empty() {
            out.push(Line::default());
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let text = trimmed[hashes..].trim().to_string();
            let style = match hashes {
                1 => Style::new()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                2 => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                _ => Style::new().add_modifier(Modifier::BOLD),
            };
            out.push(Line::styled(text, style));
            continue;
        }
        if matches!(trimmed, "---" | "***" | "___") {
            out.push(Line::styled(
                "─".repeat(40),
                Style::new().fg(Color::DarkGray),
            ));
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("> ")
            .or(trimmed.strip_prefix('>').filter(|r| r.is_empty()))
        {
            let mut spans = vec![Span::styled("▎ ", Style::new().fg(Color::DarkGray))];
            spans.extend(inline(rest, Style::new().add_modifier(Modifier::ITALIC)));
            out.push(Line::from(spans));
            continue;
        }
        let indent = " ".repeat(raw.len() - trimmed.len());
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let (mark, rest) = if let Some(r) = rest.strip_prefix("[ ] ") {
                ("☐ ", r)
            } else if let Some(r) = rest
                .strip_prefix("[x] ")
                .or_else(|| rest.strip_prefix("[X] "))
            {
                ("☑ ", r)
            } else {
                ("• ", rest)
            };
            let mut spans = vec![
                Span::raw(indent),
                Span::styled(mark, Style::new().fg(Color::Cyan)),
            ];
            spans.extend(inline(rest, Style::new()));
            out.push(Line::from(spans));
            continue;
        }
        let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && trimmed[digits..].starts_with(". ") {
            let mut spans = vec![
                Span::raw(indent),
                Span::styled(
                    trimmed[..digits + 2].to_string(),
                    Style::new().fg(Color::Cyan),
                ),
            ];
            spans.extend(inline(&trimmed[digits + 2..], Style::new()));
            out.push(Line::from(spans));
            continue;
        }
        out.push(Line::from(inline(raw, Style::new())));
    }
    out
}

/// Inline styles: `code`, **bold**, *italic* / _italic_ and [links](url).
fn inline(s: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = s.chars().collect();
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let flush = |buf: &mut String, spans: &mut Vec<Span<'static>>| {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), base));
        }
    };
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..chars.len().saturating_sub(pat.len() - 1)).find(|&j| chars[j..].starts_with(pat))
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '`'
            && let Some(end) = find(i + 1, &['`'])
        {
            flush(&mut buf, &mut spans);
            let code: String = chars[i + 1..end].iter().collect();
            spans.push(Span::styled(code, base.fg(Color::Yellow)));
            i = end + 1;
            continue;
        }
        if c == '*'
            && chars.get(i + 1) == Some(&'*')
            && let Some(end) = find(i + 2, &['*', '*'])
        {
            flush(&mut buf, &mut spans);
            let t: String = chars[i + 2..end].iter().collect();
            spans.push(Span::styled(t, base.add_modifier(Modifier::BOLD)));
            i = end + 2;
            continue;
        }
        if (c == '*' || c == '_')
            && chars.get(i + 1).is_some_and(|n| !n.is_whitespace())
            && let Some(end) = find(i + 1, &[c])
        {
            flush(&mut buf, &mut spans);
            let t: String = chars[i + 1..end].iter().collect();
            spans.push(Span::styled(t, base.add_modifier(Modifier::ITALIC)));
            i = end + 1;
            continue;
        }
        if c == '['
            && let Some(close) = find(i + 1, &[']'])
            && chars.get(close + 1) == Some(&'(')
            && let Some(paren) = find(close + 2, &[')'])
        {
            flush(&mut buf, &mut spans);
            let text: String = chars[i + 1..close].iter().collect();
            let url: String = chars[close + 2..paren].iter().collect();
            spans.push(Span::styled(
                text,
                base.fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
            ));
            spans.push(Span::styled(format!(" ({url})"), base.fg(Color::DarkGray)));
            i = paren + 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut spans);
    spans
}
