//! «Расследования»: case list, overview, entity cards, link graph, timeline.

use crate::investigation::{self as inv, Case, Event, Kind, Link, Reliability, Source};
use crate::theme::{self, caption, font, icon_button};
use eframe::egui::{self, Align2, Color32, FontFamily, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use egui_phosphor::regular as ic;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const AUTOSAVE: Duration = Duration::from_millis(800);

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Overview,
    Graph,
    Timeline,
    Map,
    Entity(usize),
}

pub fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Person => ic::USER,
        Kind::Nick => ic::AT,
        Kind::Email => ic::ENVELOPE_SIMPLE,
        Kind::Phone => ic::PHONE,
        Kind::Domain => ic::GLOBE,
        Kind::Org => ic::BUILDINGS,
        Kind::Place => ic::MAP_PIN,
        Kind::Other => ic::CUBE,
    }
}

fn rel_style(r: Reliability) -> (&'static str, Color32) {
    match r {
        Reliability::Confirmed => (ic::SEAL_CHECK, theme::GOOD),
        Reliability::Likely => (ic::SEAL_QUESTION, theme::WARN),
        Reliability::Doubtful => (ic::SEAL_WARNING, theme::DANGER),
    }
}

pub struct Inv {
    pub root: PathBuf,
    pub case: Option<Case>,
    pub view: View,
    last_edit: Instant,
    case_rename: Option<(PathBuf, String, bool)>,
    name_buf: Option<(usize, String)>,
    filter: String,
    pan: Vec2,
    zoom: f32,
    link_drag: Option<usize>,
    fit_next: bool,
    pub error: Option<String>,
    pw: Option<PwPrompt>,
    search: String,
    extract: Option<ExtractBox>,
    map: crate::map::MapView,
    jobs_tx: std::sync::mpsc::Sender<Job>,
    jobs_rx: std::sync::mpsc::Receiver<Job>,
    busy: usize,
    /// Automatic checks per (case, entity): running flag and findings with "added" marks.
    auto: std::collections::HashMap<(PathBuf, String), (bool, Vec<(crate::autosearch::Finding, bool)>)>,
}

#[derive(PartialEq, Clone, Copy)]
enum PwMode {
    Unlock,
    Encrypt,
    Decrypt,
}

struct PwPrompt {
    dir: PathBuf,
    mode: PwMode,
    a: String,
    b: String,
    err: Option<String>,
    focus: bool,
}

struct ExtractBox {
    text: String,
    found: Vec<(bool, crate::extract::Found)>,
}

/// Results of background work (page copies, geocoding), applied on the UI thread.
enum Job {
    Archived { case: PathBuf, entity: String, url: String, result: Result<(String, Vec<u8>), String> },
    Geocoded { case: PathBuf, entity: String, result: Result<(f64, f64), String> },
    Auto { case: PathBuf, entity: String, finding: Option<crate::autosearch::Finding> },
}

/// A sidebar row: icon + text, hover/selected background.
fn row(ui: &mut egui::Ui, indent: f32, icon: &str, icon_col: Color32, text: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());
    let r = rect.shrink2(egui::vec2(8.0, 1.0));
    if selected {
        ui.painter().rect_filled(r, 6.0, theme::SELECTED);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 6.0, theme::HOVER);
    }
    let y = rect.center().y;
    let x = rect.left() + indent;
    ui.painter().text(Pos2::new(x, y), Align2::LEFT_CENTER, icon, font(15.0), icon_col);
    let col = if selected { theme::TEXT } else { theme::MUTED };
    ui.painter()
        .with_clip_rect(r)
        .text(Pos2::new(x + 22.0, y), Align2::LEFT_CENTER, text, font(13.5), col);
    resp
}

/// A rounded panel with a header (icon, title, count) and an optional add button.
fn card<R>(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    count: Option<usize>,
    add_tip: Option<&str>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let mut add = false;
    let r = egui::Frame::NONE
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(12)
        .inner_margin(egui::Margin { left: 18, right: 14, top: 12, bottom: 16 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(15.0).color(theme::FAINT));
                ui.label(RichText::new(title).size(13.5).color(theme::MUTED));
                if let Some(n) = count.filter(|n| *n > 0) {
                    ui.label(RichText::new(n.to_string()).size(12.0).color(theme::FAINT));
                }
                if let Some(tip) = add_tip {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        add = icon_button(ui, ic::PLUS, tip).clicked();
                    });
                }
            });
            ui.add_space(8.0);
            body(ui)
        })
        .inner;
    ui.add_space(14.0);
    (add, r)
}

fn x_button(ui: &mut egui::Ui) -> bool {
    icon_button(ui, ic::X, "Удалить").clicked()
}

fn input<'a>(text: &'a mut String, hint: &str, w: f32) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .hint_text(RichText::new(hint).color(theme::FAINT))
        .desired_width(w)
        .margin(egui::vec2(8.0, 5.0))
}

/// What to search for: a field value that looks like the entity's type, else the name.
/// For an email card the name usually is the address; an empty "Адрес" field must not win.
fn probe_value(e: &inv::Doc) -> String {
    let looks = |v: &str| match e.kind {
        Kind::Email => v.contains('@'),
        Kind::Phone => v.chars().filter(|c| c.is_ascii_digit()).count() >= 7,
        Kind::Domain => v.contains('.') && !v.contains(' '),
        Kind::Place => true,
        _ => false,
    };
    if looks(&e.name) && e.kind != Kind::Place {
        return e.name.clone();
    }
    e.fields.iter().map(|(_, v)| v.trim()).find(|v| !v.is_empty() && looks(v)).map(str::to_string).unwrap_or_else(|| e.name.clone())
}

fn open_target(case: &Case, url: &str) -> Result<(), String> {
    let target = if url.contains("://") || url.starts_with("mailto:") {
        url.to_string()
    } else if url.contains('.') && !url.contains(' ') && !url.starts_with("files/") {
        format!("https://{url}")
    } else {
        case.openable(url)?.display().to_string()
    };
    crate::fsops::open_external(&target, false);
    Ok(())
}

/// Colored rounded square with the kind icon.
fn kind_badge(ui: &mut egui::Ui, k: Kind, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    ui.painter().rect_filled(rect, size * 0.28, theme::tint(k.color(), 0.16));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, kind_icon(k), font(size * 0.5), k.color());
    resp
}

