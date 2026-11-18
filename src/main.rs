#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autosearch;
mod extract;
mod fsops;
mod inv_ui;
mod investigation;
mod map;
mod theme;
mod timescale;
mod notes;
mod palette;
mod pdf;
mod select;
mod vault;
mod writer;

use eframe::egui::{self, RichText, Sense, Stroke};
use egui_phosphor::regular as ic;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const AUTOSAVE: Duration = Duration::from_millis(800);

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, Default)]
enum Mode {
    #[default]
    Notes,
    Investigations,
}

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, Default, Debug)]
pub enum TextStyle {
    #[default]
    Sans,
    Serif,
    Mono,
}

impl TextStyle {
    fn label(self) -> &'static str {
        match self {
            TextStyle::Sans => "Без засечек",
            TextStyle::Serif => "С засечками",
            TextStyle::Mono => "Моноширинный",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    root: PathBuf,
    font_size: f32,
    text_style: TextStyle,
    wide: bool,
    sidebar_width: f32,
    expanded: HashSet<PathBuf>,
    last_open: Option<PathBuf>,
    mode: Mode,
    inv_root: PathBuf,
    last_case: Option<PathBuf>,
    smooth: bool,
    /// Draw the app's own pointer instead of the system one.
    own_cursor: bool,
    /// Palette commands used lately, newest first.
    recent_cmds: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let docs = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            root: docs.join("Quiet Notes"),
            font_size: 16.0,
            text_style: TextStyle::Sans,
            wide: false,
            sidebar_width: 264.0,
            expanded: HashSet::new(),
            last_open: None,
            mode: Mode::Notes,
            inv_root: docs.join("Quiet Investigations"),
            last_case: None,
            smooth: true,
            own_cursor: true,
            recent_cmds: Vec::new(),
        }
    }
}

/// Changes to the tree, collected while drawing and applied afterwards.
enum Action {
    Open(PathBuf),
    NewNote(PathBuf),
    NewFolder(PathBuf),
    StartRename(PathBuf),
    Rename(PathBuf, String),
    Delete(PathBuf),
    Move(PathBuf, PathBuf),
    Toggle(PathBuf),
}

struct App {
    s: Settings,
    open: Option<PathBuf>,
    text: String,
    dirty: bool,
    last_edit: Instant,
    renaming: Option<(PathBuf, String, bool)>, // path, buffer, focus requested
    show_settings: bool,
    error: Option<String>,
    focus_editor: bool,
    filter: String,
    inv: inv_ui::Inv,
    /// When the current note or section appeared, for the fade-in.
    shown_at: Instant,
    shown_key: Option<PathBuf>,
    /// Text as last saved, for version history.
    saved_text: String,
    quick: Option<palette::State>,
    focus_mode: bool,
    reading: bool,
    backlinks: (Option<PathBuf>, Instant, Vec<(PathBuf, String)>),
    tags: (Instant, Vec<(String, usize)>),
    hits: Option<(String, HashSet<PathBuf>)>,
    history: Option<(Vec<(PathBuf, String)>, usize)>,
    /// Notes and folders picked with Ctrl/⌘ or a drag frame.
    pick: select::Picker<PathBuf>,
    /// Where you have been, for the back arrow.
    nav: Vec<Nav>,
    nav_cur: Option<Nav>,
}

/// A place in the app the back arrow can return to.
#[derive(Clone, PartialEq)]
enum Nav {
    Note(Option<PathBuf>),
    Case(Option<PathBuf>, inv_ui::View),
}

/// What a palette row does.
#[derive(Clone)]
enum Cmd {
    Note(PathBuf),
    Case(PathBuf),
    Entity(usize),
    NewNote,
    NewFolder,
    NewCase,
    NewEntity(investigation::Kind),
    Tab(inv_ui::View),
    Mode(Mode),
    Back,
    Reading,
    Focus,
    Cursor,
    Settings,
}

fn word_count(t: &str) -> usize {
    t.split_whitespace().count()
}

