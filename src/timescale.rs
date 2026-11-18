//! The case timeline as a horizontal scale: years and months on an axis,
//! one dot per event, stacked when they share a day.

use crate::investigation::{Case, Key, Kind, TimelineItem};
use crate::inv_ui::kind_icon;
use crate::theme::{self, font};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashSet;

/// Visible window of the scale: day at the left edge and pixels per day.
#[derive(Default)]
pub struct Scale {
    origin: f64,
    px_day: f64,
    fitted: bool,
}

impl Scale {
    pub fn refit(&mut self) {
        self.fitted = false;
    }
}

/// Days since 1970-01-01 for "YYYY", "YYYY-MM" or "YYYY-MM-DD".
pub fn day_num(s: &str) -> Option<f64> {
    let mut it = s.trim().split('-');
    let y: i64 = it.next()?.trim().parse().ok()?;
    if !(1000..=9999).contains(&y) {
        return None;
    }
    let m: i64 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(1).clamp(1, 12);
    let d: i64 = it.next().and_then(|v| v.get(..2.min(v.len())).and_then(|v| v.parse().ok())).unwrap_or(1).clamp(1, 31);
    Some(civil_days(y, m, d) as f64)
}

/// Howard Hinnant's days_from_civil.
fn civil_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

const MONTHS: [&str; 12] = ["янв", "фев", "мар", "апр", "май", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];

/// Chips that hide and show kinds of cards. Returns true when something changed.
pub fn kind_filter(ui: &mut egui::Ui, present: &[Kind], hidden: &mut HashSet<Kind>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for &k in present {
            let off = hidden.contains(&k);
            let text = format!("{}  {}", kind_icon(k), k.label());
            let col = if off { theme::FAINT } else { theme::TEXT };
            let g = ui.painter().layout_no_wrap(text, font(12.0), col);
            let (rect, resp) = ui.allocate_exact_size(g.size() + egui::vec2(18.0, 8.0), Sense::click());
            let fill = if off { Color32::TRANSPARENT } else { theme::tint(k.color(), if resp.hovered() { 0.24 } else { 0.14 }) };
            ui.painter().rect(rect, 8.0, fill, Stroke::new(1.0, theme::tint(k.color(), if off { 0.25 } else { 0.0 })), egui::StrokeKind::Inside);
            ui.painter().galley(rect.min + egui::vec2(9.0, 4.0), g, col);
            let resp = resp.on_hover_text(if off { "Показать" } else { "Скрыть" }).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                if off {
                    hidden.remove(&k);
                } else {
                    hidden.insert(k);
                }
                changed = true;
            }
        }
    });
    changed
}

