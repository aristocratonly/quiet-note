//! Look of the note editor: a light Markdown styling applied while typing.
//! The text itself stays plain; only the way it is drawn changes.

use crate::theme;
use crate::TextStyle;
use eframe::egui::{text::LayoutJob, Color32, FontFamily, FontId, Stroke, TextFormat};

pub struct Look {
    pub size: f32,
    pub style: TextStyle,
    /// Byte range of the paragraph being written; other text is dimmed (focus mode).
    pub focus: Option<std::ops::Range<usize>>,
    /// Reading mode: Markdown markers are hidden.
    pub reading: bool,
    /// Treat the first non-empty line as the title.
    pub title: bool,
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
    build(text, look, wrap).0
}

/// Layout plus the [[link]] spans as byte ranges in the job's text.
pub fn build(text: &str, look: &Look, wrap: f32) -> (LayoutJob, Vec<(std::ops::Range<usize>, String)>) {
    let mut b = B { job: LayoutJob::default(), links: vec![], look };
    b.job.wrap.max_width = wrap;
    let s = look.size;
    let mut first = look.title;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start_sections = b.job.sections.len();
        let line_range = offset..offset + line.len();
        offset += line.len();
        b.line(line, s, &mut first);
        if let Some(f) = &look.focus {
            let inside = line_range.start <= f.end && f.start <= line_range.end.saturating_sub(1).max(line_range.start);
            if !inside {
                for sec in &mut b.job.sections[start_sections..] {
                    sec.format.color = sec.format.color.gamma_multiply(0.22);
                }
            }
        }
    }
    (b.job, b.links)
}

struct B<'a> {
    job: LayoutJob,
    links: Vec<(std::ops::Range<usize>, String)>,
    look: &'a Look,
}