/// Clickable chip with kind icon and name.
fn entity_chip(ui: &mut egui::Ui, k: Kind, name: &str) -> egui::Response {
    let text = format!("{}  {}", kind_icon(k), name);
    let g = ui.painter().layout_no_wrap(text, font(12.5), theme::TEXT);
    let size = g.size() + egui::vec2(20.0, 10.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { theme::tint(k.color(), 0.22) } else { theme::tint(k.color(), 0.12) };
    ui.painter().rect_filled(rect, 8.0, fill);
    let g = ui.painter().layout_no_wrap(format!("{}  {}", kind_icon(k), name), font(12.5), theme::TEXT);
    ui.painter().galley(rect.min + egui::vec2(10.0, 5.0), g, theme::TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl Inv {
    pub fn new(root: PathBuf, last_case: Option<PathBuf>) -> Self {
        let _ = std::fs::create_dir_all(&root);
        let case = last_case.filter(|p| p.is_dir() && !crate::vault::is_locked(p)).map(|p| Case::load(&p));
        let (jobs_tx, jobs_rx) = std::sync::mpsc::channel();
        Self {
            root,
            case,
            view: View::Overview,
            last_edit: Instant::now(),
            case_rename: None,
            name_buf: None,
            filter: String::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            link_drag: None,
            fit_next: true,
            error: None,
            pw: None,
            search: String::new(),
            extract: None,
            map: Default::default(),
            jobs_tx,
            jobs_rx,
            busy: 0,
            auto: Default::default(),
        }
    }

    pub fn current_dir(&self) -> Option<PathBuf> {
        self.case.as_ref().map(|c| c.dir.clone())
    }

    pub fn flush(&mut self) {
        if let Some(c) = &mut self.case {
            if let Err(e) = c.save() {
                self.error = Some(e);
            }
        }
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.flush();
        self.root = root;
        self.case = None;
        self.view = View::Overview;
    }

    fn open_case(&mut self, dir: &Path) {
        self.flush();
        if crate::vault::is_locked(dir) {
            self.pw = Some(PwPrompt { dir: dir.to_path_buf(), mode: PwMode::Unlock, a: String::new(), b: String::new(), err: None, focus: false });
            return;
        }
        self.show_case(Case::load(dir));
    }

    fn show_case(&mut self, case: Case) {
        self.case = Some(case);
        self.map = Default::default();
        self.view = View::Overview;
        self.name_buf = None;
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        self.fit_next = true;
        self.filter.clear();
    }

    fn edited(&mut self) {
        self.last_edit = Instant::now();
    }

    fn go(&mut self, v: View) {
        self.view = v;
        self.name_buf = None;
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        while let Ok(job) = self.jobs_rx.try_recv() {
            if !matches!(job, Job::Auto { .. }) {
                self.busy = self.busy.saturating_sub(1);
            }
            self.apply_job(job);
        }
        if self.auto.values().any(|a| a.0) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.case.as_ref().is_some_and(|c| c.is_dirty()) {
            let since = self.last_edit.elapsed();
            if since >= AUTOSAVE {
                self.flush();
            } else {
                ctx.request_repaint_after(AUTOSAVE - since);
            }
        }
    }

    pub fn new_entity(&mut self, kind: Kind) {
        if let Some(c) = &mut self.case {
            let i = c.add_entity(kind);
            self.go(View::Entity(i));
            self.edited();
        }
    }

    fn new_case(&mut self) {
        match inv::create_case(&self.root) {
            Ok(dir) => {
                self.open_case(&dir);
                self.case_rename = Some((dir, String::new(), false));
            }
            Err(e) => self.error = Some(e),
        }
    }

    // ---------------------------------------------------------------- sidebar

    pub fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            caption(ui, "Расследования");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if icon_button(ui, ic::PLUS, "Новое расследование").clicked() {
                    self.new_case();
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(input(&mut self.search, &format!("{}  Поиск по всем делам", ic::MAGNIFYING_GLASS), ui.available_width() - 26.0));
        });
        ui.add_space(8.0);
        if !self.search.trim().is_empty() {
            self.search_results(ui);
            return;
        }

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let cases = inv::list_cases(&self.root);
            if cases.is_empty() {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    if theme::pill(ui, ic::PLUS, "Начать расследование").clicked() {
                        self.new_case();
                    }
                });
            }
            let open_dir = self.current_dir();
            for dir in cases {
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let is_open = open_dir.as_ref() == Some(&dir);

                let mut renaming_here = false;
                if let Some((rp, buf, focused)) = &mut self.case_rename {
                    if *rp == dir {
                        renaming_here = true;
                        let mut done = None;
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            let r = ui.add(input(buf, &name, ui.available_width() - 26.0));
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
                            Some(Some(n)) => self.rename_case(&dir, &n),
                            Some(None) => self.case_rename = None,
                            None => {}
                        }
                    }
                }
                if !renaming_here {
                    let locked = crate::vault::is_locked(&dir);
                    let unlocked = is_open && self.case.as_ref().is_some_and(|c| c.secret.is_some());
                    let (icon, col) = match (is_open, locked) {
                        (true, true) => (ic::LOCK_SIMPLE_OPEN, theme::ACCENT),
                        (true, false) => (ic::FOLDER_OPEN, theme::ACCENT),
                        (false, true) => (ic::LOCK_SIMPLE, theme::WARN),
                        (false, false) => (ic::FOLDER, theme::FAINT),
                    };
                    let r = row(ui, 14.0, icon, col, &name, is_open && self.view != View::Graph && false);
                    if r.clicked() {
                        if is_open {
                            self.go(View::Overview);
                        } else {
                            self.open_case(&dir);
                        }
                    }
                    if r.double_clicked() {
                        self.case_rename = Some((dir.clone(), name.clone(), false));
                    }
                    r.context_menu(|ui| {
                        ui.set_min_width(190.0);
                        if ui.button(format!("{}  Переименовать", ic::PENCIL_SIMPLE)).clicked() {
                            self.case_rename = Some((dir.clone(), name.clone(), false));
                            ui.close();
                        }
                        if ui.button(format!("{}  {}", ic::ARROW_SQUARE_OUT, crate::fsops::FILE_MANAGER)).clicked() {
                            crate::fsops::open_external(&dir.display().to_string(), false);
                            ui.close();
                        }
                        ui.separator();
                        if !locked {
                            if ui.button(format!("{}  Зашифровать паролем…", ic::LOCK_SIMPLE)).clicked() {
                                self.pw = Some(PwPrompt { dir: dir.clone(), mode: PwMode::Encrypt, a: String::new(), b: String::new(), err: None, focus: false });
                                ui.close();
                            }
                        } else if unlocked {
                            if ui.button(format!("{}  Заблокировать", ic::LOCK_SIMPLE)).clicked() {
                                self.flush();
                                self.case = None;
                                ui.close();
                            }
                            if ui.button(format!("{}  Снять шифрование…", ic::LOCK_SIMPLE_OPEN)).clicked() {
                                self.pw = Some(PwPrompt { dir: dir.clone(), mode: PwMode::Decrypt, a: String::new(), b: String::new(), err: None, focus: false });
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button(RichText::new(format!("{}  В корзину", ic::TRASH)).color(theme::DANGER)).clicked() {
                            if is_open {
                                self.flush();
                                self.case = None;
                            }
                            if let Err(e) = trash::delete(&dir) {
                                self.error = Some(format!("Не удалось удалить: {e}"));
                            }
                            ui.close();
                        }
                    });
                }
                if is_open {
                    self.case_children(ui);
                }
            }
        });
    }

    fn rename_case(&mut self, dir: &Path, name: &str) {
        self.case_rename = None;
        if name.trim().is_empty() {
            return;
        }
        let res = inv::valid_name(name).and_then(|n| {
            let target = self.root.join(&n);
            if target == dir {
                return Ok(target);
            }
            if target.exists() {
                return Err(format!("«{n}» уже существует"));
            }
            self.flush();
            std::fs::rename(dir, &target).map_err(|e| format!("Не удалось переименовать: {e}"))?;
            Ok(target)
        });
        match res {
            Ok(target) => {
                if let Some(c) = &mut self.case {
                    if c.dir == dir {
                        c.dir = target;
                    }
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn case_children(&mut self, ui: &mut egui::Ui) {
        let Some(c) = &self.case else { return };
        if c.entities.len() > 6 {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(34.0);
                ui.add(input(&mut self.filter, &format!("{}  Найти в деле", ic::MAGNIFYING_GLASS), ui.available_width() - 26.0));
            });
            ui.add_space(4.0);
        }
        let Some(c) = &self.case else { return };
        let f = self.filter.trim().to_lowercase();
        let mut order: Vec<usize> = (0..c.entities.len())
            .filter(|&i| f.is_empty() || c.entities[i].name.to_lowercase().contains(&f))
            .collect();
        order.sort_by_key(|&i| (Kind::ALL.iter().position(|k| *k == c.entities[i].kind), c.entities[i].name.to_lowercase()));
        let items: Vec<(usize, String, Kind)> =
            order.into_iter().map(|i| (i, c.entities[i].name.clone(), c.entities[i].kind)).collect();
        if items.is_empty() && f.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(36.0);
                ui.label(RichText::new("Сущностей пока нет").size(12.0).color(theme::FAINT));
            });
        }
        let mut delete = None;
        for (i, name, kind) in items {
            let r = row(ui, 34.0, kind_icon(kind), kind.color(), &name, self.view == View::Entity(i));
            if r.clicked() {
                self.go(View::Entity(i));
            }
            r.context_menu(|ui| {
                if ui.button(RichText::new(format!("{}  В корзину", ic::TRASH)).color(theme::DANGER)).clicked() {
                    delete = Some(i);
                    ui.close();
                }
            });
        }
        if let Some(i) = delete {
            self.delete_entity(i);
        }
        ui.add_space(10.0);
    }

    fn delete_entity(&mut self, i: usize) {
        let Some(c) = &mut self.case else { return };
        match c.delete_entity(i) {
            Ok(()) => {
                let v = match self.view {
                    View::Entity(j) if j == i => View::Overview,
                    View::Entity(j) if j > i => View::Entity(j - 1),
                    v => v,
                };
                self.go(v);
            }
            Err(e) => self.error = Some(e),
        }
    }

    // ---------------------------------------------------------------- central

    pub fn central(&mut self, ui: &mut egui::Ui, font_size: f32, family: FontFamily) {
        self.pw_window(ui.ctx());
        self.extract_window(ui.ctx());
        if let Some(e) = self.error.clone() {
            crate::error_bar(ui, &e, &mut self.error);
        }
        if self.case.is_none() {
            let n = inv::list_cases(&self.root).len();
            let hint = if n == 0 { "Нажмите ＋ слева, чтобы начать" } else { "Выберите расследование слева" };
            crate::empty_state(ui, ic::DETECTIVE, "Расследования", hint);
            return;
        }
        self.header(ui);
        match self.view {
            View::Graph => self.graph(ui),
            View::Overview => self.padded(ui, |s, ui| s.overview(ui, FontId::new(font_size, family))),
            View::Timeline => self.padded(ui, |s, ui| s.timeline(ui)),
            View::Map => self.map_view(ui),
            View::Entity(i) => self.padded(ui, |s, ui| s.entity(ui, i, FontId::new(font_size, family))),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let name = self.case.as_ref().unwrap().name();
        egui::Frame::NONE.inner_margin(egui::Margin { left: 28, right: 20, top: 14, bottom: 10 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ic::DETECTIVE).size(17.0).color(theme::ACCENT));
                ui.label(RichText::new(&name).size(15.5).color(theme::TEXT));
                ui.add_space(18.0);

                // Segmented tabs
                let tabs = [(View::Overview, ic::SQUARES_FOUR, "Обзор"), (View::Graph, ic::GRAPH, "Граф"), (View::Timeline, ic::CLOCK_COUNTER_CLOCKWISE, "Хронология"), (View::Map, ic::MAP_TRIFOLD, "Карта")];
                egui::Frame::NONE.fill(theme::SURFACE).corner_radius(9).inner_margin(3).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (v, icon, label) in tabs {
                        let on = self.view == v;
                        let txt = RichText::new(format!("{icon}  {label}")).size(12.5).color(if on { theme::TEXT } else { theme::FAINT });
                        let b = egui::Button::new(txt)
                            .fill(if on { theme::SELECTED } else { Color32::TRANSPARENT })
                            .stroke(if on { Stroke::new(1.0, theme::BORDER) } else { Stroke::NONE })
                            .corner_radius(7);
                        if ui.add(b).clicked() {
                            self.go(v);
                        }
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = egui::Button::new(RichText::new(format!("{}  Сущность", ic::PLUS)).size(12.5).color(theme::TEXT))
                        .fill(theme::ACCENT_BG)
                        .stroke(Stroke::new(1.0, theme::tint(theme::ACCENT, 0.35)))
                        .corner_radius(8);
                    let r = ui.add(b);
                    let tools = theme::icon_button(ui, ic::DOTS_THREE, "Инструменты");
                    egui::Popup::menu(&tools).show(|ui| {
                        ui.set_min_width(220.0);
                        if ui.button(format!("{}  Найти сущности в тексте…", ic::MAGIC_WAND)).clicked() {
                            self.extract = Some(ExtractBox { text: String::new(), found: vec![] });
                            ui.close();
                        }
                        if ui.button(format!("{}  Проверить всё дело автоматически", ic::LIGHTNING)).on_hover_text("Никнеймы, почты, домены, телефоны и IP проверятся по открытым источникам").clicked() {
                            self.auto_case();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(format!("{}  Отчёт в PDF…", ic::FILE_PDF)).clicked() {
                            self.report(true);
                            ui.close();
                        }
                        if ui.button(format!("{}  Отчёт в Markdown…", ic::FILE_TEXT)).clicked() {
                            self.report(false);
                            ui.close();
                        }
                    });
                    egui::Popup::menu(&r).show(|ui| {
                        ui.set_min_width(180.0);
                        for k in Kind::ALL {
                            if ui.button(RichText::new(format!("{}  {}", kind_icon(k), k.label()))).clicked() {
                                self.new_entity(k);
                                ui.close();
                            }
                        }
                    });
                });
            });
        });
        let y = ui.cursor().min.y;
        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, theme::BORDER));
    }

    fn padded(&mut self, ui: &mut egui::Ui, f: impl FnOnce(&mut Self, &mut egui::Ui)) {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let avail = ui.available_width();
            let w = avail.min(880.0) - 56.0;
            let side = ((avail - w) / 2.0).max(28.0);
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    f(self, ui);
                    ui.add_space(40.0);
                });
            });
        });
    }

    fn overview(&mut self, ui: &mut egui::Ui, text_font: FontId) {
        let c = self.case.as_mut().unwrap();
        let links: usize = c.entities.iter().map(|e| e.links.len()).sum();
        let sources: usize = c.entities.iter().map(|e| e.sources.len()).sum();
        let events = c.timeline().len();
        let stats = [
            (ic::CUBE, c.entities.len(), "сущность", "сущности", "сущностей"),
            (ic::LINK, links, "связь", "связи", "связей"),
            (ic::PAPERCLIP, sources, "источник", "источника", "источников"),
            (ic::CALENDAR_BLANK, events, "событие", "события", "событий"),
        ];
        let gap = 12.0;
        let tile_w = (ui.available_width() - gap * 3.0) / 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (icon, n, one, few, many) in stats {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(tile_w, 76.0), Sense::hover());
                let p = ui.painter();
                p.rect(rect, 12.0, theme::SURFACE, Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);
                p.text(rect.left_top() + egui::vec2(16.0, 14.0), Align2::LEFT_TOP, icon, font(15.0), theme::FAINT);
                p.text(rect.left_bottom() + egui::vec2(16.0, -14.0), Align2::LEFT_BOTTOM, n.to_string(), font(24.0), theme::TEXT);
                p.text(rect.right_bottom() + egui::vec2(-14.0, -17.0), Align2::RIGHT_BOTTOM, crate::plural(n, one, few, many), font(12.0), theme::FAINT);
            }
        });
        ui.add_space(16.0);

        // Entities at a glance
        let mut goto = None;
        let mut add_kind = None;
        let (_, _) = card(ui, ic::CUBE, "Сущности", Some(c.entities.len()), None, |ui| {
            if c.entities.is_empty() {
                ui.label(RichText::new("Начните с того, что уже известно: человек, никнейм, почта, домен…").color(theme::FAINT));
                ui.add_space(6.0);
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    let mut order: Vec<usize> = (0..c.entities.len()).collect();
                    order.sort_by_key(|&i| (Kind::ALL.iter().position(|k| *k == c.entities[i].kind), c.entities[i].name.to_lowercase()));
                    for i in order {
                        let e = &c.entities[i];
                        if entity_chip(ui, e.kind, &e.name).clicked() {
                            goto = Some(i);
                        }
                    }
                });
                ui.add_space(10.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                for k in Kind::ALL {
                    let b = egui::Button::new(RichText::new(format!("{}  {}", ic::PLUS, k.label())).size(12.0).color(theme::FAINT))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, theme::BORDER))
                        .corner_radius(8);
                    if ui.add(b).clicked() {
                        add_kind = Some(k);
                    }
                }
            });
        });

        let mut changed = false;
        card(ui, ic::TEXT_ALIGN_LEFT, "Заметки по делу", None, None, |ui| {
            changed = ui
                .add(
                    egui::TextEdit::multiline(&mut c.overview.notes)
                        .hint_text(RichText::new("Цель, гипотезы, что уже известно, следующие шаги…").color(theme::FAINT))
                        .font(text_font)
                        .desired_rows(12)
                        .desired_width(f32::INFINITY)
                        .frame(false),
                )
                .changed();
        });
        if changed {
            c.touch(None);
            self.edited();
        }
        if let Some(i) = goto {
            self.go(View::Entity(i));
        }
        if let Some(k) = add_kind {
            self.new_entity(k);
        }
    }

    fn timeline(&mut self, ui: &mut egui::Ui) {
        let c = self.case.as_mut().unwrap();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Хронология").size(22.0).color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::pill(ui, ic::PLUS, "Событие дела").clicked() {
                    c.overview.events.push(Event { date: inv::today(), text: String::new() });
                    c.touch(None);
                }
            });
        });
        ui.label(RichText::new("Все события из карточек и общие события дела, по дате").size(12.5).color(theme::FAINT));
        ui.add_space(22.0);

        let items = c.timeline();
        if items.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(ic::CLOCK_COUNTER_CLOCKWISE).size(30.0).color(theme::FAINT));
                ui.add_space(6.0);
                ui.label(RichText::new("Событий пока нет").color(theme::MUTED));
                ui.label(RichText::new("Добавляйте их в карточках сущностей или кнопкой выше").size(12.5).color(theme::FAINT));
            });
            return;
        }

        let mut changed = false;
        let mut remove = None;
        let mut goto = None;
        let mut used = vec![false; c.overview.events.len()];
        let line_x = ui.cursor().min.x + 104.0;
        let top = ui.cursor().min.y;
        let line_slot = ui.painter().add(egui::Shape::Noop);
        let mut last_year = String::new();
        for it in &items {
            let year = it.date.get(..4).unwrap_or("").to_string();
            if year != last_year && !year.is_empty() {
                ui.add_space(if last_year.is_empty() { 0.0 } else { 10.0 });
                ui.horizontal(|ui| {
                    ui.add_space(118.0);
                    ui.label(RichText::new(&year).size(12.0).color(theme::FAINT));
                });
                last_year = year;
            }
            let row_top = ui.cursor().min.y;
            let (dot_col, mut row_h) = (match it.owner {
                None => theme::ACCENT,
                Some(i) => c.entities[i].kind.color(),
            }, 0.0);
            ui.horizontal(|ui| {
                let date = if it.date.is_empty() { "без даты".to_string() } else { it.date.clone() };
                let (r, _) = ui.allocate_exact_size(egui::vec2(92.0, 26.0), Sense::hover());
                if it.owner.is_some() {
                    ui.painter().text(r.right_center(), Align2::RIGHT_CENTER, &date, FontId::monospace(12.0), theme::MUTED);
                }
                ui.add_space(26.0);
                match it.owner {
                    None => {
                        let Some(j) = (0..c.overview.events.len())
                            .find(|&j| !used[j] && c.overview.events[j].date == it.date && c.overview.events[j].text == it.text)
                        else {
                            return;
                        };
                        used[j] = true;
                        let e = &mut c.overview.events[j];
                        // The date input sits over the date column.
                        let date_rect = Rect::from_min_size(r.min - egui::vec2(8.0, 0.0), egui::vec2(100.0, 26.0));
                        changed |= ui.put(date_rect, input(&mut e.date, "ГГГГ-ММ-ДД", 100.0).font(FontId::monospace(12.0))).changed();
                        ui.label(RichText::new("дело").size(12.0).color(theme::ACCENT));
                        changed |= ui.add(input(&mut e.text, "Что произошло", ui.available_width() - 40.0)).changed();
                        if x_button(ui) {
                            remove = Some(j);
                        }
                    }
                    Some(i) => {
                        let d = &c.entities[i];
                        if entity_chip(ui, d.kind, &d.name).clicked() {
                            goto = Some(i);
                        }
                        ui.add(egui::Label::new(RichText::new(&it.text).color(theme::TEXT)).wrap());
                    }
                }
            });
            row_h = (ui.cursor().min.y - row_top).max(row_h);
            ui.painter().circle(Pos2::new(line_x, row_top + 13.0), 4.5, theme::BG, Stroke::new(2.0, dot_col));
            let _ = row_h;
            ui.add_space(6.0);
        }
        let bottom = ui.cursor().min.y;
        ui.painter().set(line_slot, egui::Shape::line_segment([Pos2::new(line_x, top + 13.0), Pos2::new(line_x, bottom - 19.0)], Stroke::new(1.0, theme::BORDER)));
        if let Some(j) = remove {
            c.overview.events.remove(j);
            changed = true;
        }
        if changed {
            c.touch(None);
            self.last_edit = Instant::now();
        }
        if let Some(i) = goto {
            self.go(View::Entity(i));
        }
    }

    fn entity(&mut self, ui: &mut egui::Ui, i: usize, text_font: FontId) {
        let c = self.case.as_mut().unwrap();
        if i >= c.entities.len() {
            self.view = View::Overview;
            return;
        }
        let names: Vec<(String, Kind)> = c.entities.iter().map(|e| (e.name.clone(), e.kind)).collect();
        let incoming: Vec<(usize, String)> = c
            .entities
            .iter()
            .enumerate()
            .flat_map(|(j, e)| e.links.iter().filter(|l| l.target == names[i].0).map(move |l| (j, l.relation.clone())))
            .collect();
        let mut changed = false;
        let mut goto = None;
        let mut rename = None;
        let mut attach = false;
        let mut open_req: Option<String> = None;
        let mut archive_req: Option<String> = None;
        let mut geocode_req: Option<String> = None;

        // ---- Header: badge, name, type
        let buf = match &mut self.name_buf {
            Some((j, b)) if *j == i => b,
            _ => {
                self.name_buf = Some((i, names[i].0.clone()));
                &mut self.name_buf.as_mut().unwrap().1
            }
        };
        let e = &mut c.entities[i];
        ui.horizontal(|ui| {
            kind_badge(ui, e.kind, 52.0);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let r = ui.add(
                    egui::TextEdit::singleline(buf)
                        .font(font(22.0))
                        .frame(false)
                        .margin(egui::vec2(0.0, 2.0))
                        .desired_width(ui.available_width()),
                );
                if r.lost_focus() && *buf != names[i].0 {
                    rename = Some(buf.clone());
                }
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("kind")
                        .width(150.0)
                        .selected_text(RichText::new(format!("{}  {}", kind_icon(e.kind), e.kind.label())).size(12.5).color(e.kind.color()))
                        .show_ui(ui, |ui| {
                            for k in Kind::ALL {
                                changed |= ui.selectable_value(&mut e.kind, k, format!("{}  {}", kind_icon(k), k.label())).changed();
                            }
                        });
                    let summary = format!(
                        "{} {} · {} {}",
                        e.links.len() + incoming.len(),
                        crate::plural(e.links.len() + incoming.len(), "связь", "связи", "связей"),
                        e.sources.len(),
                        crate::plural(e.sources.len(), "источник", "источника", "источников")
                    );
                    ui.label(RichText::new(summary).size(12.0).color(theme::FAINT));
                });
            });
        });
        ui.add_space(12.0);

        // ---- Search links: open in the browser, nothing is sent by the app itself.
        let probe = probe_value(e);
        let links = crate::extract::search_links(e.kind, &probe);
        if !links.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                ui.label(RichText::new(format!("{}  Искать", ic::GLOBE)).size(12.0).color(theme::FAINT));
                for (label, url) in links {
                    let b = egui::Button::new(RichText::new(label).size(12.0).color(theme::MUTED)).fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(10);
                    if ui.add(b).on_hover_text(&url).clicked() {
                        crate::fsops::open_external(&url, false);
                    }
                }
            });
        }
        let mut auto_start = false;
        let mut auto_add: Vec<usize> = vec![];
        let auto_key = (c.dir.clone(), e.name.clone());
        if crate::autosearch::supported(e.kind, &probe) {
            ui.add_space(10.0);
            let state = self.auto.get(&auto_key);
            let running = state.is_some_and(|s| s.0);
            ui.horizontal(|ui| {
                if running {
                    ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
                    ui.label(RichText::new("Идёт автопроверка…").size(12.5).color(theme::MUTED));
                } else {
                    let label = if state.is_some() { "Проверить ещё раз" } else { "Проверить автоматически" };
                    auto_start = theme::pill(ui, ic::LIGHTNING, label)
                        .on_hover_text(format!("{} будет проверен по открытым источникам. Запросы уходят напрямую с вашего компьютера.", probe))
                        .clicked();
                }
                if let Some((_, f)) = state {
                    let open = f.iter().filter(|(x, added)| x.add.is_some() && !added).count();
                    ui.label(RichText::new(format!("находок: {}", f.len())).size(12.0).color(theme::FAINT));
                    if open > 1 && ui.add(egui::Button::new(RichText::new(format!("Добавить всё ({open})")).size(12.0)).corner_radius(8)).clicked() {
                        auto_add = f.iter().enumerate().filter(|(_, (x, a))| x.add.is_some() && !a).map(|(k, _)| k).collect();
                    }
                }
            });
            if let Some((run, f)) = state {
                if f.is_empty() && !run {
                    ui.label(RichText::new("Ничего не найдено").size(12.5).color(theme::FAINT));
                }
                ui.add_space(4.0);
                for (k, (x, added)) in f.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized([110.0, 22.0], egui::Label::new(RichText::new(x.check).size(11.5).color(theme::FAINT)).truncate());
                        ui.label(RichText::new(&x.text).size(12.5).color(if *added { theme::FAINT } else { theme::TEXT }));
                        if *added {
                            ui.label(RichText::new(ic::CHECK).color(theme::GOOD));
                        } else if x.add.is_some() && icon_button(ui, ic::PLUS, "Добавить в карточку").clicked() {
                            auto_add.push(k);
                        }
                    });
                }
            }
        }
        if e.kind == Kind::Place {
            let addr = e.fields.iter().find(|(k, v)| k.to_lowercase().contains("адрес") && !v.trim().is_empty()).map(|(_, v)| v.clone());
            let has = e.fields.iter().any(|(_, v)| crate::extract::coords(v).is_some());
            if let (Some(a), false) = (addr, has) {
                ui.add_space(6.0);
                if theme::pill(ui, ic::MAP_PIN, "Найти координаты по адресу").on_hover_text("Адрес отправится в OpenStreetMap Nominatim").clicked() {
                    geocode_req = Some(a);
                }
            }
        }
        ui.add_space(16.0);

        // ---- Data fields
        let (add, _) = card(ui, ic::LIST_BULLETS, "Данные", None, Some("Добавить поле"), |ui| {
            if e.fields.is_empty() {
                ui.label(RichText::new("Нет полей").size(12.5).color(theme::FAINT));
            }
            let mut rm = None;
            let w = ui.available_width();
            for (j, (k, v)) in e.fields.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.add_sized([160.0, 28.0], input(k, "поле", 160.0).text_color(theme::MUTED)).changed();
                    changed |= ui.add_sized([w - 160.0 - 50.0, 28.0], input(v, "значение", 0.0)).changed();
                    if x_button(ui) {
                        rm = Some(j);
                    }
                });
            }
            if let Some(j) = rm {
                e.fields.remove(j);
                changed = true;
            }
        });
        if add {
            e.fields.push(Default::default());
            changed = true;
        }

        // ---- Links
        let can_link = names.len() > 1;
        let (add, _) = card(
            ui,
            ic::LINK,
            "Связи",
            Some(e.links.len() + incoming.len()),
            can_link.then_some("Добавить связь"),
            |ui| {
                if !can_link {
                    ui.label(RichText::new("Добавьте ещё сущности, чтобы связать их").size(12.5).color(theme::FAINT));
                } else if e.links.is_empty() && incoming.is_empty() {
                    ui.label(RichText::new("Связей пока нет. На графе можно связать узлы: Shift + перетаскивание").size(12.5).color(theme::FAINT));
                }
                let mut rm = None;
                let w = ui.available_width();
                for (j, l) in e.links.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= ui.add_sized([170.0, 28.0], input(&mut l.relation, "как связан", 0.0)).changed();
                        ui.label(RichText::new(ic::ARROW_RIGHT).color(theme::FAINT));
                        let shown = names.iter().find(|(n, _)| *n == l.target);
                        let sel = match shown {
                            Some((n, k)) => RichText::new(format!("{}  {}", kind_icon(*k), n)).color(theme::TEXT),
                            None if l.target.is_empty() => RichText::new("выберите…").color(theme::FAINT),
                            None => RichText::new(format!("{}  (нет)", l.target)).color(theme::DANGER),
                        };
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt(("target", i, j)).width((w - 170.0 - 110.0).max(140.0)).selected_text(sel).show_ui(ui, |ui| {
                                for (n_i, (n, k)) in names.iter().enumerate() {
                                    if n_i != i {
                                        changed |= ui.selectable_value(&mut l.target, n.clone(), format!("{}  {}", kind_icon(*k), n)).changed();
                                    }
                                }
                            });
                            if let Some(t) = names.iter().position(|(n, _)| *n == l.target) {
                                if icon_button(ui, ic::ARROW_SQUARE_OUT, "Открыть карточку").clicked() {
                                    goto = Some(t);
                                }
                            }
                        });
                        if x_button(ui) {
                            rm = Some(j);
                        }
                    });
                }
                if let Some(j) = rm {
                    e.links.remove(j);
                    changed = true;
                }
                if !incoming.is_empty() {
                    if !e.links.is_empty() {
                        ui.add_space(10.0);
                    }
                    ui.label(RichText::new("Ссылаются на эту карточку").size(11.5).color(theme::FAINT));
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for (j, rel) in &incoming {
                            let (n, k) = &names[*j];
                            let label = if rel.is_empty() { n.clone() } else { format!("{n} — {rel}") };
                            if entity_chip(ui, *k, &label).clicked() {
                                goto = Some(*j);
                            }
                        }
                    });
                }
            },
        );
        if add {
            e.links.push(Link::default());
            changed = true;
        }

        // ---- Sources
        let (add, _) = card(ui, ic::PAPERCLIP, "Источники и доказательства", Some(e.sources.len()), Some("Добавить ссылку"), |ui| {
            let mut rm = None;
            let w = ui.available_width();
            for (j, s) in e.sources.iter_mut().enumerate() {
                if j > 0 {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    let (icon, col) = rel_style(s.reliability);
                    let b = egui::Button::new(RichText::new(format!("{icon}  {}", s.reliability.label())).size(12.0).color(col))
                        .fill(theme::tint(col, 0.12))
                        .stroke(Stroke::NONE)
                        .corner_radius(7)
                        .min_size(egui::vec2(128.0, 26.0));
                    if ui.add(b).on_hover_text("Нажмите, чтобы изменить надёжность").clicked() {
                        let k = Reliability::ALL.iter().position(|r| *r == s.reliability).unwrap_or(0);
                        s.reliability = Reliability::ALL[(k + 1) % Reliability::ALL.len()];
                        changed = true;
                    }
                    let is_file = s.url.starts_with("files/");
                    let mut te = input(&mut s.url, "ссылка или путь к файлу", 0.0);
                    if is_file {
                        te = te.text_color(theme::ACCENT);
                    }
                    changed |= ui.add_sized([w - 136.0 - 116.0, 28.0], te).changed();
                    if !s.url.is_empty() && icon_button(ui, ic::ARROW_SQUARE_OUT, "Открыть").clicked() {
                        open_req = Some(s.url.clone());
                    }
                    if s.url.starts_with("http") {
                        if icon_button(ui, ic::ARCHIVE, "Сохранить копию страницы в дело").clicked() {
                            archive_req = Some(s.url.clone());
                        }
                    } else {
                        ui.add_space(ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
                    }
                    if x_button(ui) {
                        rm = Some(j);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(136.0);
                    ui.label(RichText::new(ic::CALENDAR_BLANK).color(theme::FAINT));
                    changed |= ui.add_sized([104.0, 28.0], input(&mut s.date, "дата", 0.0).font(FontId::monospace(12.0))).changed();
                    changed |= ui.add_sized([w - 136.0 - 24.0 - 104.0 - 50.0, 28.0], input(&mut s.note, "что подтверждает", 0.0)).changed();
                });
            }
            if let Some(j) = rm {
                e.sources.remove(j);
                changed = true;
            }
            if !e.sources.is_empty() {
                ui.add_space(10.0);
            }
            attach = theme::pill(ui, ic::PAPERCLIP, "Приложить файл…").on_hover_text("Копия файла сохранится в папке расследования").clicked();
        });
        if add {
            e.sources.push(Source { date: inv::today(), ..Default::default() });
            changed = true;
        }

        // ---- Events
        let (add, _) = card(ui, ic::CALENDAR_BLANK, "События", Some(e.events.len()), Some("Добавить событие"), |ui| {
            if e.events.is_empty() {
                ui.label(RichText::new("События появятся в общей хронологии дела").size(12.5).color(theme::FAINT));
            }
            let mut rm = None;
            let w = ui.available_width();
            for (j, ev) in e.events.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.add_sized([116.0, 28.0], input(&mut ev.date, "ГГГГ-ММ-ДД", 0.0).font(FontId::monospace(12.0))).changed();
                    changed |= ui.add_sized([w - 116.0 - 50.0, 28.0], input(&mut ev.text, "что произошло", 0.0)).changed();
                    if x_button(ui) {
                        rm = Some(j);
                    }
                });
            }
            if let Some(j) = rm {
                e.events.remove(j);
                changed = true;
            }
        });
        if add {
            e.events.push(Event::default());
            changed = true;
        }

        // ---- Notes
        card(ui, ic::TEXT_ALIGN_LEFT, "Заметки", None, None, |ui| {
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut e.notes)
                        .hint_text(RichText::new("Наблюдения, версии, что проверить…").color(theme::FAINT))
                        .font(text_font)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .frame(false),
                )
                .changed();
        });

        if let Some(u) = open_req {
            if let Err(err) = open_target(c, &u) {
                self.error = Some(err);
            }
        }
        let (case_dir, ent_name) = (c.dir.clone(), c.entities[i].name.clone());
        if let Some(u) = archive_req {
            self.archive(case_dir.clone(), ent_name.clone(), u);
        }
        if let Some(a) = geocode_req {
            let (tx, case, entity) = (self.jobs_tx.clone(), case_dir.clone(), ent_name.clone());
            self.busy += 1;
            std::thread::spawn(move || {
                let _ = tx.send(Job::Geocoded { case, entity, result: crate::map::geocode(&a) });
            });
        }
        if auto_start {
            let kind = self.case.as_ref().unwrap().entities[i].kind;
            self.start_auto(case_dir.clone(), ent_name.clone(), kind, probe.clone());
        }
        if !auto_add.is_empty() {
            self.apply_auto(i, &auto_key, &auto_add);
        }
        let c = self.case.as_mut().unwrap();
        if attach {
            if let Some(file) = rfd::FileDialog::new().pick_file() {
                match c.attach(&file) {
                    Ok(rel) => {
                        c.entities[i].sources.push(Source {
                            url: rel,
                            date: inv::today(),
                            note: file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                            ..Default::default()
                        });
                        changed = true;
                    }
                    Err(err) => self.error = Some(err),
                }
            }
        }
        if changed {
            c.touch(Some(i));
            self.last_edit = Instant::now();
        }
        if let Some(n) = rename {
            if let Err(err) = c.rename_entity(i, &n) {
                self.error = Some(err);
                self.name_buf = None;
            }
        }
        if let Some(t) = goto {
            self.go(View::Entity(t));
        }
    }

    // ---------------------------------------------------------------- graph

    fn graph(&mut self, ui: &mut egui::Ui) {
        let (rect, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if bg.dragged() {
            self.pan += bg.drag_delta();
        }
        if ui.rect_contains_pointer(rect) {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
            if scroll != 0.0 {
                let old = self.zoom;
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.35, 2.5);
                // zoom around pointer
                if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                    let center = rect.center() + self.pan;
                    let rel = ptr - center;
                    self.pan += rel - rel * (self.zoom / old);
                }
            }
        }
        let zoom = self.zoom;
        let selected = match self.view {
            View::Entity(i) => Some(i),
            _ => None,
        };
        let c = self.case.as_mut().unwrap();
        let painter = ui.painter_at(rect);

        // Dot grid
        let step = 26.0 * zoom;
        let off = Vec2::new(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
        let mut y = rect.top() + off.y;
        while y < rect.bottom() {
            let mut x = rect.left() + off.x;
            while x < rect.right() {
                painter.circle_filled(Pos2::new(x, y), 0.9, Color32::from_rgb(0x21, 0x23, 0x28));
                x += step;
            }
            y += step;
        }

        let n = c.entities.len();
        if n > 1 && c.pos.is_empty() {
            force_layout(c);
            c.touch_pos();
            self.fit_next = true;
        }
        let mut new_pos = false;
        for (k, e) in c.entities.iter().enumerate() {
            if !c.pos.contains_key(&e.name) {
                let a = k as f32 / n.max(1) as f32 * std::f32::consts::TAU;
                let r = 120.0 + 18.0 * n as f32;
                c.pos.insert(e.name.clone(), [r * a.cos(), r * a.sin()]);
                new_pos = true;
            }
        }
        if new_pos {
            c.touch_pos();
        }
        if std::mem::take(&mut self.fit_next) && n > 0 {
            let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            for e in &c.entities {
                let v = c.pos[&e.name];
                lo = lo.min(Vec2::new(v[0] - 110.0, v[1] - 24.0));
                hi = hi.max(Vec2::new(v[0] + 110.0, v[1] + 24.0));
            }
            let size = hi - lo;
            self.zoom = (rect.width() * 0.88 / size.x).min((rect.height() - 120.0) * 0.88 / size.y).clamp(0.35, 1.2);
            self.pan = -((lo + hi) * 0.5) * self.zoom;
        }
        let zoom = self.zoom;
        let center = rect.center() + self.pan;
        let to_screen = |v: &[f32; 2]| center + Vec2::new(v[0], v[1]) * zoom;

        // Node rects first, for hit testing and edge clipping.
        let node_font = font((13.0 * zoom.sqrt()).clamp(10.0, 16.0));
        let nodes: Vec<(Rect, std::sync::Arc<egui::Galley>)> = c
            .entities
            .iter()
            .map(|e| {
                let p = to_screen(&c.pos[&e.name]);
                let g = painter.layout_no_wrap(e.name.clone(), node_font.clone(), theme::TEXT);
                let size = g.size() + egui::vec2(42.0, 16.0);
                (Rect::from_center_size(p, size), g)
            })
            .collect();
        let hovered = ui.input(|i| i.pointer.hover_pos()).and_then(|p| nodes.iter().position(|(r, _)| r.contains(p)));
        let focus = hovered.or(selected);
        let neighbours = |a: usize, b: usize| a == b || c.entities[a].links.iter().any(|l| c.entities.get(b).is_some_and(|e| e.name == l.target)) || c.entities[b].links.iter().any(|l| l.target == c.entities[a].name);

        // Edges
        for (a, e) in c.entities.iter().enumerate() {
            for l in &e.links {
                let Some(b) = c.find(&l.target) else { continue };
                if a == b {
                    continue;
                }
                let (ra, rb) = (nodes[a].0, nodes[b].0);
                let lit = focus.is_none_or(|f| f == a || f == b);
                let col = if lit && focus.is_some() { theme::tint(theme::ACCENT, 0.8) } else if lit { Color32::from_rgb(0x4b, 0x4f, 0x5a) } else { Color32::from_rgb(0x2a, 0x2c, 0x32) };
                let from = edge_point(ra, rb.center());
                let to = edge_point(rb, ra.center());
                painter.line_segment([from, to], Stroke::new(1.3, col));
                // arrow head
                let dir = (to - from).normalized();
                let perp = Vec2::new(-dir.y, dir.x);
                let tip = to;
                painter.add(egui::Shape::convex_polygon(vec![tip, tip - dir * 8.0 + perp * 4.0, tip - dir * 8.0 - perp * 4.0], col, Stroke::NONE));
                if !l.relation.is_empty() && lit {
                    let mid = from + (to - from) * 0.5;
                    let g = painter.layout_no_wrap(l.relation.clone(), font(11.0), theme::MUTED);
                    let r = Rect::from_center_size(mid, g.size() + egui::vec2(12.0, 6.0));
                    painter.rect(r, 6.0, theme::BG, Stroke::new(1.0, theme::BORDER), StrokeKind::Inside);
                    painter.galley(r.min + egui::vec2(6.0, 3.0), g, theme::MUTED);
                }
            }
        }

        // Nodes
        let shift = ui.input(|i| i.modifiers.shift);
        let mut open = None;
        let mut moved = false;
        let mut link_to = None;
        for (k, e) in c.entities.iter().enumerate() {
            let (r, g) = &nodes[k];
            let resp = ui.interact(*r, ui.id().with(("node", &e.name)), Sense::click_and_drag());
            if resp.drag_started() && shift {
                self.link_drag = Some(k);
            }
            if resp.dragged() && self.link_drag.is_none() {
                let d = resp.drag_delta() / zoom;
                if let Some(pp) = c.pos.get_mut(&e.name) {
                    pp[0] += d.x;
                    pp[1] += d.y;
                }
                moved = true;
            }
            if resp.drag_stopped() && self.link_drag == Some(k) {
                if let Some(t) = ui.input(|i| i.pointer.interact_pos()).and_then(|p| nodes.iter().position(|(nr, _)| nr.contains(p))) {
                    if t != k {
                        link_to = Some((k, t));
                    }
                }
                self.link_drag = None;
            }
            if resp.clicked() {
                open = Some(k);
            }
            let col = e.kind.color();
            let dim = focus.is_some_and(|f| !neighbours(f, k));
            let is_sel = selected == Some(k);
            let fill = if resp.hovered() || is_sel { theme::SELECTED } else { theme::SURFACE };
            let stroke = if is_sel { Stroke::new(1.5, col) } else { Stroke::new(1.0, theme::tint(col, if dim { 0.2 } else { 0.55 })) };
            let alpha: f32 = if dim { 0.4 } else { 1.0 };
            painter.rect(r.expand(0.0), r.height() / 2.0, fill.gamma_multiply(alpha.max(0.8)), stroke, StrokeKind::Inside);
            let ic_c = Pos2::new(r.left() + 16.0, r.center().y);
            painter.circle_filled(ic_c, 10.0, theme::tint(col, 0.18 * alpha));
            painter.text(ic_c, Align2::CENTER_CENTER, kind_icon(e.kind), font(11.5), col.gamma_multiply(alpha));
            painter.galley(Pos2::new(r.left() + 32.0, r.center().y - g.size().y / 2.0), g.clone(), theme::TEXT.gamma_multiply(alpha));
            let _ = resp.on_hover_cursor(if shift { egui::CursorIcon::Crosshair } else { egui::CursorIcon::Grab });
        }
        if let Some(src) = self.link_drag {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let from = nodes[src].0.center();
                dashed(&painter, from, p, theme::ACCENT);
            }
        }
        if let Some((a, b)) = link_to {
            let target = c.entities[b].name.clone();
            if !c.entities[a].links.iter().any(|l| l.target == target) {
                c.entities[a].links.push(Link { target, relation: String::new() });
                c.touch(Some(a));
                self.last_edit = Instant::now();
            }
        }
        if moved {
            c.touch_pos();
            self.last_edit = Instant::now();
        }

        if n == 0 {
            painter.text(rect.center() - egui::vec2(0.0, 12.0), Align2::CENTER_CENTER, ic::GRAPH, font(32.0), theme::FAINT.gamma_multiply(0.6));
            painter.text(rect.center() + egui::vec2(0.0, 22.0), Align2::CENTER_CENTER, "Добавьте сущности — они появятся здесь", font(13.5), theme::FAINT);
        }

        // Legend (kinds present)
        let mut kinds: Vec<Kind> = Kind::ALL.into_iter().filter(|k| c.entities.iter().any(|e| e.kind == *k)).collect();
        kinds.dedup();
        let mut x = rect.left() + 18.0;
        let ly = rect.bottom() - 20.0;
        for k in kinds {
            painter.circle_filled(Pos2::new(x + 4.0, ly), 3.5, k.color());
            let g = painter.layout_no_wrap(k.label().to_string(), font(11.5), theme::FAINT);
            let w = g.size().x;
            painter.galley(Pos2::new(x + 13.0, ly - g.size().y / 2.0), g, theme::FAINT);
            x += w + 28.0;
        }
        painter.text(
            rect.right_bottom() + egui::vec2(-18.0, -20.0),
            Align2::RIGHT_CENTER,
            "Shift + перетаскивание — связать  ·  колесо — масштаб",
            font(11.5),
            theme::FAINT,
        );

        // Floating toolbar
        let bar = Rect::from_min_size(rect.right_top() + egui::vec2(-130.0, 14.0), egui::vec2(116.0, 34.0));
        let mut layout = false;
        let mut recenter = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(9).inner_margin(4).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    layout = icon_button(ui, ic::MAGIC_WAND, "Расставить автоматически").clicked();
                    recenter = icon_button(ui, ic::CROSSHAIR, "Вписать в окно").clicked();
                    ui.label(RichText::new(format!("{:.0}%", zoom * 100.0)).size(11.5).color(theme::FAINT));
                });
            });
        });
        if recenter {
            self.fit_next = true;
        }
        if layout {
            force_layout(c);
            c.touch_pos();
            self.fit_next = true;
            self.last_edit = Instant::now();
        }
        if let Some(k) = open {
            self.go(View::Entity(k));
        }
    }
}

