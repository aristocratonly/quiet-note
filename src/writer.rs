//! Look of the note editor: a light Markdown styling applied while typing.
//! The text itself stays plain; only the way it is drawn changes.

use crate::theme;
use crate::TextStyle;
use eframe::egui::{text::LayoutJob, Color32, FontFamily, FontId, Stroke, TextFormat};

pub struct Look {
    pub size: f32,
    pub style: TextStyle,
}

impl Look {
    fn fam(&self, bold: bool, italic: bool) -> FontFamily {
        match (self.style, bold, italic) {
            (TextStyle::Mono, _, _) => FontFamily::Monospace,
            (TextStyle::Serif, true, _) => FontFamily::Name(theme::SERIF_BOLD.into()),
            (TextStyle::Serif, _, true) => FontFamily::Name(theme::SERIF_ITALIC.into()),
            (TextStyle::Serif, _, _) => FontFamily::Name(theme::SERIF.into()),
            (TextStyle::Sans, true, _) => FontFamily::Name(theme::WRITE_SANS_BOLD.into()),
            (TextStyle::Sans, _, _) => FontFamily::Name(theme::WRITE_SANS.into()),
        }
    }

    fn fmt(&self, size: f32, bold: bool, italic: bool, color: Color32) -> TextFormat {
        TextFormat {
            font_id: FontId::new(size, self.fam(bold, italic)),
            color,
            line_height: Some((size * 1.7).round()),
            italics: italic && self.style != TextStyle::Serif,
            ..Default::default()
        }
    }
}

pub const BODY: Color32 = Color32::from_rgb(0xd9, 0xdb, 0xe1);
const MARK: Color32 = Color32::from_rgb(0x4a, 0x4d, 0x57);
const QUOTE: Color32 = Color32::from_rgb(0xa9, 0xac, 0xb6);
const CODE_BG: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x25);

/// Builds the styled layout for the whole note.
pub fn layout(text: &str, look: &Look, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let s = look.size;
    let mut first = true;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let nl = if body.len() < line.len() { "\n" } else { "" };
        let trimmed = body.trim_start();
        let indent = &body[..body.len() - trimmed.len()];

        if first && !body.trim().is_empty() {
            // The first line is the title.
            first = false;
            let (marks, rest) = split_heading(body);
            job.append(marks, 0.0, look.fmt(s * 1.6, false, false, MARK));
            job.append(rest, 0.0, look.fmt(s * 1.6, true, false, theme::TEXT));
            job.append(nl, 0.0, look.fmt(s * 1.6, false, false, BODY));
            continue;
        }
        if !body.trim().is_empty() {
            first = false;
        }

        if trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let k = match level { 1 => 1.45, 2 => 1.25, _ => 1.1 };
            let (marks, rest) = split_heading(body);
            job.append(marks, 0.0, look.fmt(s * k, false, false, MARK));
            inline(&mut job, rest, look, s * k, true, false, theme::TEXT);
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            job.append(indent, 0.0, look.fmt(s, false, false, BODY));
            job.append("> ", 0.0, look.fmt(s, true, false, theme::ACCENT.gamma_multiply(0.7)));
            inline(&mut job, rest, look, s, false, true, QUOTE);
        } else if let Some(rest) = trimmed.strip_prefix("- [ ] ").or_else(|| trimmed.strip_prefix("- [x] ")) {
            let done = trimmed.starts_with("- [x]");
            job.append(indent, 0.0, look.fmt(s, false, false, BODY));
            job.append(&trimmed[..6], 0.0, look.fmt(s, false, false, if done { theme::GOOD } else { MARK }));
            let mut f = look.fmt(s, false, false, if done { theme::FAINT } else { BODY });
            if done {
                f.strikethrough = Stroke::new(1.0, theme::FAINT);
            }
            job.append(rest, 0.0, f);
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            job.append(indent, 0.0, look.fmt(s, false, false, BODY));
            job.append(&trimmed[..2], 0.0, look.fmt(s, true, false, theme::ACCENT.gamma_multiply(0.8)));
            inline(&mut job, rest, look, s, false, false, BODY);
        } else if let Some(n) = numbered(trimmed) {
            job.append(indent, 0.0, look.fmt(s, false, false, BODY));
            job.append(&trimmed[..n], 0.0, look.fmt(s, false, false, theme::ACCENT.gamma_multiply(0.8)));
            inline(&mut job, &trimmed[n..], look, s, false, false, BODY);
        } else if trimmed == "---" || trimmed == "***" {
            job.append(body, 0.0, look.fmt(s, false, false, MARK));
        } else {
            inline(&mut job, body, look, s, false, false, BODY);
        }
        job.append(nl, 0.0, look.fmt(s, false, false, BODY));
    }
    job
}

fn split_heading(line: &str) -> (&str, &str) {
    let t = line.trim_start();
    if t.starts_with('#') {
        let n = line.len() - t.len() + t.chars().take_while(|c| *c == '#').count();
        let n = if line[n..].starts_with(' ') { n + 1 } else { n };
        (&line[..n], &line[n..])
    } else {
        ("", line)
    }
}

fn numbered(t: &str) -> Option<usize> {
    let d = t.chars().take_while(|c| c.is_ascii_digit()).count();
    (d > 0 && d < 4 && t[d..].starts_with(". ")).then_some(d + 2)
}

/// **bold**, *italic*, `code` and [[links]] inside a line. Markers stay visible but dimmed.
fn inline(job: &mut LayoutJob, line: &str, look: &Look, size: f32, bold: bool, italic: bool, color: Color32) {
    let mut rest = line;
    while !rest.is_empty() {
        let next = ["**", "`", "[[", "*"].iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min_by_key(|(i, m)| (*i, std::cmp::Reverse(m.len())));
        let Some((i, m)) = next else {
            job.append(rest, 0.0, look.fmt(size, bold, italic, color));
            return;
        };
        let close = match m { "[[" => "]]", x => x };
        let after = &rest[i + m.len()..];
        let Some(j) = after.find(close).filter(|j| *j > 0) else {
            job.append(&rest[..i + m.len()], 0.0, look.fmt(size, bold, italic, color));
            rest = after;
            continue;
        };
        job.append(&rest[..i], 0.0, look.fmt(size, bold, italic, color));
        let inner = &after[..j];
        job.append(m, 0.0, look.fmt(size, false, false, MARK));
        match m {
            "**" => job.append(inner, 0.0, look.fmt(size, true, italic, theme::TEXT)),
            "*" => job.append(inner, 0.0, look.fmt(size, bold, true, color)),
            "`" => {
                let mut f = look.fmt(size * 0.88, false, false, theme::WARN);
                f.font_id.family = FontFamily::Monospace;
                f.background = CODE_BG;
                job.append(inner, 0.0, f);
            }
            _ => {
                job.append(inner, 0.0, look.fmt(size, bold, italic, theme::ACCENT));
            }
        }
        job.append(close, 0.0, look.fmt(size, false, false, MARK));
        rest = &after[j + close.len()..];
    }
}

/// Smooth 0→1 easing for fades (ease-out cubic).
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}
