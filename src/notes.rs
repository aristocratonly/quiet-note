//! Notes beyond the editor: the index of all notes, [[links]], backlinks, #tags,
//! version history, image attachments and export.

use std::fs;
use std::path::{Path, PathBuf};

pub const HISTORY_DIR: &str = ".history";
pub const ATTACH_DIR: &str = ".attachments";
const HISTORY_KEEP: usize = 60;
const HISTORY_EVERY_SECS: u64 = 300;

/// Every note under the root, depth-first.
pub fn all_notes(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for p in crate::fsops::list(d) {
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    walk(root, &mut out);
    out
}

pub fn title(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Finds the note a [[link]] points to: by name, case-insensitive; `a/b` matches by path tail.
pub fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let t = target.split('|').next().unwrap_or(target).trim().to_lowercase();
    all_notes(root).into_iter().find(|p| {
        let rel = p.strip_prefix(root).unwrap_or(p).with_extension("").to_string_lossy().to_lowercase();
        title(p).to_lowercase() == t || rel == t || rel.ends_with(&format!("/{t}"))
    })
}

/// `[[link]]` spans in a text: (byte range of the whole link, target).
pub fn links(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = text[from..].find("[[") {
        let s = from + i;
        let Some(j) = text[s + 2..].find("]]") else { break };
        let inner = &text[s + 2..s + 2 + j];
        if !inner.contains('\n') && !inner.is_empty() {
            out.push((s..s + 2 + j + 2, inner.to_string()));
        }
        from = s + 2 + j + 2;
    }
    out
}

/// Notes that link to `target`, with the line that holds the link.
pub fn backlinks(root: &Path, target: &Path) -> Vec<(PathBuf, String)> {
    let name = title(target).to_lowercase();
    all_notes(root)
        .into_iter()
        .filter(|p| p != target)
        .filter_map(|p| {
            let t = fs::read_to_string(&p).ok()?;
            let line = t.lines().find(|l| links(l).iter().any(|(_, x)| x.split('|').next().unwrap_or(x).trim().to_lowercase() == name))?;
            Some((p, line.trim().chars().take(140).collect()))
        })
        .collect()
}

/// `#tags` in a text (not Markdown headings): letters, digits, `_`, `-`, `/`.
pub fn tags(text: &str) -> Vec<String> {
    let mut out = vec![];
    for (i, _) in text.match_indices('#') {
        let before = text[..i].chars().next_back();
        if before.is_some_and(|c| !c.is_whitespace() && c != '(') {
            continue;
        }
        let tag: String = text[i + 1..].chars().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/')).collect();
        if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
            out.push(tag.to_lowercase());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// All tags in the notes with how many notes use each, most used first.
pub fn all_tags(root: &Path) -> Vec<(String, usize)> {
    let mut m: std::collections::BTreeMap<String, usize> = Default::default();
    for p in all_notes(root) {
        if let Ok(t) = fs::read_to_string(&p) {
            for tag in tags(&t) {
                *m.entry(tag).or_default() += 1;
            }
        }
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

// ------------------------------------------------------------------ history

fn history_dir(note: &Path) -> PathBuf {
    let parent = note.parent().unwrap_or(Path::new("."));
    parent.join(HISTORY_DIR).join(title(note))
}

/// Saves a copy of the note's previous text if the last copy is old enough.
pub fn snapshot(note: &Path, previous: &str) {
    if previous.trim().is_empty() {
        return;
    }
    let dir = history_dir(note);
    let mut list = versions(note);
    if let Some((p, _)) = list.first() {
        let fresh = fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() < HISTORY_EVERY_SECS);
        if fresh || fs::read_to_string(p).is_ok_and(|t| t == previous) {
            return;
        }
    }
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string();
    let _ = fs::write(dir.join(format!("{stamp}.md")), previous);
    list = versions(note);
    for (p, _) in list.into_iter().skip(HISTORY_KEEP) {
        let _ = fs::remove_file(p);
    }
}

/// Saved versions, newest first: (file, human label).
pub fn versions(note: &Path) -> Vec<(PathBuf, String)> {
    let mut v: Vec<PathBuf> = fs::read_dir(history_dir(note)).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v.reverse();
    v.into_iter()
        .map(|p| {
            let s = title(&p);
            let label = match s.split_once(' ') {
                Some((d, t)) => format!("{d}  {}", t.replace('-', ":")),
                None => s,
            };
            (p, label)
        })
        .collect()
}

/// Keeps history with the note when it is renamed or moved.
pub fn move_history(old: &Path, new: &Path) {
    let (a, b) = (history_dir(old), history_dir(new));
    if a.exists() && !b.exists() {
        if let Some(p) = b.parent() {
            let _ = fs::create_dir_all(p);
        }
        let _ = fs::rename(a, b);
    }
}

// ------------------------------------------------------------------ images

pub fn is_image(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(), Some("png" | "jpg" | "jpeg" | "gif" | "webp"))
}

/// Copies an image next to the note and returns the Markdown that shows it.
pub fn attach_image(note: &Path, src: &Path) -> Result<String, String> {
    let dir = note.parent().unwrap_or(Path::new(".")).join(ATTACH_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("Не удалось сохранить картинку: {e}"))?;
    let stem = title(src);
    let ext = src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or("png".into());
    let mut name = format!("{stem}.{ext}");
    let mut i = 2;
    while dir.join(&name).exists() {
        name = format!("{stem} {i}.{ext}");
        i += 1;
    }
    fs::copy(src, dir.join(&name)).map_err(|e| format!("Не удалось сохранить картинку: {e}"))?;
    Ok(format!("![{stem}]({ATTACH_DIR}/{})", name.replace(' ', "%20")))
}

/// `![alt](path)` on a line by itself → absolute path of the image.
pub fn image_line(note: &Path, line: &str) -> Option<PathBuf> {
    let t = line.trim();
    let rest = t.strip_prefix("![")?;
    let (_, after) = rest.split_once("](")?;
    let path = after.strip_suffix(')')?.replace("%20", " ");
    if path.contains("://") {
        return None;
    }
    Some(note.parent().unwrap_or(Path::new(".")).join(path))
}

// ------------------------------------------------------------------ export

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn inline_html(s: &str) -> String {
    let mut t = esc(s);
    let re = |p: &str| regex::Regex::new(p).unwrap();
    t = re(r"\*\*(.+?)\*\*").replace_all(&t, "<strong>$1</strong>").into_owned();
    t = re(r"\*(.+?)\*").replace_all(&t, "<em>$1</em>").into_owned();
    t = re(r"`(.+?)`").replace_all(&t, "<code>$1</code>").into_owned();
    t = re(r"\[\[(.+?)\]\]").replace_all(&t, "<span class=link>$1</span>").into_owned();
    t
}

/// A standalone HTML page for the note; images are embedded.
pub fn to_html(note: &Path, text: &str) -> String {
    use base64::Engine;
    let mut body = String::new();
    let mut first = true;
    let mut in_list = false;
    let close_list = |body: &mut String, in_list: &mut bool| {
        if *in_list {
            body.push_str("</ul>\n");
            *in_list = false;
        }
    };
    let mut doc_title = title(note);
    for line in text.lines() {
        let t = line.trim();
        if first && !t.is_empty() {
            first = false;
            doc_title = t.trim_start_matches('#').trim().to_string();
            body += &format!("<h1>{}</h1>\n", inline_html(&doc_title));
            continue;
        }
        if let Some(img) = image_line(note, t) {
            close_list(&mut body, &mut in_list);
            if let Ok(bytes) = fs::read(&img) {
                let ext = img.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let mime = if ext == "jpg" { "jpeg".to_string() } else { ext };
                body += &format!("<img src=\"data:image/{mime};base64,{}\">\n", base64::engine::general_purpose::STANDARD.encode(bytes));
            }
        } else if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            if !in_list {
                body.push_str("<ul>\n");
                in_list = true;
            }
            let r = r.replace("[ ] ", "☐ ").replace("[x] ", "✓ ");
            body += &format!("<li>{}</li>\n", inline_html(&r));
        } else {
            close_list(&mut body, &mut in_list);
            if t.is_empty() {
                continue;
            } else if t.starts_with('#') {
                let lvl = t.chars().take_while(|c| *c == '#').count().clamp(1, 3) + 1;
                body += &format!("<h{lvl}>{}</h{lvl}>\n", inline_html(t.trim_start_matches('#').trim()));
            } else if let Some(r) = t.strip_prefix("> ") {
                body += &format!("<blockquote>{}</blockquote>\n", inline_html(r));
            } else if t == "---" {
                body.push_str("<hr>\n");
            } else {
                body += &format!("<p>{}</p>\n", inline_html(t));
            }
        }
    }
    close_list(&mut body, &mut in_list);
    format!(
        "<!doctype html>\n<html lang=\"ru\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>\nbody{{max-width:40em;margin:4em auto;padding:0 1.2em;font:18px/1.7 Literata,Georgia,serif;color:#1d1f24;background:#fbfbf8}}\nh1{{font-size:2em;line-height:1.2;margin:0 0 1em}}h2,h3,h4{{margin:1.6em 0 .4em;line-height:1.3}}\nblockquote{{margin:1em 0;padding-left:1em;border-left:2px solid #9aa4d8;color:#555;font-style:italic}}\ncode{{font:.88em ui-monospace,monospace;background:#eee;padding:.1em .3em;border-radius:4px}}\n.link{{color:#4655b8}}img{{max-width:100%;border-radius:6px}}\n@media (prefers-color-scheme:dark){{body{{background:#141518;color:#d9dbe1}}blockquote{{color:#a9acb6}}code{{background:#1d1f25}}.link{{color:#93a4f5}}}}\n</style></head><body>\n{body}</body></html>\n",
        esc(&doc_title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_and_tags() {
        let l = links("см. [[Идеи]] и [[a/b|текст]]");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].1, "Идеи");
        assert_eq!(tags("# Заголовок\nтекст #работа и #идеи/новые, не#тег, #123"), vec!["идеи/новые", "работа"]);
    }

    #[test]
    fn html_export() {
        let h = to_html(Path::new("/x/n.md"), "Титул\n\n**жир** и [[связь]]\n- пункт");
        assert!(h.contains("<h1>Титул</h1>") && h.contains("<strong>жир</strong>") && h.contains("<li>пункт</li>"));
    }
}