/// Where the segment from the rect centre to `toward` leaves the rect (rounded-pill approximation).
fn edge_point(r: Rect, toward: Pos2) -> Pos2 {
    let c = r.center();
    let d = toward - c;
    if d.length() < 1.0 {
        return c;
    }
    let tx = if d.x.abs() > 0.0 { (r.width() / 2.0 + 3.0) / d.x.abs() } else { f32::INFINITY };
    let ty = if d.y.abs() > 0.0 { (r.height() / 2.0 + 3.0) / d.y.abs() } else { f32::INFINITY };
    c + d * tx.min(ty).min(1.0)
}

fn dashed(p: &egui::Painter, a: Pos2, b: Pos2, col: Color32) {
    let len = (b - a).length();
    let dir = (b - a) / len.max(1.0);
    let mut t = 0.0;
    while t < len {
        let e = (t + 6.0).min(len);
        p.line_segment([a + dir * t, a + dir * e], Stroke::new(1.5, col));
        t += 11.0;
    }
}

/// Plain force-directed layout: nodes repel, links pull.
fn force_layout(c: &mut Case) {
    let names: Vec<String> = c.entities.iter().map(|e| e.name.clone()).collect();
    let n = names.len();
    if n == 0 {
        return;
    }
    let mut p: Vec<Vec2> = names
        .iter()
        .enumerate()
        .map(|(i, nm)| {
            c.pos.get(nm).map(|v| Vec2::new(v[0], v[1])).unwrap_or_else(|| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                Vec2::new(a.cos(), a.sin()) * 200.0
            })
        })
        .collect();
    let edges: Vec<(usize, usize)> = c
        .entities
        .iter()
        .enumerate()
        .flat_map(|(i, e)| e.links.iter().filter_map(|l| names.iter().position(|x| *x == l.target)).map(move |j| (i, j)))
        .filter(|(i, j)| i != j)
        .collect();
    let k = 170.0f32;
    let mut linked = vec![false; n];
    for &(i, j) in &edges {
        linked[i] = true;
        linked[j] = true;
    }
    for step in 0..400 {
        let t = 30.0 * (1.0 - step as f32 / 400.0) + 0.5;
        let mut d = vec![Vec2::ZERO; n];
        for i in 0..n {
            for j in (i + 1)..n {
                if !linked[i] || !linked[j] {
                    continue;
                }
                let mut v = p[i] - p[j];
                if v.length() < 1.0 {
                    v = Vec2::new((i as f32).sin() + 0.5, (j as f32).cos() + 0.5);
                }
                let f = v.normalized() * (k * k / v.length().max(1.0));
                d[i] += f;
                d[j] -= f;
            }
        }
        for &(i, j) in &edges {
            let v = p[i] - p[j];
            let f = v.normalized() * (v.length() * v.length() / k);
            d[i] -= f;
            d[j] += f;
        }
        for i in 0..n {
            // Nodes without links are only pushed away by the others; pull them back harder
            // so they stay near the group instead of flying off-screen.
            if !linked[i] {
                continue; // placed in a row afterwards
            }
            d[i] -= p[i] * 0.02;
            let len = d[i].length();
            if len > 0.0 {
                p[i] += d[i] / len * len.min(t);
            }
        }
    }
    // Nodes without links: a tidy row under the group.
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for i in (0..n).filter(|&i| linked[i]) {
        lo = lo.min(p[i]);
        hi = hi.max(p[i]);
    }
    if lo.x > hi.x {
        (lo, hi) = (Vec2::ZERO, Vec2::ZERO);
    }
    let lonely: Vec<usize> = (0..n).filter(|&i| !linked[i]).collect();
    let per_row = 4usize;
    for (k2, &i) in lonely.iter().enumerate() {
        let (row, col) = (k2 / per_row, k2 % per_row);
        let in_row = (lonely.len() - row * per_row).min(per_row) as f32;
        let x = (lo.x + hi.x) / 2.0 + (col as f32 - (in_row - 1.0) / 2.0) * 190.0;
        p[i] = Vec2::new(x / 1.3, (hi.y + 160.0 + row as f32 * 90.0) / 0.85);
    }
    // Wider than tall, to suit the window.
    for (nm, v) in names.iter().zip(p) {
        c.pos.insert(nm.clone(), [v.x * 1.3, v.y * 0.85]);
    }
}