/// Russian plural: 1 слово, 2 слова, 5 слов.
pub fn plural<'a>(n: usize, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    let (d, h) = (n % 10, n % 100);
    if d == 1 && h != 11 {
        one
    } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
        few
    } else {
        many
    }
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let s: Settings = cc
            .storage
            .and_then(|st| eframe::get_value(st, eframe::APP_KEY))
            .unwrap_or_default();
        Self::with_settings(&cc.egui_ctx, s)
    }

    fn with_settings(ctx: &egui::Context, s: Settings) -> Self {
        theme::apply(ctx);
        egui_extras::install_image_loaders(ctx);
        let _ = std::fs::create_dir_all(&s.root);
        let inv = inv_ui::Inv::new(s.inv_root.clone(), s.last_case.clone());
        let mut app = Self {
            inv,
            s,
            open: None,
            text: String::new(),
            dirty: false,
            last_edit: Instant::now(),
            renaming: None,
            show_settings: false,
            error: None,
            focus_editor: false,
            filter: String::new(),
            shown_at: Instant::now(),
            shown_key: None,
            saved_text: String::new(),
            quick: None,
            focus_mode: false,
            reading: false,
            backlinks: (None, Instant::now(), vec![]),
            tags: (Instant::now() - Duration::from_secs(60), vec![]),
            hits: None,
            history: None,
            pick: Default::default(),
            nav: Vec::new(),
            nav_cur: None,
        };
        if let Some(p) = app.s.last_open.clone() {
            if p.is_file() {
                app.open_note(p);
            }
        }
        app
    }

    fn flush(&mut self) {
        if self.dirty {
            if let Some(p) = self.open.clone() {
                // The path must be a note file. If something turned it into a folder (or it vanished
                // together with its folder), keep the text: save it as a new note instead of failing.
                let target = if p.is_dir() || !p.parent().is_some_and(|d| d.is_dir()) {
                    let dir = if p.is_dir() { p.clone() } else { self.s.root.clone() };
                    match fsops::new_note(&dir) {
                        Ok(np) => {
                            self.error = Some(format!("Файл заметки пропал, текст сохранён в «{}»", fsops::display_name(&np)));
                            self.open = Some(np.clone());
                            self.s.last_open = Some(np.clone());
                            np
                        }
                        Err(e) => {
                            self.error = Some(e);
                            return;
                        }
                    }
                } else {
                    p
                };
                notes::snapshot(&target, &self.saved_text);
                match std::fs::write(&target, &self.text) {
                    Ok(_) => {
                        self.dirty = false;
                        self.saved_text = self.text.clone();
                    }
                    Err(e) => self.error = Some(format!("Не удалось сохранить «{}»: {e}", target.display())),
                }
            }
        }
    }

    fn open_note(&mut self, p: PathBuf) {
        self.flush();
        if !p.is_file() {
            return;
        }
        match std::fs::read_to_string(&p) {
            Ok(t) => {
                self.saved_text = t.clone();
                self.text = t;
                self.open = Some(p.clone());
                self.s.last_open = Some(p);
                self.dirty = false;
                self.focus_editor = true;
            }
            Err(e) => self.error = Some(format!("Не удалось открыть: {e}")),
        }
    }

    fn apply(&mut self, a: Action) {
        let res: Result<(), String> = match a {
            Action::Open(p) => {
                self.open_note(p);
                Ok(())
            }
            Action::Toggle(p) => {
                if !self.s.expanded.remove(&p) {
                    self.s.expanded.insert(p);
                }
                Ok(())
            }
            Action::NewNote(dir) => fsops::new_note(&dir).map(|p| {
                self.s.expanded.insert(dir);
                self.open_note(p.clone());
                self.renaming = Some((p, String::new(), false));
            }),
            Action::NewFolder(dir) => fsops::new_folder(&dir).map(|p| {
                self.s.expanded.insert(dir);
                self.renaming = Some((p, "Новая папка".into(), false));
            }),
            Action::StartRename(p) => {
                self.renaming = Some((p.clone(), fsops::display_name(&p), false));
                Ok(())
            }
            Action::Rename(p, name) => {
                self.renaming = None;
                self.flush();
                fsops::rename(&p, &name).map(|np| {
                    notes::move_history(&p, &np);
                    self.rebase(&p, &np)
                })
            }
            Action::Delete(p) => {
                self.flush();
                fsops::delete(&p).map(|_| {
                    if self.open.as_ref().is_some_and(|o| o.starts_with(&p)) {
                        self.open = None;
                        self.text.clear();
                    }
                })
            }
            Action::Move(src, dir) => {
                self.flush();
                fsops::move_into(&src, &dir).map(|np| {
                    notes::move_history(&src, &np);
                    self.s.expanded.insert(dir);
                    self.rebase(&src, &np);
                })
            }
        };
        if let Err(e) = res {
            self.error = Some(e);
        }
    }

    /// Keep the open note and expanded folders pointing at the right place after a move/rename.
    fn rebase(&mut self, old: &Path, new: &Path) {
        let fix = |p: &PathBuf| p.strip_prefix(old).map(|rest| new.join(rest)).ok();
        if let Some(np) = self.open.as_ref().and_then(fix) {
            self.open = Some(np.clone());
            self.s.last_open = Some(np);
        }
        self.s.expanded = self.s.expanded.iter().map(|p| fix(p).unwrap_or_else(|| p.clone())).collect();
    }

    fn tree(&mut self, ui: &mut egui::Ui, dir: &Path, depth: usize, actions: &mut Vec<Action>) {
        let filter = self.filter.trim().to_lowercase();
        for entry in fsops::list(dir) {
            let is_dir = entry.is_dir();
            if !filter.is_empty() && !self.hits.as_ref().is_some_and(|(_, h)| h.contains(&entry)) {
                continue;
            }
            let expanded = self.s.expanded.contains(&entry) || (!filter.is_empty() && is_dir);
            let selected = self.open.as_ref() == Some(&entry);
            let indent = 12.0 + depth as f32 * 16.0;

            // Inline rename
            if let Some((rp, buf, focused)) = &mut self.renaming {
                if *rp == entry {
                    let mut done = None;
                    ui.horizontal(|ui| {
                        ui.add_space(indent + 30.0);
                        let r = ui.add(
                            egui::TextEdit::singleline(buf)
                                .hint_text("Название")
                                .desired_width(ui.available_width() - 26.0),
                        );
                        if !*focused {
                            r.request_focus();
                            *focused = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            done = Some(None);
                        } else if r.lost_focus() {
                            done = Some(Some(buf.clone()));
                        }
                    });
                    match done {
                        Some(Some(name)) => actions.push(Action::Rename(entry.clone(), name)),
                        Some(None) => self.renaming = None,
                        None => {}
                    }
                    if is_dir && expanded {
                        self.tree(ui, &entry, depth + 1, actions);
                    }
                    continue;
                }
            }

            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click_and_drag());
            self.pick.row(entry.clone(), rect);
            let multi = select::multi(ui);
            if !multi && !self.pick.dragging() {
                resp.dnd_set_drag_payload(entry.clone());
            }
            let picked = self.pick.is_picked(&entry);

            let drop_target = if is_dir { entry.clone() } else { dir.to_path_buf() };
            let hovering = resp.dnd_hover_payload::<PathBuf>().is_some();
            if let Some(src) = resp.dnd_release_payload::<PathBuf>() {
                for p in self.drag_set(&src) {
                    if !drop_target.starts_with(&p) {
                        actions.push(Action::Move(p, drop_target.clone()));
                    }
                }
            }

            let row = rect.shrink2(egui::vec2(8.0, 1.0));
            let p = ui.painter();
            let t_hover = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.14);
            let t_sel = ui.ctx().animate_bool_with_time(resp.id.with("s"), selected, 0.22);
            if hovering && is_dir {
                p.rect(row, 6.0, theme::DROP, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            } else if picked {
                select::paint_picked(ui, row);
            } else {
                let base = theme::lerp(theme::SIDEBAR, theme::HOVER, t_hover);
                let fill = theme::lerp(base, theme::SELECTED, t_sel);
                if t_hover > 0.0 || t_sel > 0.0 {
                    p.rect_filled(row, 6.0, fill);
                }
                if t_sel > 0.0 && !is_dir {
                    let bar = egui::Rect::from_min_size(row.left_top() + egui::vec2(0.0, 6.0), egui::vec2(2.0, (row.height() - 12.0) * t_sel));
                    p.rect_filled(bar, 1.0, theme::ACCENT.gamma_multiply(t_sel));
                }
            }
            let y = rect.center().y;
            let x0 = rect.left() + indent;
            if is_dir {
                let caret = if expanded { ic::CARET_DOWN } else { ic::CARET_RIGHT };
                p.text(egui::pos2(x0, y), egui::Align2::LEFT_CENTER, caret, theme::font(10.0), theme::FAINT);
            }
            let (icon, icol) = if is_dir {
                (if expanded { ic::FOLDER_OPEN } else { ic::FOLDER }, theme::MUTED)
            } else {
                (ic::FILE_TEXT, if selected { theme::ACCENT } else { theme::FAINT })
            };
            p.text(egui::pos2(x0 + 14.0, y), egui::Align2::LEFT_CENTER, icon, theme::font(15.0), icol);
            let color = if selected { theme::TEXT } else { theme::MUTED };
            let name = fsops::display_name(&entry);
            let text_x = x0 + 36.0;
            ui.painter().with_clip_rect(row.shrink2(egui::vec2(4.0, 0.0))).text(
                egui::pos2(text_x, y),
                egui::Align2::LEFT_CENTER,
                name,
                theme::font(13.5),
                color,
            );

            if resp.clicked() && multi {
                self.pick.toggle(entry.clone());
            } else if resp.clicked() {
                self.pick.clear();
                actions.push(if is_dir { Action::Toggle(entry.clone()) } else { Action::Open(entry.clone()) });
            }
            if resp.double_clicked() && !multi {
                actions.push(Action::StartRename(entry.clone()));
            }
            let parent_for_new = if is_dir { entry.clone() } else { dir.to_path_buf() };
            let group = picked && self.pick.picked.len() > 1;
            let group_set = if group { self.drag_set(&entry) } else { Vec::new() };
            let mut clear_picks = false;
            resp.context_menu(|ui| {
                if group {
                    ui.set_min_width(190.0);
                    ui.label(RichText::new(format!("Выбрано: {}", group_set.len())).size(12.0).color(theme::FAINT));
                    ui.separator();
                    if ui.button(format!("{}  Снять выделение", ic::X)).clicked() {
                        clear_picks = true;
                        ui.close();
                    }
                    if ui.button(RichText::new(format!("{}  Все в корзину", ic::TRASH)).color(theme::DANGER)).clicked() {
                        for p in &group_set {
                            actions.push(Action::Delete(p.clone()));
                        }
                        clear_picks = true;
                        ui.close();
                    }
                    return;
                }
                ui.set_min_width(190.0);
                if ui.button(format!("{}  Новая заметка", ic::NOTE_PENCIL)).clicked() {
                    actions.push(Action::NewNote(parent_for_new.clone()));
                    ui.close();
                }
                if ui.button(format!("{}  Новая папка", ic::FOLDER_PLUS)).clicked() {
                    actions.push(Action::NewFolder(parent_for_new.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button(format!("{}  Переименовать", ic::PENCIL_SIMPLE)).clicked() {
                    actions.push(Action::StartRename(entry.clone()));
                    ui.close();
                }
                if ui.button(format!("{}  {}", ic::ARROW_SQUARE_OUT, fsops::FILE_MANAGER)).clicked() {
                    fsops::open_external(&entry.display().to_string(), true);
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new(format!("{}  В корзину", ic::TRASH)).color(theme::DANGER)).clicked() {
                    actions.push(Action::Delete(entry.clone()));
                    ui.close();
                }
            });
            if clear_picks {
                self.pick.clear();
            }

            if is_dir && expanded {
                self.tree(ui, &entry, depth + 1, actions);
            }
        }
    }

    /// What a drag or group action works on: every picked item if `src` is picked,
    /// otherwise just `src`. Items inside a picked folder go along with it.
    fn drag_set(&self, src: &Path) -> Vec<PathBuf> {
        if !self.pick.is_picked(&src.to_path_buf()) {
            return vec![src.to_path_buf()];
        }
        let all = &self.pick.picked;
        let mut v: Vec<PathBuf> = all
            .iter()
            .filter(|p| p.exists() && !all.iter().any(|q| q != *p && p.starts_with(q)))
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// Which notes and folders match the search: names, text, or `#tag`.
    fn refresh_hits(&mut self) {
        let f = self.filter.trim().to_lowercase();
        if f.is_empty() {
            self.hits = None;
            return;
        }
        if self.hits.as_ref().is_some_and(|(q, _)| *q == f) {
            return;
        }
        let mut set = HashSet::new();
        for p in notes::all_notes(&self.s.root) {
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let ok = match f.strip_prefix('#') {
                Some(tag) => notes::tags(&text).iter().any(|t| t.starts_with(tag)),
                None => notes::title(&p).to_lowercase().contains(&f) || text.to_lowercase().contains(&f),
            };
            if ok {
                let mut cur = p.clone();
                set.insert(p);
                while let Some(parent) = cur.parent() {
                    if parent == self.s.root || !parent.starts_with(&self.s.root) {
                        break;
                    }
                    set.insert(parent.to_path_buf());
                    cur = parent.to_path_buf();
                }
            }
        }
        self.hits = Some((f, set));
    }

    fn tag_bar(&mut self, ui: &mut egui::Ui) {
        if self.tags.0.elapsed() > Duration::from_secs(4) {
            self.tags = (Instant::now(), notes::all_tags(&self.s.root));
        }
        if self.tags.1.is_empty() {
            return;
        }
        let tags: Vec<_> = self.tags.1.iter().take(14).cloned().collect();
        ui.horizontal_wrapped(|ui| {
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
            for (t, n) in tags {
                let q = format!("#{t}");
                let on = self.filter.trim().eq_ignore_ascii_case(&q);
                let txt = RichText::new(format!("#{t}")).size(11.5).color(if on { theme::TEXT } else { theme::ACCENT });
                let b = egui::Button::new(txt).fill(if on { theme::DROP } else { theme::ACCENT_BG }).stroke(Stroke::NONE).corner_radius(10);
                if ui.add(b).on_hover_text(format!("{n} {}", plural(n, "заметка", "заметки", "заметок"))).clicked() {
                    self.filter = if on { String::new() } else { q };
                }
            }
        });
        ui.add_space(10.0);
    }

    fn mode_switch(&mut self, ui: &mut egui::Ui) {
        let full = egui::Rect::from_min_size(ui.cursor().min + egui::vec2(12.0, 0.0), egui::vec2(ui.available_width() - 24.0, 34.0));
        ui.allocate_rect(full, Sense::hover());
        ui.painter().rect_filled(full, 9.0, theme::SURFACE);
        let half = full.width() / 2.0;
        for (k, (m, icon, label)) in
            [(Mode::Notes, ic::NOTE, "Заметки"), (Mode::Investigations, ic::DETECTIVE, "Расследования")].into_iter().enumerate()
        {
            let r = egui::Rect::from_min_size(full.min + egui::vec2(half * k as f32, 0.0), egui::vec2(half, full.height())).shrink(3.0);
            let resp = ui.interact(r, ui.id().with(("mode", k)), Sense::click());
            let on = self.s.mode == m;
            if on {
                ui.painter().rect(r, 7.0, theme::SELECTED, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 7.0, theme::HOVER);
            }
            let col = if on { theme::TEXT } else { theme::FAINT };
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, format!("{icon}  {label}"), theme::font(12.0), col);
            if resp.clicked() && !on {
                self.flush();
                self.inv.flush();
                self.s.mode = m;
            }
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        egui::SidePanel::left("tree")
            .resizable(true)
            .default_width(self.s.sidebar_width)
            .width_range(210.0..=480.0)
            .show_separator_line(true)
            .frame(egui::Frame::NONE.fill(theme::SIDEBAR).inner_margin(egui::Margin::symmetric(0, 14)))
            .show(ctx, |ui| {
                self.s.sidebar_width = ui.available_width();
                self.mode_switch(ui);
                ui.add_space(14.0);

                egui::TopBottomPanel::bottom("sidebar_footer")
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 2)))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            if theme::icon_button(ui, ic::GEAR_SIX, "Настройки").clicked() {
                                self.show_settings = !self.show_settings;
                            }
                            let root = if self.s.mode == Mode::Notes { &self.s.root } else { &self.inv.root };
                            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.label(RichText::new(name).size(11.5).color(theme::FAINT)).on_hover_text(root.display().to_string());
                        });
                    });

                if self.s.mode == Mode::Investigations {
                    self.inv.sidebar(ui);
                    return;
                }

                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let w = ui.available_width() - 26.0;
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text(format!("{}  Поиск", ic::MAGNIFYING_GLASS))
                            .desired_width(w)
                            .margin(egui::vec2(9.0, 6.0)),
                    );
                });
                ui.add_space(12.0);
                self.refresh_hits();
                self.tag_bar(ui);

                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    theme::caption(ui, "Заметки");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        if theme::icon_button(ui, ic::FOLDER_PLUS, &theme::keys("Новая папка  ⇧⌘N")).clicked() {
                            actions.push(Action::NewFolder(self.s.root.clone()));
                        }
                        if theme::icon_button(ui, ic::NOTE_PENCIL, &theme::keys("Новая заметка  ⌘N")).clicked() {
                            actions.push(Action::NewNote(self.s.root.clone()));
                        }
                    });
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let root = self.s.root.clone();
                    let area = ui.clip_rect();
                    self.pick.begin();
                    self.tree(ui, &root, 0, actions);

                    // Empty space below the tree: drop here to move to the root, right-click to create.
                    let (rect, resp) = ui.allocate_exact_size(ui.available_size().max(egui::vec2(0.0, 60.0)), Sense::click());
                    if resp.dnd_hover_payload::<PathBuf>().is_some() {
                        ui.painter().rect_stroke(rect.shrink(8.0), 6.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                        ui.painter().text(rect.center_top() + egui::vec2(0.0, 26.0), egui::Align2::CENTER_CENTER, "В корень", theme::font(12.0), theme::ACCENT);
                    }
                    if let Some(src) = resp.dnd_release_payload::<PathBuf>() {
                        for p in self.drag_set(&src) {
                            actions.push(Action::Move(p, root.clone()));
                        }
                    }
                    if resp.clicked() && !select::multi(ui) {
                        self.pick.clear();
                    }
                    resp.context_menu(|ui| {
                        if ui.button(format!("{}  Новая заметка", ic::NOTE_PENCIL)).clicked() {
                            actions.push(Action::NewNote(root.clone()));
                            ui.close();
                        }
                        if ui.button(format!("{}  Новая папка", ic::FOLDER_PLUS)).clicked() {
                            actions.push(Action::NewFolder(root.clone()));
                            ui.close();
                        }
                    });
                    self.pick.finish(ui, area);
                });
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new(RichText::new("Настройки").size(15.0))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([400.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                egui::Grid::new("settings").num_columns(2).spacing([18.0, 14.0]).show(ui, |ui| {
                    ui.label(RichText::new("Размер текста").color(theme::MUTED));
                    ui.add(egui::Slider::new(&mut self.s.font_size, 12.0..=28.0).step_by(1.0).fixed_decimals(0));
                    ui.end_row();
                    ui.label(RichText::new("Шрифт").color(theme::MUTED));
                    egui::ComboBox::from_id_salt("font").width(170.0).selected_text(self.s.text_style.label()).show_ui(ui, |ui| {
                        for t in [TextStyle::Sans, TextStyle::Serif, TextStyle::Mono] {
                            ui.selectable_value(&mut self.s.text_style, t, t.label());
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new("Ширина текста").color(theme::MUTED));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.s.wide, false, "Колонка");
                        ui.selectable_value(&mut self.s.wide, true, "Во всю ширину");
                    });
                    ui.end_row();
                    ui.label(RichText::new("Плавность").color(theme::MUTED));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.s.smooth, true, "Включена");
                        ui.selectable_value(&mut self.s.smooth, false, "Выключена");
                    });
                    ui.end_row();
                    ui.label(RichText::new("Курсор").color(theme::MUTED));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.s.own_cursor, true, "Прицел");
                        ui.selectable_value(&mut self.s.own_cursor, false, "Системный");
                    });
                    ui.end_row();
                });
                ui.separator();
                for (label, notes) in [("Папка заметок", true), ("Папка расследований", false)] {
                    let path = if notes { self.s.root.clone() } else { self.inv.root.clone() };
                    ui.horizontal(|ui| {
                        let w = ui.available_width() - 110.0;
                        ui.vertical(|ui| {
                            ui.set_max_width(w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(label).color(theme::MUTED));
                            let shown = match dirs::home_dir().and_then(|h| path.strip_prefix(h).ok().map(|r| format!("~/{}", r.display()))) {
                                Some(s) => s,
                                None => path.display().to_string(),
                            };
                            ui.add(egui::Label::new(RichText::new(shown).size(11.5).color(theme::FAINT)).truncate())
                                .on_hover_text(path.display().to_string());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Изменить…").clicked() {
                                if let Some(dir) = rfd::FileDialog::new().set_directory(&path).pick_folder() {
                                    if notes {
                                        self.flush();
                                        self.s.root = dir;
                                        self.s.expanded.clear();
                                        self.open = None;
                                        self.text.clear();
                                    } else {
                                        self.inv.set_root(dir);
                                    }
                                }
                            }
                        });
                    });
                }
            });
        self.show_settings = open;
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = self.error.clone() {
            error_bar(ui, &e, &mut self.error);
        }
        let Some(path) = self.open.clone() else {
            if !self.nav.is_empty() {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if back_button(ui) {
                        self.back();
                    }
                });
            }
            empty_state(ui, ic::NOTE_PENCIL, "Пустой лист", &theme::keys("⌘N — новая заметка,  ⌘P — найти заметку"));
            return;
        };

        // Top bar: breadcrumbs and note tools. Hidden in focus mode.
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            if self.focus_mode {
                ui.label(RichText::new("Esc — выйти из режима фокуса").size(11.5).color(theme::FAINT.gamma_multiply(0.7)));
                return;
            }
            if !self.nav.is_empty() {
                ui.add_space(-8.0);
                if back_button(ui) {
                    self.back();
                }
                ui.add_space(4.0);
            }
            let rel = path.strip_prefix(&self.s.root).unwrap_or(&path).with_extension("");
            let parts: Vec<String> = rel.iter().map(|c| c.to_string_lossy().into_owned()).collect();
            for (i, part) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                ui.label(RichText::new(part).size(12.5).color(if last { theme::MUTED } else { theme::FAINT }));
                if !last {
                    ui.label(RichText::new(ic::CARET_RIGHT).size(10.0).color(theme::FAINT));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                let r = theme::icon_button(ui, ic::EXPORT, "Экспорт");
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(170.0);
                    if ui.button(format!("{}  В PDF…", ic::FILE_PDF)).clicked() {
                        self.export(&path, true);
                        ui.close();
                    }
                    if ui.button(format!("{}  В HTML…", ic::FILE_HTML)).clicked() {
                        self.export(&path, false);
                        ui.close();
                    }
                });
                if theme::icon_button(ui, ic::CLOCK_COUNTER_CLOCKWISE, "История версий").clicked() {
                    self.flush();
                    self.history = Some((notes::versions(&path), 0));
                }
                if theme::icon_button(ui, ic::CORNERS_OUT, &theme::keys("Режим фокуса  ⇧⌘F")).clicked() {
                    self.focus_mode = true;
                }
                let (icon, tip) = if self.reading { (ic::PENCIL_SIMPLE, "Писать  ⌘E") } else { (ic::BOOK_OPEN, "Читать  ⌘E") };
                if theme::icon_button(ui, icon, &theme::keys(tip)).clicked() {
                    self.reading = !self.reading;
                }
            });
        });

        if !self.focus_mode {
            egui::TopBottomPanel::bottom("status")
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 8)))
                .show_separator_line(false)
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        let w = word_count(&self.text);
                        let c = self.text.chars().count();
                        let info = format!("{w} {}  ·  {c} {}", plural(w, "слово", "слова", "слов"), plural(c, "символ", "символа", "символов"));
                        ui.label(RichText::new(info).size(11.5).color(theme::FAINT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let t = if self.dirty { "Сохраняется…" } else { "Сохранено" };
                            ui.label(RichText::new(t).size(11.5).color(theme::FAINT));
                            if !self.reading {
                                ui.add_space(14.0);
                                ui.label(RichText::new(theme::keys("⌘ + клик — открыть ссылку")).size(11.5).color(theme::FAINT.gamma_multiply(0.7)));
                            }
                        });
                    });
                });
        }

        // Fade the page in when another note opens.
        if self.shown_key.as_ref() != Some(&path) {
            self.shown_key = Some(path.clone());
            self.shown_at = Instant::now();
        }
        let fade = if self.s.smooth { writer::ease(self.shown_at.elapsed().as_secs_f32() / 0.32) } else { 1.0 };
        if fade < 1.0 {
            ui.ctx().request_repaint();
        }
        ui.set_opacity(fade);

        if self.backlinks.0.as_ref() != Some(&path) || self.backlinks.1.elapsed() > Duration::from_secs(5) {
            self.backlinks = (Some(path.clone()), Instant::now(), notes::backlinks(&self.s.root, &path));
        }

        let editor_id = egui::Id::new("note-editor");
        let focus_range = if self.focus_mode && !self.reading {
            egui::TextEdit::load_state(ui.ctx(), editor_id).and_then(|st| st.cursor.char_range()).map(|r| {
                let b = self.text.char_indices().nth(r.primary.index).map(|(i, _)| i).unwrap_or(self.text.len());
                let start = self.text[..b].rfind("\n\n").map(|i| i + 2).unwrap_or(0);
                let end = self.text[b..].find("\n\n").map(|i| b + i).unwrap_or(self.text.len());
                start..end
            })
        } else {
            None
        };

        let mut open_link: Option<String> = None;
        egui::ScrollArea::vertical().auto_shrink(false).animated(self.s.smooth).show(ui, |ui| {
            let avail = ui.available_width();
            let col = if self.s.wide { avail - 120.0 } else { (self.s.font_size * 38.0).min(avail - 96.0) };
            let side = ((avail - col) / 2.0).max(28.0);
            let lift = (1.0 - fade) * 10.0;
            let top = if self.focus_mode { 90.0 } else { 40.0 } + lift;
            let bl_h = if self.backlinks.2.is_empty() { 0.0 } else { 60.0 + 30.0 * self.backlinks.2.len().min(6) as f32 };

            if self.reading {
                open_link = self.reading_view(ui, &path, side, top);
            } else {
                let look = writer::Look { size: self.s.font_size, style: self.s.text_style, focus: focus_range.clone(), reading: false, title: true };
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| ui.fonts_mut(|f| f.layout_job(writer::layout(text.as_str(), &look, wrap)));
                let min_h = (ui.available_height() - bl_h).max(200.0);
                let text_w = (avail - 2.0 * side).max(200.0);
                let out = ui.horizontal(|ui| {
                    ui.add_space(side);
                    egui::TextEdit::multiline(&mut self.text)
                    .id(editor_id)
                    .frame(false)
                    .layouter(&mut layouter)
                    .hint_text(RichText::new("Заголовок\n\nНачните писать…  **жирный**, *курсив*, # заголовок, - список, > цитата, [[ссылка]], #тег").color(theme::FAINT))
                    .margin(egui::Margin { left: 0, right: 0, top: top as i8, bottom: 60 })
                    .lock_focus(true)
                    .desired_width(text_w)
                    .min_size(egui::vec2(text_w, min_h))
                    .show(ui)
                })
                .inner;
                if self.focus_editor {
                    out.response.request_focus();
                    self.focus_editor = false;
                }
                if out.response.changed() {
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
                if ui.input(|i| i.modifiers.command) && out.response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if out.response.clicked() && ui.input(|i| i.modifiers.command) {
                    if let Some(pos) = out.response.interact_pointer_pos() {
                        let c = out.galley.cursor_from_pos(pos - out.galley_pos);
                        let b = self.text.char_indices().nth(c.index).map(|(i, _)| i).unwrap_or(self.text.len());
                        open_link = notes::links(&self.text).into_iter().find(|(r, _)| r.contains(&b)).map(|(_, t)| t);
                    }
                }
            }

            if !self.backlinks.2.is_empty() && !self.focus_mode {
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(col.min(avail - 2.0 * side));
                        ui.separator();
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("{}  Ссылаются на эту заметку", ic::LINK)).size(12.0).color(theme::FAINT));
                        ui.add_space(4.0);
                        for (p, line) in self.backlinks.2.clone().into_iter().take(6) {
                            let r = ui.add(egui::Label::new(RichText::new(format!("{}  ", notes::title(&p))).color(theme::ACCENT).size(13.0)).sense(Sense::click()));
                            let r = r.on_hover_text(line).on_hover_cursor(egui::CursorIcon::PointingHand);
                            if r.clicked() {
                                self.open_note(p);
                            }
                        }
                        ui.add_space(30.0);
                    });
                });
            }
        });
        if let Some(t) = open_link {
            self.follow_link(&path, &t);
        }
    }

    /// Reading mode: Markdown markers hidden, images shown, links open on click.
    fn reading_view(&mut self, ui: &mut egui::Ui, note: &Path, side: f32, top: f32) -> Option<String> {
        let mut clicked = None;
        let wrap = ui.available_width() - 2.0 * side;
        ui.add_space(top);
        // Split the note at image lines.
        let mut segments: Vec<(String, Option<PathBuf>)> = vec![];
        let mut buf = String::new();
        for line in self.text.split_inclusive('\n') {
            if let Some(img) = notes::image_line(note, line) {
                segments.push((std::mem::take(&mut buf), None));
                segments.push((String::new(), Some(img)));
            } else {
                buf.push_str(line);
            }
        }
        segments.push((buf, None));
        let mut first = true;
        for (text, img) in segments {
            ui.horizontal(|ui| {
                ui.add_space(side);
                if let Some(p) = img {
                    ui.add(egui::Image::new(format!("file://{}", p.display())).max_width(wrap).max_height(560.0).corner_radius(8));
                    return;
                }
                if text.trim().is_empty() && !first {
                    return;
                }
                let look = writer::Look { size: self.s.font_size, style: self.s.text_style, focus: None, reading: true, title: first };
                first = false;
                let (job, links) = writer::build(text.trim_end_matches('\n'), &look, wrap);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
                if let Some(pos) = resp.hover_pos() {
                    let c = galley.cursor_from_pos(pos - rect.min);
                    let b = galley.job.text.char_indices().nth(c.index).map(|(i, _)| i).unwrap_or(0);
                    if let Some((_, t)) = links.iter().find(|(r, _)| r.contains(&b)) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            clicked = Some(t.clone());
                        }
                    }
                }
                ui.painter().galley(rect.min, galley, theme::TEXT);
            });
            ui.add_space(10.0);
        }
        ui.add_space(60.0);
        clicked
    }

    /// Opens the note a [[link]] names, creating it next to the current note if there is none.
    fn follow_link(&mut self, from: &Path, target: &str) {
        if let Some(p) = notes::resolve(&self.s.root, target) {
            self.open_note(p);
            return;
        }
        let name = target.split('|').next().unwrap_or(target).trim();
        if fsops::display_name(Path::new(name)).is_empty() || name.contains(['/', '\\']) {
            self.error = Some(format!("Не удалось создать заметку «{name}»"));
            return;
        }
        let dir = from.parent().map(Path::to_path_buf).unwrap_or_else(|| self.s.root.clone());
        let p = dir.join(format!("{name}.md"));
        match std::fs::write(&p, format!("{name}\n\n")) {
            Ok(()) => self.open_note(p),
            Err(e) => self.error = Some(format!("Не удалось создать заметку: {e}")),
        }
    }

    fn export(&mut self, note: &Path, pdf: bool) {
        self.flush();
        let name = notes::title(note);
        let (ext, label) = if pdf { ("pdf", "PDF") } else { ("html", "HTML") };
        let Some(out) = rfd::FileDialog::new().set_file_name(format!("{name}.{ext}")).add_filter(label, &[ext]).save_file() else { return };
        let res = if pdf {
            pdf::write(&out, &name, &pdf::from_markdown(&self.text))
        } else {
            std::fs::write(&out, notes::to_html(note, &self.text)).map_err(|e| format!("Не удалось сохранить: {e}"))
        };
        if let Err(e) = res {
            self.error = Some(e);
        }
    }

    /// Inserts dropped images into the open note at the cursor.
    fn drop_images(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if files.is_empty() || self.s.mode != Mode::Notes {
            return;
        }
        let Some(note) = self.open.clone() else { return };
        let mut insert = String::new();
        for f in files {
            if !notes::is_image(&f) {
                self.error = Some("В заметку можно перетащить картинку: PNG, JPG, GIF или WebP".into());
                continue;
            }
            match notes::attach_image(&note, &f) {
                Ok(md) => insert += &format!("\n{md}\n"),
                Err(e) => self.error = Some(e),
            }
        }
        if insert.is_empty() {
            return;
        }
        let at = egui::TextEdit::load_state(ctx, egui::Id::new("note-editor"))
            .and_then(|st| st.cursor.char_range())
            .and_then(|r| self.text.char_indices().nth(r.primary.index).map(|(i, _)| i))
            .unwrap_or(self.text.len());
        let at = self.text[at..].find('\n').map(|i| at + i).unwrap_or(self.text.len());
        self.text.insert_str(at, &insert);
        self.dirty = true;
        self.last_edit = Instant::now();
    }

    fn palette_items(&self) -> Vec<palette::Item<Cmd>> {
        use investigation::Kind;
        use inv_ui::View;
        let mut v = Vec::new();
        let mut add = |icon, color, label: String, hint: &str, key: String, cmd| {
            v.push(palette::Item { icon, color, label, hint: hint.to_string(), key, cmd });
        };
        for p in notes::all_notes(&self.s.root) {
            let rel = p.strip_prefix(&self.s.root).unwrap_or(&p).with_extension("").display().to_string();
            add(ic::FILE_TEXT, theme::MUTED, rel.clone(), "заметка", format!("note:{rel}"), Cmd::Note(p));
        }
        for d in investigation::list_cases(&self.inv.root) {
            let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            add(ic::DETECTIVE, theme::ACCENT, name.clone(), "дело", format!("case:{name}"), Cmd::Case(d));
        }
        if let Some(c) = &self.inv.case {
            for (i, e) in c.entities.iter().enumerate() {
                let hint = format!("{} в деле", e.kind.label().to_lowercase());
                add(inv_ui::kind_icon(e.kind), e.kind.color(), e.name.clone(), &hint, format!("ent:{}", e.name), Cmd::Entity(i));
            }
            for (view, icon, name) in [(View::Overview, ic::SQUARES_FOUR, "Обзор"), (View::Graph, ic::GRAPH, "Граф"), (View::Timeline, ic::CLOCK_COUNTER_CLOCKWISE, "Хронология"), (View::Map, ic::MAP_TRIFOLD, "Карта")] {
                add(icon, theme::FAINT, format!("Открыть: {name}"), "вкладка дела", format!("tab:{name}"), Cmd::Tab(view));
            }
            for k in Kind::ALL {
                add(ic::PLUS, k.color(), format!("Новая карточка: {}", k.label()), "команда", format!("newent:{}", k.key()), Cmd::NewEntity(k));
            }
        }
        add(ic::NOTE_PENCIL, theme::FAINT, "Новая заметка".into(), "⌘N", "cmd:newnote".into(), Cmd::NewNote);
        add(ic::FOLDER_PLUS, theme::FAINT, "Новая папка".into(), "⇧⌘N", "cmd:newfolder".into(), Cmd::NewFolder);
        add(ic::DETECTIVE, theme::FAINT, "Новое расследование".into(), "команда", "cmd:newcase".into(), Cmd::NewCase);
        add(ic::NOTEBOOK, theme::FAINT, "Перейти в заметки".into(), "раздел", "cmd:notes".into(), Cmd::Mode(Mode::Notes));
        add(ic::DETECTIVE, theme::FAINT, "Перейти в расследования".into(), "раздел", "cmd:invs".into(), Cmd::Mode(Mode::Investigations));
        if !self.nav.is_empty() {
            add(ic::ARROW_LEFT, theme::FAINT, "Назад".into(), "⌘[", "cmd:back".into(), Cmd::Back);
        }
        if self.open.is_some() {
            add(ic::BOOK_OPEN, theme::FAINT, "Режим чтения".into(), "⌘E", "cmd:read".into(), Cmd::Reading);
            add(ic::ARROWS_OUT, theme::FAINT, "Режим фокуса".into(), "⇧⌘F", "cmd:focus".into(), Cmd::Focus);
        }
        let cur = if self.s.own_cursor { "Курсор: системный" } else { "Курсор: прицел" };
        add(ic::CROSSHAIR, theme::FAINT, cur.into(), "настройка", "cmd:cursor".into(), Cmd::Cursor);
        add(ic::GEAR, theme::FAINT, "Настройки".into(), "команда", "cmd:settings".into(), Cmd::Settings);
        v
    }

    fn run(&mut self, cmd: Cmd, actions: &mut Vec<Action>) {
        let here = self.open.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| self.s.root.clone());
        match cmd {
            Cmd::Note(p) => {
                self.s.mode = Mode::Notes;
                self.open_note(p);
            }
            Cmd::Case(d) => {
                self.s.mode = Mode::Investigations;
                self.inv.open_case(&d);
            }
            Cmd::Entity(i) => {
                self.s.mode = Mode::Investigations;
                self.inv.go(inv_ui::View::Entity(i));
            }
            Cmd::NewNote => {
                self.s.mode = Mode::Notes;
                actions.push(Action::NewNote(here));
            }
            Cmd::NewFolder => {
                self.s.mode = Mode::Notes;
                actions.push(Action::NewFolder(here));
            }
            Cmd::NewCase => {
                self.s.mode = Mode::Investigations;
                self.inv.new_case();
            }
            Cmd::NewEntity(k) => self.inv.new_entity(k),
            Cmd::Tab(v) => {
                self.s.mode = Mode::Investigations;
                self.inv.go(v);
            }
            Cmd::Mode(m) => self.s.mode = m,
            Cmd::Back => self.back(),
            Cmd::Reading => self.reading = !self.reading,
            Cmd::Focus => self.focus_mode = !self.focus_mode,
            Cmd::Cursor => self.s.own_cursor = !self.s.own_cursor,
            Cmd::Settings => self.show_settings = true,
        }
    }

    fn quick_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(st) = &mut self.quick else { return };
        let query = st.query.clone();
        let items = palette::rank(self.palette_items(), &query, &self.s.recent_cmds, 40);
        let st = self.quick.as_mut().unwrap();
        match palette::show(ctx, st, &items) {
            palette::Outcome::None => {}
            palette::Outcome::Close => self.quick = None,
            palette::Outcome::Run(k) => {
                self.quick = None;
                let it = &items[k];
                palette::remember(&mut self.s.recent_cmds, &it.key);
                self.flush();
                self.run(it.cmd.clone(), actions);
            }
            palette::Outcome::Create(name) => {
                self.quick = None;
                self.s.mode = Mode::Notes;
                let from = self.open.clone().unwrap_or_else(|| self.s.root.join("x.md"));
                self.follow_link(&from, &name);
            }
        }
    }

    fn history_window(&mut self, ctx: &egui::Context) {
        let Some((list, sel)) = &mut self.history else { return };
        let mut open = true;
        let mut restore = None;
        egui::Window::new(RichText::new("История версий").size(15.0))
            .open(&mut open)
            .collapsible(false)
            .default_size([760.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                if list.is_empty() {
                    ui.label(RichText::new("Копий пока нет. Они сохраняются сами, не чаще раза в 5 минут, пока вы пишете.").color(theme::FAINT));
                    return;
                }
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(190.0);
                        egui::ScrollArea::vertical().id_salt("hist-list").max_height(440.0).show(ui, |ui| {
                            for (k, (_, label)) in list.iter().enumerate() {
                                if ui.selectable_label(*sel == k, RichText::new(label).size(13.0)).clicked() {
                                    *sel = k;
                                }
                            }
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        let text = std::fs::read_to_string(&list[*sel].0).unwrap_or_default();
                        egui::ScrollArea::vertical().id_salt("hist-text").max_height(400.0).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(&text).size(13.5).color(theme::MUTED)).wrap());
                        });
                        ui.add_space(8.0);
                        if theme::pill(ui, ic::ARROW_COUNTER_CLOCKWISE, "Восстановить эту версию").clicked() {
                            restore = Some(text);
                        }
                    });
                });
            });
        if let Some(t) = restore {
            if let Some(p) = self.open.clone() {
                notes::snapshot(&p, &self.text); // the current text stays in history too
            }
            self.text = t;
            self.dirty = true;
            self.last_edit = Instant::now();
            open = false;
        }
        if !open {
            self.history = None;
        }
    }

    fn nav_now(&self) -> Nav {
        match self.s.mode {
            Mode::Notes => Nav::Note(self.open.clone()),
            Mode::Investigations => Nav::Case(self.inv.current_dir(), self.inv.view),
        }
    }

    /// Remember the previous place whenever you move somewhere new.
    fn track_nav(&mut self) {
        let now = self.nav_now();
        if let Some(prev) = self.nav_cur.take() {
            if prev != now {
                self.nav.push(prev);
                if self.nav.len() > 100 {
                    self.nav.remove(0);
                }
            }
        }
        self.nav_cur = Some(now);
    }

    fn back(&mut self) {
        let Some(to) = self.nav.pop() else { return };
        self.flush();
        self.inv.flush();
        match &to {
            Nav::Note(p) => {
                self.s.mode = Mode::Notes;
                match p {
                    Some(p) if p.is_file() => self.open_note(p.clone()),
                    Some(_) => return self.back(),
                    None => {
                        self.open = None;
                        self.text.clear();
                    }
                }
            }
            Nav::Case(dir, view) => {
                self.s.mode = Mode::Investigations;
                if !self.inv.restore(dir.as_deref(), *view) {
                    return self.back();
                }
            }
        }
        self.nav_cur = Some(self.nav_now());
    }

    fn ui(&mut self, ctx: &egui::Context) {
        let mut actions = Vec::new();
        theme::motion(ctx, self.s.smooth);

        let (new_note, new_folder, save) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
            )
        });
        let (quick, read, focus, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::P) | i.consume_key(egui::Modifiers::COMMAND, egui::Key::K),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::E),
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let back = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::OpenBracket) || i.pointer.button_pressed(egui::PointerButton::Extra1)
        });
        if back {
            self.back();
        }
        // Picked notes: Esc lets go, Delete sends them to the trash.
        if self.s.mode == Mode::Notes && self.quick.is_none() && !self.pick.picked.is_empty() && ctx.memory(|m| m.focused().is_none()) {
            let (esc, del) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)));
            if esc {
                self.pick.clear();
            } else if del {
                let any = self.pick.picked.iter().next().cloned().unwrap();
                for p in self.drag_set(&any) {
                    actions.push(Action::Delete(p));
                }
                self.pick.clear();
            }
        }
        if quick {
            self.quick = if self.quick.is_some() { None } else { Some(palette::State::default()) };
        }
        if self.s.mode == Mode::Notes && self.open.is_some() {
            if read {
                self.reading = !self.reading;
            }
            if focus {
                self.focus_mode = !self.focus_mode;
            }
        }
        if esc && self.focus_mode && self.quick.is_none() {
            self.focus_mode = false;
        }
        let focus_view = self.focus_mode && self.s.mode == Mode::Notes && self.open.is_some();
        self.drop_images(ctx);
        let target_dir = self
            .open
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| self.s.root.clone());
        if self.s.mode == Mode::Investigations {
            if new_note {
                self.inv.new_entity(investigation::Kind::Person);
            }
        } else if new_folder {
            actions.push(Action::NewFolder(target_dir.clone()));
        } else if new_note {
            actions.push(Action::NewNote(target_dir));
        }
        if save {
            self.flush();
            self.inv.flush();
        }

        if !focus_view {
            self.sidebar(ctx, &mut actions);
        }

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::BG)).show(ctx, |ui| {
            if self.s.mode == Mode::Investigations {
                self.inv.central(ui, self.s.font_size, theme::family_for(self.s.text_style));
            } else {
                self.editor(ui);
            }
        });

        if self.show_settings {
            self.settings_window(ctx);
        }
        self.quick_window(ctx, &mut actions);
        self.history_window(ctx);

        for a in actions {
            self.apply(a);
        }
        self.inv.tick(ctx);
        if self.inv.back_clicked {
            self.inv.back_clicked = false;
            self.back();
        }
        self.track_nav();
        self.inv.can_back = !self.nav.is_empty();
        if self.s.own_cursor {
            draw_cursor(ctx);
        }

        if self.dirty {
            let since = self.last_edit.elapsed();
            if since >= AUTOSAVE {
                self.flush();
            } else {
                ctx.request_repaint_after(AUTOSAVE - since);
            }
        }
        // Pick up files changed outside the app now and then.
        ctx.request_repaint_after(Duration::from_secs(2));
    }
}

