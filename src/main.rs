#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod fsops;
mod inv_ui;
mod investigation;
mod theme;

use eframe::egui::{self, FontId, RichText, Sense, Stroke};
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
                match std::fs::write(&target, &self.text) {
                    Ok(_) => self.dirty = false,
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
                fsops::rename(&p, &name).map(|np| self.rebase(&p, &np))
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
            if !filter.is_empty() && !fsops::matches(&entry, &filter) {
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
            resp.dnd_set_drag_payload(entry.clone());

            let drop_target = if is_dir { entry.clone() } else { dir.to_path_buf() };
            let hovering = resp.dnd_hover_payload::<PathBuf>().is_some();
            if let Some(src) = resp.dnd_release_payload::<PathBuf>() {
                actions.push(Action::Move((*src).clone(), drop_target));
            }

            let row = rect.shrink2(egui::vec2(8.0, 1.0));
            let p = ui.painter();
            if hovering && is_dir {
                p.rect(row, 6.0, theme::DROP, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
            } else if selected {
                p.rect_filled(row, 6.0, theme::SELECTED);
            } else if resp.hovered() {
                p.rect_filled(row, 6.0, theme::HOVER);
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

            if resp.clicked() {
                actions.push(if is_dir { Action::Toggle(entry.clone()) } else { Action::Open(entry.clone()) });
            }
            if resp.double_clicked() {
                actions.push(Action::StartRename(entry.clone()));
            }
            let parent_for_new = if is_dir { entry.clone() } else { dir.to_path_buf() };
            resp.context_menu(|ui| {
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

            if is_dir && expanded {
                self.tree(ui, &entry, depth + 1, actions);
            }
        }
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

                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    theme::caption(ui, "Заметки");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        if theme::icon_button(ui, ic::FOLDER_PLUS, "Новая папка  ⇧⌘N").clicked() {
                            actions.push(Action::NewFolder(self.s.root.clone()));
                        }
                        if theme::icon_button(ui, ic::NOTE_PENCIL, "Новая заметка  ⌘N").clicked() {
                            actions.push(Action::NewNote(self.s.root.clone()));
                        }
                    });
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let root = self.s.root.clone();
                    self.tree(ui, &root, 0, actions);

                    // Empty space below the tree: drop here to move to the root, right-click to create.
                    let (rect, resp) = ui.allocate_exact_size(ui.available_size().max(egui::vec2(0.0, 60.0)), Sense::click());
                    if resp.dnd_hover_payload::<PathBuf>().is_some() {
                        ui.painter().rect_stroke(rect.shrink(8.0), 6.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
                        ui.painter().text(rect.center_top() + egui::vec2(0.0, 26.0), egui::Align2::CENTER_CENTER, "В корень", theme::font(12.0), theme::ACCENT);
                    }
                    if let Some(src) = resp.dnd_release_payload::<PathBuf>() {
                        actions.push(Action::Move((*src).clone(), root.clone()));
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
            empty_state(ui, ic::NOTE_PENCIL, "Пустой лист", "⌘N — новая заметка,  ⇧⌘N — новая папка");
            return;
        };

        // Breadcrumbs
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            let rel = path.strip_prefix(&self.s.root).unwrap_or(&path).with_extension("");
            let parts: Vec<String> = rel.iter().map(|c| c.to_string_lossy().into_owned()).collect();
            for (i, part) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                ui.label(RichText::new(part).size(12.5).color(if last { theme::MUTED } else { theme::FAINT }));
                if !last {
                    ui.label(RichText::new(ic::CARET_RIGHT).size(10.0).color(theme::FAINT));
                }
            }
        });

        // Status bar
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
                    });
                });
            });

        let font = FontId::new(self.s.font_size, theme::family_for(self.s.text_style));
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let avail = ui.available_width();
            let col = if self.s.wide { avail - 96.0 } else { (self.s.font_size * 42.0).min(avail - 96.0) };
            let side = ((avail - col) / 2.0).clamp(24.0, 127.0);
            let edit = egui::TextEdit::multiline(&mut self.text)
                .frame(false)
                .font(font)
                .text_color(theme::TEXT)
                .hint_text(RichText::new("Начните писать…").color(theme::FAINT))
                .margin(egui::Margin { left: side as i8, right: side as i8, top: 22, bottom: 60 })
                .lock_focus(true)
                .desired_width(f32::INFINITY);
            let r = ui.add_sized(ui.available_size(), edit);
            if self.focus_editor {
                r.request_focus();
                self.focus_editor = false;
            }
            if r.changed() {
                self.dirty = true;
                self.last_edit = Instant::now();
            }
        });
    }

    fn ui(&mut self, ctx: &egui::Context) {
        let mut actions = Vec::new();

        let (new_note, new_folder, save) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::N),
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
            )
        });
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

        self.sidebar(ctx, &mut actions);

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

        for a in actions {
            self.apply(a);
        }
        self.inv.tick(ctx);

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