// -------------------------------------------------------------------- tools

impl Inv {
    #[cfg(test)]
    pub fn fake_auto(&mut self, entity: &str) {
        use crate::autosearch::{Add, Finding};
        let c = self.case.as_ref().unwrap();
        let i = c.find(entity).unwrap();
        let f = |check: &'static str, text: &str, url: &str| Finding { check, text: text.into(), add: Some(Add::Profile { site: check.into(), url: url.into() }) };
        let list = vec![
            (f("GitHub", "Найден профиль: https://github.com/dark_fox", "https://github.com/dark_fox"), false),
            (f("Telegram", "Найден профиль: https://t.me/dark_fox", "https://t.me/dark_fox"), false),
            (f("Reddit", "Найден профиль: https://www.reddit.com/user/dark_fox", "https://www.reddit.com/user/dark_fox"), false),
            (f("Хабр", "Найден профиль: https://habr.com/ru/users/dark_fox/", "https://habr.com/ru/users/dark_fox/"), false),
        ];
        self.auto.insert((c.dir.clone(), entity.into()), (true, list));
        self.view = View::Entity(i);
    }

    #[cfg(test)]
    pub fn fake_auto_added(&mut self, entity: &str) {
        self.fake_auto(entity);
        let c = self.case.as_ref().unwrap();
        let (i, key) = (c.find(entity).unwrap(), (c.dir.clone(), entity.to_string()));
        if c.pos.is_empty() {
            let c = self.case.as_mut().unwrap();
            force_layout(c);
        }
        self.apply_auto(i, &key, &[0, 1, 2, 3]);
        self.view = View::Graph;
    }