/// The arrow that returns to the previous place.
pub fn back_button(ui: &mut egui::Ui) -> bool {
    theme::icon_button(ui, ic::ARROW_LEFT, &theme::keys("Назад  ⌘[")).clicked()
}

/// The app's own pointer: a thin sight that tightens on press and
/// changes shape over text and buttons. Resize arrows stay system ones.
fn draw_cursor(ctx: &egui::Context) {
    let icon = ctx.output(|o| o.cursor_icon);
    let resize = !matches!(icon, egui::CursorIcon::Default | egui::CursorIcon::PointingHand | egui::CursorIcon::Text | egui::CursorIcon::Grab | egui::CursorIcon::Grabbing);
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()).filter(|_| ctx.input(|i| i.pointer.has_pointer())) else { return };
    if resize {
        return;
    }
    ctx.set_cursor_icon(egui::CursorIcon::None);
    let down = ctx.input(|i| i.pointer.any_down());
    let hand = matches!(icon, egui::CursorIcon::PointingHand | egui::CursorIcon::Grab | egui::CursorIcon::Grabbing);
    let t_hand = ctx.animate_bool_with_time(egui::Id::new("cursor_hand"), hand, 0.12);
    let t_down = ctx.animate_bool_with_time(egui::Id::new("cursor_down"), down, 0.08);
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Debug, egui::Id::new("cursor")));
    let ink = theme::lerp(theme::TEXT, theme::ACCENT, t_hand);
    let line = Stroke::new(1.2, ink);
    let shadow = Stroke::new(3.2, egui::Color32::from_black_alpha(140));

    if icon == egui::CursorIcon::Text {
        // A tall caret with serifs.
        for s in [shadow, Stroke::new(1.2, theme::TEXT)] {
            p.line_segment([pos - egui::vec2(0.0, 9.0), pos + egui::vec2(0.0, 9.0)], s);
            p.line_segment([pos + egui::vec2(-3.5, -9.0), pos + egui::vec2(3.5, -9.0)], s);
            p.line_segment([pos + egui::vec2(-3.5, 9.0), pos + egui::vec2(3.5, 9.0)], s);
        }
        return;
    }
    let r = 7.0 + 4.0 * t_hand - 3.0 * t_down;
    let gap = r + 3.0;
    let tick = 5.0 - 2.0 * t_down;
    for s in [shadow, line] {
        p.circle_stroke(pos, r, s);
        for d in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(0.0, -1.0)] {
            p.line_segment([pos + d * gap, pos + d * (gap + tick)], s);
        }
    }
    if t_hand > 0.0 {
        p.circle_filled(pos, r, theme::tint(theme::ACCENT, 0.12 * t_hand));
    }
    p.circle_filled(pos, 1.6, if down { theme::DANGER } else { ink });
}