/// Draw the scale. Returns the event's owner when a dot is clicked.
pub fn show(ui: &mut egui::Ui, st: &mut Scale, case: &Case, items: &[&TimelineItem]) -> Option<Key> {
    let mut goto = None;
    let dated: Vec<(f64, &TimelineItem)> = items.iter().filter_map(|it| day_num(&it.date).map(|d| (d, *it))).collect();
    let undated: Vec<&TimelineItem> = items.iter().filter(|it| day_num(&it.date).is_none()).copied().collect();

    let h = 340.0;
    let (rect, bg) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect(rect, 12.0, theme::tint(theme::SELECTED, 0.55), Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);

    let (lo, hi) = dated.iter().fold((f64::MAX, f64::MIN), |(a, b), (d, _)| (a.min(*d), b.max(*d)));
    if !st.fitted && !dated.is_empty() {
        let span = (hi - lo).max(60.0);
        st.px_day = (rect.width() as f64 - 120.0) / span;
        st.origin = lo - 60.0 / st.px_day - (span - (hi - lo)) / 2.0;
        st.fitted = true;
    }
    if st.px_day <= 0.0 {
        st.px_day = 1.0;
    }

    // Drag to move, ⌘/Ctrl + wheel or pinch to zoom, sideways wheel to move.
    if bg.dragged() {
        st.origin -= bg.drag_delta().x as f64 / st.px_day;
    }
    if bg.hovered() {
        let (zoom, dx) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.x));
        if dx != 0.0 {
            st.origin -= dx as f64 / st.px_day;
        }
        if zoom != 1.0 {
            if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                let at = st.origin + (ptr.x - rect.left()) as f64 / st.px_day;
                st.px_day = (st.px_day * zoom as f64).clamp(0.004, 80.0);
                st.origin = at - (ptr.x - rect.left()) as f64 / st.px_day;
            }
        }
    }
    let x_of = |d: f64| rect.left() + ((d - st.origin) * st.px_day) as f32;
    let axis_y = rect.bottom() - 44.0;
    p.hline(rect.x_range().shrink(14.0), axis_y, Stroke::new(1.0, theme::BORDER));

    if dated.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, "Нет событий с датой. Дата пишется как ГГГГ-ММ-ДД", font(13.0), theme::FAINT);
    } else {
        // Ticks: years always, months when there is room.
        let end = st.origin + rect.width() as f64 / st.px_day;
        let y0 = 1970 + (st.origin / 365.2425).floor() as i64 - 1;
        let y1 = 1970 + (end / 365.2425).ceil() as i64 + 1;
        let year_px = st.px_day * 365.2425;
        let every = [1, 2, 5, 10, 25, 50, 100].into_iter().find(|n| year_px * *n as f64 >= 56.0).unwrap_or(200);
        let months = year_px >= 12.0 * 34.0;
        for y in y0..=y1 {
            if months {
                for m in 2..=12 {
                    let x = x_of(civil_days(y, m, 1) as f64);
                    if rect.x_range().contains(x) {
                        p.vline(x, (axis_y - 3.0)..=(axis_y + 3.0), Stroke::new(1.0, theme::BORDER));
                        p.text(Pos2::new(x, axis_y + 14.0), Align2::CENTER_CENTER, MONTHS[(m - 1) as usize], font(10.5), theme::FAINT);
                    }
                }
            }
            if y % every != 0 {
                continue;
            }
            let x = x_of(civil_days(y, 1, 1) as f64);
            if !rect.x_range().contains(x) {
                continue;
            }
            p.vline(x, (rect.top() + 14.0)..=(axis_y + 5.0), Stroke::new(1.0, theme::tint(theme::BORDER, 0.5)));
            p.text(Pos2::new(x, axis_y + if months { 28.0 } else { 16.0 }), Align2::CENTER_CENTER, y.to_string(), font(11.5), theme::MUTED);
        }

        // Dots, stacked when they fall on the same few pixels.
        let mut stack: Vec<(f32, usize)> = Vec::new();
        for (d, it) in &dated {
            let x = x_of(*d);
            if !(rect.left() - 8.0..=rect.right() + 8.0).contains(&x) {
                continue;
            }
            let n = match stack.iter_mut().find(|(sx, _)| (sx - x).abs() < 11.0) {
                Some(s) => {
                    s.1 += 1;
                    s.1 - 1
                }
                None => {
                    stack.push((x, 1));
                    0
                }
            };
            let c = Pos2::new(x, axis_y - 16.0 - n as f32 * 15.0);
            if c.y < rect.top() + 10.0 {
                continue;
            }
            let (col, who) = match it.owner {
                None => (theme::ACCENT, "Дело".to_string()),
                Some(i) => (case.entities[i].kind.color(), case.entities[i].name.clone()),
            };
            let id = ui.id().with(("tl_dot", it.date.as_str(), it.text.as_str(), n));
            let r = ui.interact(Rect::from_center_size(c, Vec2::splat(13.0)), id, Sense::click());
            let hot = r.hovered();
            p.circle(c, if hot { 6.5 } else { 5.0 }, if hot { col } else { theme::BG }, Stroke::new(2.0, col));
            if n == 0 {
                p.vline(x, (c.y + 6.0)..=axis_y, Stroke::new(1.0, theme::tint(col, 0.35)));
            }
            let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_ui(|ui| {
                ui.label(RichText::new(&it.date).monospace().size(11.5).color(theme::MUTED));
                ui.label(RichText::new(&who).size(13.0).color(col));
                if !it.text.is_empty() {
                    ui.label(RichText::new(&it.text).size(13.0).color(theme::TEXT));
                }
            });
            if r.clicked() {
                goto = Some(it.owner);
            }
        }
    }

    // Zoom buttons in the corner.
    let mut tools = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.right_top() + egui::vec2(-112.0, 10.0), egui::vec2(104.0, 26.0))));
    tools.horizontal(|ui| {
        if theme::icon_button(ui, egui_phosphor::regular::MINUS, "Мельче").clicked() {
            st.px_day = (st.px_day / 1.5).max(0.004);
        }
        if theme::icon_button(ui, egui_phosphor::regular::PLUS, "Крупнее").clicked() {
            st.px_day = (st.px_day * 1.5).min(80.0);
        }
        if theme::icon_button(ui, egui_phosphor::regular::CORNERS_OUT, "Вместить всё").clicked() {
            st.fitted = false;
        }
    });
    if bg.double_clicked() {
        st.fitted = false;
    }

    if !undated.is_empty() {
        ui.add_space(14.0);
        ui.label(RichText::new(format!("Без даты: {}", undated.len())).size(12.0).color(theme::FAINT));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for it in undated {
                let (name, kind) = match it.owner {
                    Some(i) => (case.entities[i].name.as_str(), case.entities[i].kind),
                    None => ("Дело", Kind::Other),
                };
                let label = if it.text.is_empty() { name.to_string() } else { format!("{name}: {}", it.text) };
                if ui.add(egui::Button::new(RichText::new(format!("{}  {label}", kind_icon(kind))).size(12.0).color(theme::MUTED)).fill(theme::SURFACE).corner_radius(8)).clicked() {
                    goto = Some(it.owner);
                }
            }
        });
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Тяните, чтобы сдвинуть. ⌘ + колесо или щипок меняет масштаб. Двойной клик вмещает всё").size(11.5).color(theme::FAINT));
    goto
}