    #[cfg(test)]
    pub fn open_encrypt(&mut self) {
        let dir = self.current_dir().unwrap();
        self.pw = Some(PwPrompt { dir, mode: PwMode::Encrypt, a: "секрет".into(), b: String::new(), err: None, focus: true });
    }

    #[cfg(test)]
    pub fn open_extract(&mut self, text: &str) {
        let found = crate::extract::find(text).into_iter().map(|f| (true, f)).collect();
        self.extract = Some(ExtractBox { text: text.into(), found });
    }

    fn apply_job(&mut self, job: Job) {
        if let Job::Auto { case, entity, finding } = job {
            let e = self.auto.entry((case, entity)).or_default();
            match finding {
                Some(f) => e.1.push((f, false)),
                None => e.0 = false,
            }
            return;
        }
        let (case_dir, entity) = match &job {
            Job::Archived { case, entity, .. } | Job::Geocoded { case, entity, .. } => (case.clone(), entity.clone()),
            Job::Auto { .. } => unreachable!(),
        };
        let Some(c) = self.case.as_mut().filter(|c| c.dir == case_dir) else { return };
        let Some(i) = c.find(&entity) else { return };
        match job {
            Job::Archived { url, result, .. } => match result {
                Ok((name, data)) => {
                    let rel = format!("files/{name}");
                    match c.write_bytes(&rel, &data) {
                        Ok(()) => {
                            c.entities[i].sources.push(Source { url: rel, date: inv::today(), reliability: Reliability::Confirmed, note: format!("копия {url}") });
                            c.touch(Some(i));
                        }
                        Err(e) => self.error = Some(format!("Не удалось сохранить копию: {e}")),
                    }
                }
                Err(e) => self.error = Some(e),
            },
            Job::Geocoded { result, .. } => match result {
                Ok((lat, lon)) => {
                    let v = format!("{lat:.6}, {lon:.6}");
                    let e = &mut c.entities[i];
                    match e.fields.iter_mut().find(|(k, _)| k.to_lowercase().contains("координат")) {
                        Some(f) => f.1 = v,
                        None => e.fields.push(("Координаты".into(), v)),
                    }
                    c.touch(Some(i));
                    self.map.fitted = false;
                }
                Err(e) => self.error = Some(e),
            },
            Job::Auto { .. } => {}
        }
        self.edited();
    }