impl B<'_> {
    fn put(&mut self, text: &str, f: TextFormat) {
        self.job.append(text, 0.0, f);
    }

    /// A Markdown marker: dimmed when writing, hidden (or replaced) when reading.
    fn mark(&mut self, text: &str, shown: &str, f: TextFormat) {
        if self.look.reading {
            if !shown.is_empty() {
                self.put(shown, f);
            }
        } else {
            self.put(text, f);
        }
    }

    fn line(&mut self, line: &str, s: f32, first: &mut bool) {
        let look = self.look;
        let body = line.strip_suffix('\n').unwrap_or(line);
        let nl = if body.len() < line.len() { "\n" } else { "" };
        let trimmed = body.trim_start();
        let indent = &body[..body.len() - trimmed.len()];

        if *first && !body.trim().is_empty() {
            *first = false;
            let (marks, rest) = split_heading(body);
            self.mark(marks, "", look.fmt(s * 1.6, false, false, MARK));
            self.inline(rest, s * 1.6, true, false, theme::TEXT);
            self.put(nl, look.fmt(s * 1.6, false, false, BODY));
            return;
        }
        if !body.trim().is_empty() {
            *first = false;
        }

        if trimmed.starts_with("![") && trimmed.contains("](") {
            let alt = trimmed[2..].split(']').next().unwrap_or("");
            let shown = format!("{}  {}", egui_phosphor::regular::IMAGE, if alt.is_empty() { "картинка" } else { alt });
            let f = look.fmt(s * 0.9, false, false, theme::FAINT);
            if look.reading {
                self.put(&shown, f);
            } else {
                self.put(body, f);
            }
        } else if trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let k = match level { 1 => 1.45, 2 => 1.25, _ => 1.1 };
            let (marks, rest) = split_heading(body);
            self.mark(marks, "", look.fmt(s * k, false, false, MARK));
            self.inline(rest, s * k, true, false, theme::TEXT);
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            self.put(indent, look.fmt(s, false, false, BODY));
            self.mark("> ", &format!("{}  ", egui_phosphor::regular::QUOTES), look.fmt(s, true, false, theme::ACCENT.gamma_multiply(0.7)));
            self.inline(rest, s, false, true, QUOTE);
        } else if let Some(rest) = trimmed.strip_prefix("- [ ] ").or_else(|| trimmed.strip_prefix("- [x] ")) {
            let done = trimmed.starts_with("- [x]");
            self.put(indent, look.fmt(s, false, false, BODY));
            let shown = if done { "☑  " } else { "☐  " };
            self.mark(&trimmed[..6], shown, look.fmt(s, false, false, if done { theme::GOOD } else { MARK }));
            let mut f = look.fmt(s, false, false, if done { theme::FAINT } else { BODY });
            if done {
                f.strikethrough = Stroke::new(1.0, theme::FAINT);
            }
            self.put(rest, f);
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            self.put(indent, look.fmt(s, false, false, BODY));
            self.mark(&trimmed[..2], "•  ", look.fmt(s, true, false, theme::ACCENT.gamma_multiply(0.8)));
            self.inline(rest, s, false, false, BODY);
        } else if let Some(n) = numbered(trimmed) {
            self.put(indent, look.fmt(s, false, false, BODY));
            self.put(&trimmed[..n], look.fmt(s, false, false, theme::ACCENT.gamma_multiply(0.8)));
            self.inline(&trimmed[n..], s, false, false, BODY);
        } else if trimmed == "---" || trimmed == "***" {
            self.mark(body, "⸻", look.fmt(s, false, false, MARK));
        } else {
            self.inline(body, s, false, false, BODY);
        }
        self.put(nl, look.fmt(s, false, false, BODY));
    }

    /// Plain text with #tags coloured.
    fn plain(&mut self, text: &str, size: f32, bold: bool, italic: bool, color: Color32) {
        let look = self.look;
        let mut last = 0;
        for (i, _) in text.match_indices('#') {
            if i < last || text[..i].chars().next_back().is_some_and(|c| !c.is_whitespace() && c != '(') {
                continue;
            }
            let tag_len: usize = text[i + 1..].chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/')).map(char::len_utf8).sum();
            if tag_len == 0 {
                continue;
            }
            self.put(&text[last..i], look.fmt(size, bold, italic, color));
            let mut f = look.fmt(size * 0.92, false, false, theme::ACCENT);
            f.background = theme::ACCENT_BG;
            self.put(&text[i..i + 1 + tag_len], f);
            last = i + 1 + tag_len;
        }
        self.put(&text[last..], look.fmt(size, bold, italic, color));
    }

    /// **bold**, *italic*, `code` and [[links]] inside a line.
    fn inline(&mut self, line: &str, size: f32, bold: bool, italic: bool, color: Color32) {
        let look = self.look;
        let mut rest = line;
        while !rest.is_empty() {
            let next = ["**", "`", "[[", "*"].iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min_by_key(|(i, m)| (*i, std::cmp::Reverse(m.len())));
            let Some((i, m)) = next else {
                self.plain(rest, size, bold, italic, color);
                return;
            };
            let close = match m { "[[" => "]]", x => x };
            let after = &rest[i + m.len()..];
            let Some(j) = after.find(close).filter(|j| *j > 0) else {
                self.plain(&rest[..i + m.len()], size, bold, italic, color);
                rest = after;
                continue;
            };
            self.plain(&rest[..i], size, bold, italic, color);
            let inner = &after[..j];
            self.mark(m, "", look.fmt(size, false, false, MARK));
            match m {
                "**" => self.put(inner, look.fmt(size, true, italic, theme::TEXT)),
                "*" => self.put(inner, look.fmt(size, bold, true, color)),
                "`" => {
                    let mut f = look.fmt(size * 0.88, false, false, theme::WARN);
                    f.font_id.family = FontFamily::Monospace;
                    f.background = CODE_BG;
                    self.put(inner, f);
                }
                _ => {
                    let target = inner.split('|').next().unwrap_or(inner).to_string();
                    let shown = if self.look.reading { inner.split('|').next_back().unwrap_or(inner) } else { inner };
                    let start = self.job.text.len();
                    self.put(shown, look.fmt(size, bold, italic, theme::ACCENT));
                    self.links.push((start..self.job.text.len(), target));
                }
            }
            self.mark(close, "", look.fmt(size, false, false, MARK));
            rest = &after[j + close.len()..];
        }
    }
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

/// Smooth 0→1 easing for fades (ease-out cubic).
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}
