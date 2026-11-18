//! The ⌘K palette: one box to find notes, cases and cards and to run commands.

use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as ic;

use crate::theme;

pub struct Item<C> {
    pub icon: &'static str,
    pub color: Color32,
    pub label: String,
    /// Small grey text on the right: where it lives or what it is.
    pub hint: String,
    /// Stable id for "recently used".
    pub key: String,
    pub cmd: C,
}

#[derive(Default)]
pub struct State {
    pub query: String,
    pub sel: usize,
    pub focus: bool,
}

/// Does every character of the query appear in order? Lower score is better.
pub fn fuzzy(hay: &str, q: &str) -> Option<usize> {
    let hay = hay.to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    if let Some(i) = hay.find(q) {
        return Some(i);
    }
    let mut it = hay.chars().enumerate();
    let mut last = 0;
    for qc in q.chars() {
        let (i, _) = it.find(|(_, c)| *c == qc)?;
        last = i;
    }
    Some(100 + last)
}

/// Best matches first; recently used ones win ties and lead an empty query.
pub fn rank<C>(items: Vec<Item<C>>, query: &str, recent: &[String], max: usize) -> Vec<Item<C>> {
    let q = query.trim().to_lowercase();
    let mut v: Vec<(usize, usize, Item<C>)> = items
        .into_iter()
        .filter_map(|it| {
            let s = fuzzy(&it.label, &q).or_else(|| fuzzy(&it.hint, &q).map(|s| s + 60))?;
            let r = recent.iter().position(|k| *k == it.key).unwrap_or(usize::MAX);
            Some((s, r, it))
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(max).map(|t| t.2).collect()
}

/// Put `key` at the front of the recent list (at most 10).
pub fn remember(recent: &mut Vec<String>, key: &str) {
    recent.retain(|k| k != key);
    recent.insert(0, key.to_string());
    recent.truncate(10);
}

pub enum Outcome {
    None,
    Close,
    Run(usize),
    /// Enter with nothing found: the typed text.
    Create(String),
}

pub fn show<C>(ctx: &egui::Context, st: &mut State, items: &[Item<C>]) -> Outcome {
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if down {
        st.sel = (st.sel + 1).min(items.len().saturating_sub(1));
    }
    if up {
        st.sel = st.sel.saturating_sub(1);
    }
    st.sel = st.sel.min(items.len().saturating_sub(1));
    let mut out = if esc { Outcome::Close } else { Outcome::None };
    egui::Window::new("palette")
        .title_bar(false)
        .resizable(false)
        .fixed_size([560.0, 0.0])
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
        .show(ctx, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .hint_text(format!("{}  Заметка, дело, карточка или команда", ic::MAGNIFYING_GLASS))
                    .desired_width(f32::INFINITY)
                    .font(theme::font(15.0))
                    .margin(egui::vec2(10.0, 8.0)),
            );
            if !st.focus {
                r.request_focus();
                st.focus = true;
            }
            if r.changed() {
                st.sel = 0;
            }
            ui.add_space(4.0);
            if items.is_empty() {
                ui.label(RichText::new("Ничего не найдено. Enter создаст такую заметку").size(12.5).color(theme::FAINT));
            }
            egui::ScrollArea::vertical().max_height(380.0).min_scrolled_height((items.len() as f32 * 32.0).min(380.0)).auto_shrink([false, true]).show(ui, |ui| {
                for (k, it) in items.iter().enumerate() {
                    let on = k == st.sel;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
                    if on {
                        ui.painter().rect_filled(rect, 6.0, theme::SELECTED);
                        if down || up {
                            ui.scroll_to_rect(rect, None);
                        }
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, theme::tint(theme::SELECTED, 0.5));
                    }
                    let p = ui.painter();
                    let y = rect.center().y;
                    p.text(egui::pos2(rect.left() + 12.0, y), egui::Align2::LEFT_CENTER, it.icon, theme::font(14.0), it.color);
                    p.text(egui::pos2(rect.left() + 36.0, y), egui::Align2::LEFT_CENTER, &it.label, theme::font(13.5), if on { theme::TEXT } else { theme::MUTED });
                    p.text(egui::pos2(rect.right() - 10.0, y), egui::Align2::RIGHT_CENTER, &it.hint, theme::font(11.5), theme::FAINT);
                    if resp.clicked() {
                        out = Outcome::Run(k);
                    }
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("↑↓ выбрать   Enter выполнить   Esc закрыть").size(11.0).color(theme::FAINT));
            });
        });
    if enter {
        out = if items.is_empty() {
            if st.query.trim().is_empty() { Outcome::Close } else { Outcome::Create(st.query.trim().to_string()) }
        } else {
            Outcome::Run(st.sel)
        };
    }
    out
}