    /// Downloads a page in the background and stores it in the case as evidence.
    fn archive(&mut self, case: PathBuf, entity: String, url: String) {
        let tx = self.jobs_tx.clone();
        self.busy += 1;
        std::thread::spawn(move || {
            let result = (|| {
                let resp = ureq::get(&url)
                    .set("User-Agent", "Mozilla/5.0 (QuietNote page archive)")
                    .timeout(Duration::from_secs(25))
                    .call()
                    .map_err(|e| format!("Не удалось скачать страницу: {e}"))?;
                let html = resp.content_type().contains("html");
                let mut data = vec![];
                std::io::Read::read_to_end(&mut std::io::Read::take(resp.into_reader(), 30 << 20), &mut data).map_err(|e| e.to_string())?;
                let host: String = url.split("://").nth(1).unwrap_or(&url).split('/').next().unwrap_or("page").chars().filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-').collect();
                let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M");
                let ext = if html { "html" } else { url.rsplit('.').next().filter(|e| e.len() <= 4 && !e.contains('/')).unwrap_or("bin") };
                if html {
                    let note = format!("<!-- Копия {url}, сохранена Quiet Note {stamp} -->\n");
                    data.splice(0..0, note.into_bytes());
                }
                Ok((format!("{host} {stamp}.{ext}"), data))
            })();
            let _ = tx.send(Job::Archived { case, entity, url, result });
        });
    }

