//! Investigations: one folder per case.
//!
//! ```text
//! <Case>/case.md            overview text + case-level events
//! <Case>/entities/<Name>.md one card per entity
//! <Case>/files/             attached evidence
//! <Case>/graph.pos          node positions on the graph
//! ```
//!
//! Cards are plain Markdown with a small header, readable without the app:
//!
//! ```text
//! ---
//! type: person
//! field: ФИО = Иван Петров
//! link: example.com | владеет
//! source: https://… | 2026-11-17 | confirmed | WHOIS
//! event: 2024-03-01 | Зарегистрировал домен
//! ---
//! free notes
//! ```

use eframe::egui::Color32;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Person,
    Nick,
    Email,
    Phone,
    Domain,
    Org,
    Place,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 8] =
        [Kind::Person, Kind::Nick, Kind::Email, Kind::Phone, Kind::Domain, Kind::Org, Kind::Place, Kind::Other];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Nick => "nick",
            Kind::Email => "email",
            Kind::Phone => "phone",
            Kind::Domain => "domain",
            Kind::Org => "org",
            Kind::Place => "place",
            Kind::Other => "other",
        }
    }

    pub fn from_key(s: &str) -> Kind {
        Kind::ALL.into_iter().find(|k| k.key() == s).unwrap_or(Kind::Other)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Person => "Человек",
            Kind::Nick => "Никнейм",
            Kind::Email => "Почта",
            Kind::Phone => "Телефон",
            Kind::Domain => "Домен или сайт",
            Kind::Org => "Организация",
            Kind::Place => "Место",
            Kind::Other => "Другое",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Kind::Person => Color32::from_rgb(0xc9, 0x9a, 0x6b),
            Kind::Nick => Color32::from_rgb(0x9b, 0x8c, 0xd6),
            Kind::Email => Color32::from_rgb(0x6f, 0xa8, 0xc9),
            Kind::Phone => Color32::from_rgb(0x7f, 0xb8, 0x8a),
            Kind::Domain => Color32::from_rgb(0x5f, 0xb3, 0xb0),
            Kind::Org => Color32::from_rgb(0xc7, 0x86, 0x8f),
            Kind::Place => Color32::from_rgb(0xb8, 0xb0, 0x6a),
            Kind::Other => Color32::from_rgb(0x8a, 0x8d, 0x94),
        }
    }

    /// Field names a fresh card of this kind starts with.
    pub fn default_fields(self) -> &'static [&'static str] {
        match self {
            Kind::Person => &["ФИО", "Дата рождения", "Город"],
            Kind::Nick => &["Платформа", "Профиль"],
            Kind::Email => &["Адрес"],
            Kind::Phone => &["Номер", "Оператор"],
            Kind::Domain => &["Адрес", "Регистратор", "IP"],
            Kind::Org => &["Название", "Рег. номер"],
            Kind::Place => &["Адрес", "Координаты"],
            Kind::Other => &[],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Reliability {
    Confirmed,
    #[default]
    Likely,
    Doubtful,
}

impl Reliability {
    pub const ALL: [Reliability; 3] = [Reliability::Confirmed, Reliability::Likely, Reliability::Doubtful];
    fn key(self) -> &'static str {
        match self {
            Reliability::Confirmed => "confirmed",
            Reliability::Likely => "likely",
            Reliability::Doubtful => "doubtful",
        }
    }
    fn from_key(s: &str) -> Self {
        Self::ALL.into_iter().find(|r| r.key() == s).unwrap_or_default()
    }
    pub fn label(self) -> &'static str {
        match self {
            Reliability::Confirmed => "подтверждено",
            Reliability::Likely => "вероятно",
            Reliability::Doubtful => "сомнительно",
        }
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Link {
    pub target: String,
    pub relation: String,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Source {
    pub url: String,
    pub date: String,
    pub reliability: Reliability,
    pub note: String,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Event {
    pub date: String,
    pub text: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Doc {
    pub name: String,
    pub kind: Kind,
    pub fields: Vec<(String, String)>,
    pub links: Vec<Link>,
    pub sources: Vec<Source>,
    pub events: Vec<Event>,
    pub notes: String,
}

/// Keep header values on one line and free of our separators.
fn clean(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace(" | ", " / ")
}

impl Doc {
    pub fn new(name: &str, kind: Kind) -> Self {
        Self {
            name: name.into(),
            kind,
            fields: vec![],
            links: vec![],
            sources: vec![],
            events: vec![],
            notes: String::new(),
        }
    }

    pub fn serialize(&self) -> String {
        let mut h = format!("---\ntype: {}\n", self.kind.key());
        for (k, v) in &self.fields {
            h += &format!("field: {} = {}\n", clean(k).replace(" = ", " - "), clean(v));
        }
        for l in &self.links {
            h += &format!("link: {} | {}\n", clean(&l.target), clean(&l.relation));
        }
        for s in &self.sources {
            h += &format!(
                "source: {} | {} | {} | {}\n",
                clean(&s.url),
                clean(&s.date),
                s.reliability.key(),
                clean(&s.note)
            );
        }
        for e in &self.events {
            h += &format!("event: {} | {}\n", clean(&e.date), clean(&e.text));
        }
        h + "---\n" + &self.notes
    }

    pub fn parse(name: &str, text: &str) -> Self {
        let mut d = Doc::new(name, Kind::Other);
        let (head, body) = match text.strip_prefix("---\n").and_then(|r| r.find("\n---").map(|i| (r, i))) {
            Some((r, i)) => (&r[..i], r[i + 4..].strip_prefix('\n').unwrap_or(&r[i + 4..])),
            None => ("", text),
        };
        d.notes = body.to_string();
        for line in head.lines() {
            let Some((key, v)) = line.split_once(": ").or_else(|| line.strip_suffix(':').map(|k| (k, ""))) else {
                continue;
            };
            let parts: Vec<&str> = v.split(" | ").collect();
            let part = |i: usize| parts.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
            match key {
                "type" => d.kind = Kind::from_key(v.trim()),
                "field" => {
                    let (k, val) = v.split_once(" = ").unwrap_or((v, ""));
                    d.fields.push((k.trim().into(), val.trim().into()));
                }
                "link" => d.links.push(Link { target: part(0), relation: part(1) }),
                "source" => d.sources.push(Source {
                    url: part(0),
                    date: part(1),
                    reliability: Reliability::from_key(&part(2)),
                    note: parts.get(3..).map(|p| p.join(" | ")).unwrap_or_default().trim().into(),
                }),
                "event" => d.events.push(Event { date: part(0), text: parts.get(1..).map(|p| p.join(" | ")).unwrap_or_default().trim().into() }),
                _ => {}
            }
        }
        d
    }
}

pub fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() || n.contains('/') || n.starts_with('.') {
        return Err("Недопустимое название".into());
    }
    Ok(n.to_string())
}

fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.into();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap()
}

pub fn list_cases(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(root) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

pub fn create_case(root: &Path) -> Result<PathBuf, String> {
    let name = unique_name("Новое расследование", |n| root.join(n).exists());
    let dir = root.join(name);
    let err = |e: std::io::Error| format!("Не удалось создать расследование: {e}");
    fs::create_dir_all(dir.join("entities")).map_err(err)?;
    fs::create_dir_all(dir.join("files")).map_err(err)?;
    fs::write(dir.join("case.md"), Doc::new("", Kind::Other).serialize()).map_err(err)?;
    Ok(dir)
}

/// Overview is `None`, entity `i` is `Some(i)`.
pub type Key = Option<usize>;

pub struct TimelineItem {
    pub date: String,
    pub text: String,
    pub owner: Key,
}

pub struct Case {
    pub dir: PathBuf,
    pub overview: Doc,
    pub entities: Vec<Doc>,
    pub pos: HashMap<String, [f32; 2]>,
    dirty: HashSet<Key>,
    pos_dirty: bool,
}

impl Case {
    pub fn load(dir: &Path) -> Case {
        let read = |p: PathBuf| fs::read_to_string(p).unwrap_or_default();
        let overview = Doc::parse("", &read(dir.join("case.md")));
        let mut entities: Vec<Doc> = fs::read_dir(dir.join("entities"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "md"))
                    .map(|p| {
                        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
                        Doc::parse(&name, &read(p))
                    })
                    .collect()
            })
            .unwrap_or_default();
        entities.sort_by_key(|e| e.name.to_lowercase());
        let pos = read(dir.join("graph.pos"))
            .lines()
            .filter_map(|l| {
                let mut it = l.rsplitn(3, '\t');
                let y = it.next()?.parse().ok()?;
                let x = it.next()?.parse().ok()?;
                Some((it.next()?.to_string(), [x, y]))
            })
            .collect();
        Case { dir: dir.into(), overview, entities, pos, dirty: HashSet::new(), pos_dirty: false }
    }

    pub fn name(&self) -> String {
        self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn touch(&mut self, k: Key) {
        self.dirty.insert(k);
    }
    pub fn touch_pos(&mut self) {
        self.pos_dirty = true;
    }
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty() || self.pos_dirty
    }

    fn entity_path(&self, name: &str) -> PathBuf {
        self.dir.join("entities").join(format!("{name}.md"))
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.entities.iter().position(|e| e.name == name)
    }

    pub fn save(&mut self) -> Result<(), String> {
        let err = |e: std::io::Error| format!("Не удалось сохранить: {e}");
        for k in std::mem::take(&mut self.dirty) {
            match k {
                None => fs::write(self.dir.join("case.md"), self.overview.serialize()).map_err(err)?,
                Some(i) => {
                    if let Some(e) = self.entities.get(i) {
                        fs::write(self.entity_path(&e.name), e.serialize()).map_err(err)?;
                    }
                }
            }
        }
        if self.pos_dirty {
            let mut s = String::new();
            for (n, [x, y]) in &self.pos {
                s += &format!("{n}\t{x:.0}\t{y:.0}\n");
            }
            fs::write(self.dir.join("graph.pos"), s).map_err(err)?;
            self.pos_dirty = false;
        }
        Ok(())
    }

    pub fn add_entity(&mut self, kind: Kind) -> usize {
        let name = unique_name(kind.label(), |n| self.find(n).is_some() || self.entity_path(n).exists());
        let mut d = Doc::new(&name, kind);
        d.fields = kind.default_fields().iter().map(|f| (f.to_string(), String::new())).collect();
        self.entities.push(d);
        let i = self.entities.len() - 1;
        self.touch(Some(i));
        i
    }

    pub fn rename_entity(&mut self, i: usize, new: &str) -> Result<(), String> {
        let new = valid_name(new)?;
        let old = self.entities[i].name.clone();
        if new == old {
            return Ok(());
        }
        if self.find(&new).is_some() || self.entity_path(&new).exists() {
            return Err(format!("«{new}» уже есть в этом расследовании"));
        }
        self.save()?;
        let (from, to) = (self.entity_path(&old), self.entity_path(&new));
        if from.exists() {
            fs::rename(from, to).map_err(|e| format!("Не удалось переименовать: {e}"))?;
        }
        self.entities[i].name = new.clone();
        for j in 0..self.entities.len() {
            let mut hit = false;
            for l in self.entities[j].links.iter_mut().filter(|l| l.target == old) {
                l.target = new.clone();
                hit = true;
            }
            if hit {
                self.touch(Some(j));
            }
        }
        if let Some(p) = self.pos.remove(&old) {
            self.pos.insert(new, p);
            self.pos_dirty = true;
        }
        self.save()
    }

    pub fn delete_entity(&mut self, i: usize) -> Result<(), String> {
        self.save()?;
        let p = self.entity_path(&self.entities[i].name);
        if p.exists() {
            trash::delete(&p).map_err(|e| format!("Не удалось удалить: {e}"))?;
        }
        let d = self.entities.remove(i);
        if self.pos.remove(&d.name).is_some() {
            self.pos_dirty = true;
        }
        Ok(())
    }

    /// Copy a file into `files/` and return its path relative to the case.
    pub fn attach(&self, src: &Path) -> Result<String, String> {
        let files = self.dir.join("files");
        fs::create_dir_all(&files).map_err(|e| e.to_string())?;
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or("file".into());
        let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let name = unique_name(&stem, |n| files.join(format!("{n}{ext}")).exists());
        let file = format!("{name}{ext}");
        fs::copy(src, files.join(&file)).map_err(|e| format!("Не удалось приложить файл: {e}"))?;
        Ok(format!("files/{file}"))
    }

    /// Every event of the case and its cards, oldest first; undated at the end.
    pub fn timeline(&self) -> Vec<TimelineItem> {
        let mut v: Vec<TimelineItem> = self
            .overview
            .events
            .iter()
            .map(|e| (None, e))
            .chain(self.entities.iter().enumerate().flat_map(|(i, d)| d.events.iter().map(move |e| (Some(i), e))))
            .map(|(owner, e)| TimelineItem { date: e.date.clone(), text: e.text.clone(), owner })
            .collect();
        v.sort_by(|a, b| (a.date.is_empty(), &a.date).cmp(&(b.date.is_empty(), &b.date)));
        v
    }
}

/// Today as YYYY-MM-DD (UTC), without pulling in a date crate.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let z = secs / 86400 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut d = Doc::new("Иван", Kind::Person);
        d.fields.push(("ФИО".into(), "Иван Петров".into()));
        d.links.push(Link { target: "example.com".into(), relation: "владеет".into() });
        d.sources.push(Source {
            url: "https://whois".into(),
            date: "2026-11-17".into(),
            reliability: Reliability::Confirmed,
            note: "WHOIS".into(),
        });
        d.events.push(Event { date: "2024-03-01".into(), text: "Регистрация".into() });
        d.notes = "свободный текст\n---\nещё".into();
        assert_eq!(Doc::parse("Иван", &d.serialize()), d);
    }

    #[test]
    fn plain_markdown_is_notes() {
        let d = Doc::parse("x", "просто текст");
        assert_eq!(d.notes, "просто текст");
        assert_eq!(d.kind, Kind::Other);
    }

    #[test]
    fn rename_updates_links() {
        let root = std::env::temp_dir().join(format!("qn-case-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let dir = create_case(&root).unwrap();
        let mut c = Case::load(&dir);
        let a = c.add_entity(Kind::Person);
        let b = c.add_entity(Kind::Domain);
        let target = c.entities[b].name.clone();
        c.entities[a].links.push(Link { target, relation: "владеет".into() });
        c.touch(Some(a));
        c.save().unwrap();
        c.rename_entity(b, "example.com").unwrap();
        let c2 = Case::load(&dir);
        let p = c2.find("Человек").unwrap();
        assert_eq!(c2.entities[p].links[0].target, "example.com");
        assert!(dir.join("entities/example.com.md").exists());
    }

    #[test]
    fn today_format() {
        assert_eq!(today().len(), 10);
    }
}