pub fn error_bar(ui: &mut egui::Ui, e: &str, slot: &mut Option<String>) {
    egui::Frame::NONE
        .fill(theme::tint(theme::DANGER, 0.12))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .outer_margin(egui::Margin::symmetric(24, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ic::WARNING_CIRCLE).color(theme::DANGER));
                ui.label(RichText::new(e).color(theme::DANGER));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button(ui, ic::X, "Закрыть").clicked() {
                        *slot = None;
                    }
                });
            });
        });
}

pub fn empty_state(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str) {
    let rect = ui.available_rect_before_wrap();
    let c = rect.center() - egui::vec2(0.0, 30.0);
    let p = ui.painter();
    p.text(c - egui::vec2(0.0, 36.0), egui::Align2::CENTER_CENTER, icon, theme::font(36.0), theme::FAINT.gamma_multiply(0.6));
    p.text(c + egui::vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, title, theme::font(16.0), theme::MUTED);
    p.text(c + egui::vec2(0.0, 32.0), egui::Align2::CENTER_CENTER, hint, theme::font(12.5), theme::FAINT);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.flush();
        self.inv.flush();
        self.s.inv_root = self.inv.root.clone();
        self.s.last_case = self.inv.current_dir();
        eframe::set_value(storage, eframe::APP_KEY, &self.s);
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(5)
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Quiet Note")
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("Quiet Note", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[cfg(test)]
mod shots;