    fn search_results(&mut self, ui: &mut egui::Ui) {
        let q = self.search.trim().to_lowercase();
        let mut hits: Vec<(PathBuf, String, String, Kind)> = vec![]; // case, case name, entity, kind
        let mut locked = 0;
        for dir in inv::list_cases(&self.root) {
            let case = match &self.case {
                Some(c) if c.dir == dir => None,
                _ if crate::vault::is_locked(&dir) => {
                    locked += 1;
                    continue;
                }
                _ => Some(Case::load(&dir)),
            };
            let c = case.as_ref().or(self.case.as_ref()).unwrap();
            for e in &c.entities {
                let hay = format!("{} {} {}", e.name, e.fields.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(" "), e.notes).to_lowercase();
                if hay.contains(&q) {
                    hits.push((dir.clone(), c.name(), e.name.clone(), e.kind));
                }
            }
        }
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if hits.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new("Ничего не найдено").size(12.5).color(theme::FAINT));
                });
            }
            let mut go = None;
            for (dir, case_name, name, kind) in &hits {
                let r = row(ui, 14.0, kind_icon(*kind), kind.color(), &format!("{name}   ·  {case_name}"), false);
                if r.clicked() {
                    go = Some((dir.clone(), name.clone()));
                }
            }
            if locked > 0 {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new(format!("{}  Зашифрованные дела не просмотрены: {locked}", ic::LOCK_SIMPLE)).size(11.5).color(theme::FAINT));
                });
            }
            if let Some((dir, name)) = go {
                if self.current_dir().as_ref() != Some(&dir) {
                    self.open_case(&dir);
                }
                if let Some(i) = self.case.as_ref().and_then(|c| c.find(&name)) {
                    self.go(View::Entity(i));
                }
                self.search.clear();
            }
        });
    }

    fn pw_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.pw else { return };
        let name = p.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (title, hint) = match p.mode {
            PwMode::Unlock => ("Дело зашифровано", "Введите пароль, чтобы открыть его."),
            PwMode::Encrypt => ("Зашифровать дело", "Все карточки, заметки и приложенные файлы будут зашифрованы. Без пароля их не прочитать — восстановить пароль нельзя."),
            PwMode::Decrypt => ("Снять шифрование", "Файлы дела снова станут обычными. Введите пароль ещё раз."),
        };
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new(RichText::new(title).size(15.0))
            .collapsible(false)
            .resizable(false)
            .fixed_size([380.0, 0.0])
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("{}  {name}", ic::DETECTIVE)).color(theme::TEXT));
                ui.add_space(4.0);
                ui.label(RichText::new(hint).size(12.5).color(theme::MUTED));
                ui.add_space(10.0);
                let r = ui.add(input(&mut p.a, "пароль", f32::INFINITY).password(true));
                if !p.focus {
                    r.request_focus();
                    p.focus = true;
                }
                if p.mode == PwMode::Encrypt {
                    ui.add_space(6.0);
                    ui.add(input(&mut p.b, "ещё раз", f32::INFINITY).password(true));
                }
                if let Some(e) = &p.err {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).size(12.5).color(theme::DANGER));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let label = match p.mode {
                        PwMode::Unlock => "Открыть",
                        PwMode::Encrypt => "Зашифровать",
                        PwMode::Decrypt => "Снять шифрование",
                    };
                    submit = theme::pill(ui, ic::LOCK_SIMPLE, label).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                    cancel = ui.button("Отмена").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
                });
            });
        if cancel {
            self.pw = None;
            return;
        }
        if !submit {
            return;
        }
        let p = self.pw.as_ref().unwrap();
        let (dir, mode, a, b) = (p.dir.clone(), p.mode, p.a.clone(), p.b.clone());
        let res: Result<(), String> = match mode {
            PwMode::Unlock => crate::vault::unlock(&dir, &a).map(|k| {
                let case = Case::load_with(&dir, Some(k));
                self.flush();
                self.show_case(case);
            }),
            PwMode::Encrypt => {
                if a.chars().count() < 6 {
                    Err("Пароль слишком короткий: нужно хотя бы 6 символов".into())
                } else if a != b {
                    Err("Пароли не совпадают".into())
                } else {
                    let open = self.current_dir().as_ref() == Some(&dir);
                    self.flush();
                    if open {
                        self.case = None;
                    }
                    let pw = a.clone();
                    Case::encrypt(&dir, &pw).and_then(|_| crate::vault::unlock(&dir, &pw)).map(|k| {
                        if open {
                            self.show_case(Case::load_with(&dir, Some(k)));
                        }
                    })
                }
            }
            PwMode::Decrypt => crate::vault::unlock(&dir, &a).and_then(|k| {
                self.flush();
                self.case = None;
                Case::decrypt(&dir, k)?;
                self.show_case(Case::load(&dir));
                Ok(())
            }),
        };
        match res {
            Ok(()) => self.pw = None,
            Err(e) => {
                if let Some(p) = &mut self.pw {
                    p.err = Some(e);
                    p.a.clear();
                    p.b.clear();
                }
            }
        }
    }

    fn extract_window(&mut self, ctx: &egui::Context) {
        let Some(x) = &mut self.extract else { return };
        let mut open = true;
        let mut add = false;
        egui::Window::new(RichText::new("Найти сущности в тексте").size(15.0))
            .open(&mut open)
            .collapsible(false)
            .default_size([560.0, 480.0])
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("Вставьте любой текст: переписку, страницу, выгрузку. Найдутся почты, телефоны, домены, IP, @никнеймы и криптокошельки.").size(12.5).color(theme::MUTED));
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("x-text").max_height(170.0).show(ui, |ui| {
                    if ui.add(egui::TextEdit::multiline(&mut x.text).desired_rows(7).desired_width(f32::INFINITY).hint_text("Текст…")).changed() {
                        let known: Vec<String> = self.case.as_ref().map(|c| c.entities.iter().map(|e| e.name.to_lowercase()).collect()).unwrap_or_default();
                        x.found = crate::extract::find(&x.text).into_iter().map(|f| (!known.contains(&f.value.to_lowercase()), f)).collect();
                    }
                });
                ui.add_space(8.0);
                if x.found.is_empty() {
                    ui.label(RichText::new(if x.text.trim().is_empty() { "Пока пусто" } else { "Ничего не найдено" }).size(12.5).color(theme::FAINT));
                }
                egui::ScrollArea::vertical().id_salt("x-found").max_height(200.0).show(ui, |ui| {
                    for (on, f) in &mut x.found {
                        ui.horizontal(|ui| {
                            ui.checkbox(on, "");
                            ui.label(RichText::new(kind_icon(f.kind)).color(f.kind.color()));
                            ui.label(RichText::new(&f.value).color(theme::TEXT));
                            ui.label(RichText::new(f.what).size(11.5).color(theme::FAINT));
                        });
                    }
                });
                let n = x.found.iter().filter(|(on, _)| *on).count();
                if n > 0 {
                    ui.add_space(10.0);
                    add = theme::pill(ui, ic::PLUS, &format!("Добавить в дело: {n}")).clicked();
                }
            });
        if add {
            let picked: Vec<(Kind, String)> = x.found.iter().filter(|(on, _)| *on).map(|(_, f)| (f.kind, f.value.clone())).collect();
            if let Some(c) = &mut self.case {
                for (kind, name) in picked {
                    if c.find(&name).is_some() {
                        continue;
                    }
                    let i = c.add_entity(kind);
                    if let Err(e) = c.rename_entity(i, &name) {
                        self.error = Some(e);
                    }
                }
                self.edited();
            }
            open = false;
        }
        if !open {
            self.extract = None;
        }
    }

    fn map_view(&mut self, ui: &mut egui::Ui) {
        let Some(c) = &self.case else { return };
        let markers: Vec<(usize, crate::map::Marker)> = c
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let (lat, lon) = e.fields.iter().find_map(|(_, v)| crate::extract::coords(v))?;
                Some((i, crate::map::Marker { lat, lon, label: e.name.clone(), color: e.kind.color() }))
            })
            .collect();
        if markers.is_empty() {
            crate::empty_state(
                ui,
                ic::MAP_TRIFOLD,
                "На карте пока пусто",
                "Добавьте сущности с полем «Координаты» (например 55.7558, 37.6173) или найдите их по адресу в карточке места",
            );
            return;
        }
        let only: Vec<crate::map::Marker> = markers.iter().map(|(_, m)| crate::map::Marker { lat: m.lat, lon: m.lon, label: m.label.clone(), color: m.color }).collect();
        if let Some(k) = self.map.show(ui, &only) {
            self.go(View::Entity(markers[k].0));
        }
    }

    fn report(&mut self, pdf: bool) {
        self.flush();
        let Some(c) = &self.case else { return };
        let name = c.name();
        let (ext, label) = if pdf { ("pdf", "PDF") } else { ("md", "Markdown") };
        let Some(out) = rfd::FileDialog::new().set_file_name(format!("{name} — отчёт.{ext}")).add_filter(label, &[ext]).save_file() else { return };
        use crate::pdf::Block as B;
        let mut b = vec![B::Title(name.clone()), B::Small(format!("Отчёт по делу, {}. Сущностей: {}.", inv::today(), c.entities.len())), B::Gap];
        if !c.overview.notes.trim().is_empty() {
            b.push(B::Heading("Обзор".into()));
            for p in c.overview.notes.split("\n\n") {
                b.push(B::Para(p.trim().into()));
            }
        }
        for k in Kind::ALL {
            for e in c.entities.iter().filter(|e| e.kind == k) {
                b.push(B::Heading(format!("{} — {}", e.name, k.label().to_lowercase())));
                for (fk, fv) in e.fields.iter().filter(|(_, v)| !v.trim().is_empty()) {
                    b.push(B::Para(format!("{fk}: {fv}")));
                }
                for l in &e.links {
                    b.push(B::Bullet(if l.relation.is_empty() { format!("→ {}", l.target) } else { format!("→ {} ({})", l.target, l.relation) }));
                }
                for s in &e.sources {
                    let note = if s.note.is_empty() { String::new() } else { format!(" — {}", s.note) };
                    b.push(B::Small(format!("Источник: {}, {}, {}{note}", s.url, s.date, s.reliability.label())));
                }
                for ev in &e.events {
                    b.push(B::Bullet(format!("{} {}", ev.date, ev.text)));
                }
                if !e.notes.trim().is_empty() {
                    b.push(B::Quote(e.notes.trim().replace('\n', " ")));
                }
            }
        }
        let tl = c.timeline();
        if !tl.is_empty() {
            b.push(B::Heading("Хронология".into()));
            for t in tl {
                let who = t.owner.and_then(|i| c.entities.get(i)).map(|e| format!(" — {}", e.name)).unwrap_or_default();
                b.push(B::Bullet(format!("{} {}{who}", if t.date.is_empty() { "без даты" } else { &t.date }, t.text)));
            }
        }
        let res = if pdf {
            crate::pdf::write(&out, &name, &b)
        } else {
            let md: String = b
                .iter()
                .map(|x| match x {
                    B::Title(t) => format!("# {t}\n\n"),
                    B::Heading(t) => format!("\n## {t}\n\n"),
                    B::Para(t) => format!("{t}\n\n"),
                    B::Bullet(t) => format!("- {t}\n"),
                    B::Quote(t) => format!("\n> {t}\n\n"),
                    B::Small(t) => format!("*{t}*\n\n"),
                    B::Gap => "\n".into(),
                })
                .collect();
            std::fs::write(&out, md).map_err(|e| format!("Не удалось сохранить отчёт: {e}"))
        };
        if let Err(e) = res {
            self.error = Some(e);
        }
    }
}

// --------------------------------------------------------------- autosearch

impl Inv {
    fn start_auto(&mut self, case: PathBuf, entity: String, kind: Kind, value: String) {
        self.auto.insert((case.clone(), entity.clone()), (true, vec![]));
        let tx = self.jobs_tx.clone();
        std::thread::spawn(move || {
            let (t, c, e) = (tx.clone(), case.clone(), entity.clone());
            crate::autosearch::run(kind, &value, move |f| {
                let _ = t.send(Job::Auto { case: c.clone(), entity: e.clone(), finding: Some(f) });
            });
            let _ = tx.send(Job::Auto { case, entity, finding: None });
        });
    }

    /// Starts checks for every entity of the open case that has them.
    fn auto_case(&mut self) {
        let Some(c) = &self.case else { return };
        let jobs: Vec<(PathBuf, String, Kind, String)> = c
            .entities
            .iter()
            .map(|e| {
                let v = probe_value(e);
                (c.dir.clone(), e.name.clone(), e.kind, v)
            })
            .filter(|(_, _, k, v)| crate::autosearch::supported(*k, v))
            .collect();
        if jobs.is_empty() {
            self.error = Some("В деле нет никнеймов, почт, доменов, телефонов или IP для проверки".into());
        }
        for (d, n, k, v) in jobs {
            self.start_auto(d, n, k, v);
        }
    }

    /// Puts chosen findings into the card: sources, fields or new linked entities.
    fn apply_auto(&mut self, i: usize, key: &(PathBuf, String), picks: &[usize]) {
        use crate::autosearch::Add;
        let Some((_, findings)) = self.auto.get_mut(key) else { return };
        let Some(c) = &mut self.case else { return };
        let mut new_entities = vec![];
        for &k in picks {
            let Some((f, added)) = findings.get_mut(k) else { continue };
            let Some(add) = f.add.clone() else { continue };
            *added = true;
            let e = &mut c.entities[i];
            match add {
                Add::Source { url, note } => {
                    if !e.sources.iter().any(|s| s.url == url) {
                        e.sources.push(Source { url, date: inv::today(), reliability: Reliability::Likely, note });
                    }
                }
                Add::Field { key, value } => match e.fields.iter_mut().find(|(k, _)| *k == key) {
                    Some(f) if f.1.trim().is_empty() => f.1 = value,
                    Some(f) if f.1 != value => e.fields.push((key, value)),
                    Some(_) => {}
                    None => e.fields.push((key, value)),
                },
                Add::Entity { kind, name, relation } => new_entities.push((kind, name, relation, None)),
                Add::Profile { site, url } => {
                    let name = crate::autosearch::profile_name(&site, &url);
                    new_entities.push((Kind::Nick, name, format!("профиль на {site}"), Some((site, url))));
                }
            }
        }
        c.touch(Some(i));
        let from = c.entities[i].name.clone();
        // New nodes go on a ring around the card they came from, so they show up next to it on the graph.
        let origin = c.pos.get(&from).copied().unwrap_or([0.0, 0.0]);
        let total = new_entities.len().max(1);
        let ring = 170.0 + 6.0 * total as f32;
        let start = c.pos.len() as f32 * 0.7;
        for (n, (kind, name, relation, profile)) in new_entities.into_iter().enumerate() {
            if c.find(&name).is_none() {
                let j = c.add_entity(kind);
                if let Err(e) = c.rename_entity(j, &name) {
                    self.error = Some(e);
                    continue;
                }
                let j = c.find(&name).unwrap();
                if let Some((site, url)) = &profile {
                    let e = &mut c.entities[j];
                    e.fields = vec![("Платформа".into(), site.clone()), ("Профиль".into(), url.clone())];
                    e.sources.push(Source { url: url.clone(), date: inv::today(), reliability: Reliability::Likely, note: "найдено автопроверкой".into() });
                    c.touch(Some(j));
                }
                // Try spots around the source and take the one farthest from every other node.
                let _ = (start, n, total);
                let mut best = (f32::MIN, [origin[0] + ring, origin[1]]);
                for r in [ring, ring * 1.5, ring * 2.0] {
                    for k in 0..24 {
                        let a = k as f32 / 24.0 * std::f32::consts::TAU;
                        let p = [origin[0] + r * 1.3 * a.cos(), origin[1] + r * 0.8 * a.sin()];
                        let clear = c.pos.values().map(|q| ((p[0] - q[0]) / 1.8).hypot(p[1] - q[1])).fold(f32::MAX, f32::min);
                        let score = clear.min(150.0) - r * 0.05;
                        if score > best.0 {
                            best = (score, p);
                        }
                    }
                    if best.0 >= 140.0 {
                        break;
                    }
                }
                c.pos.insert(name.clone(), best.1);
                c.touch_pos();
                self.fit_next = true;
            }
            if let Some(src) = c.find(&from) {
                if !c.entities[src].links.iter().any(|l| l.target == name) {
                    c.entities[src].links.push(Link { target: name, relation });
                    c.touch(Some(src));
                }
            }
        }
        // Entity indexes may have shifted after sorting; keep the card that was open.
        if let Some(j) = c.find(&from) {
            self.view = View::Entity(j);
        }
        self.edited();
    }
}
