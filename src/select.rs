//! Picking several rows at once: Ctrl/⌘-click toggles a row, dragging a frame
//! (with Ctrl/⌘ held, or starting on empty space) picks every row it touches.

use eframe::egui::{self, Pos2, Rect, Stroke};
use std::collections::HashSet;
use std::hash::Hash;

use crate::theme;

pub struct Picker<K> {
    pub picked: HashSet<K>,
    /// Rows drawn this frame, in screen coordinates.
    rows: Vec<(K, Rect)>,
    /// Frame start and what was picked before it began (kept when Ctrl/⌘ is held).
    drag: Option<(Pos2, HashSet<K>)>,
}

impl<K> Default for Picker<K> {
    fn default() -> Self {
        Self { picked: HashSet::new(), rows: Vec::new(), drag: None }
    }
}

/// Ctrl on any system, ⌘ on macOS.
pub fn multi(ui: &egui::Ui) -> bool {
    ui.input(|i| i.modifiers.ctrl || i.modifiers.command)
}

impl<K: Hash + Eq + Clone> Picker<K> {
    pub fn begin(&mut self) {
        self.rows.clear();
    }

    pub fn row(&mut self, k: K, r: Rect) {
        self.rows.push((k, r));
    }

    pub fn is_picked(&self, k: &K) -> bool {
        self.picked.contains(k)
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn toggle(&mut self, k: K) {
        if !self.picked.remove(&k) {
            self.picked.insert(k);
        }
    }

    pub fn clear(&mut self) {
        self.picked.clear();
    }

    /// Call after all rows are drawn. `area` is the visible list area.
    pub fn finish(&mut self, ui: &egui::Ui, area: Rect) {
        let (pressed, down, pos, origin, multi) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
                i.modifiers.ctrl || i.modifiers.command,
            )
        });
        if pressed {
            if let Some(o) = origin.filter(|o| area.contains(*o)) {
                let on_row = self.rows.iter().any(|(_, r)| r.contains(o));
                let free = ui.ctx().layer_id_at(o) == Some(ui.layer_id());
                if free && (multi || !on_row) {
                    let keep = if multi { self.picked.clone() } else { HashSet::new() };
                    self.drag = Some((o, keep));
                }
            }
        }
        let Some((start, keep)) = &self.drag else { return };
        if !down {
            // A plain click on empty space just clears the picks.
            self.drag = None;
            return;
        }
        let Some(now) = pos else { return };
        let frame = Rect::from_two_pos(*start, now);
        if frame.size().length() < 4.0 {
            return;
        }
        let mut picked = keep.clone();
        for (k, r) in &self.rows {
            if r.intersects(frame) {
                picked.insert(k.clone());
            }
        }
        self.picked = picked;
        let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("pick_frame")));
        let shown = frame.intersect(area);
        p.rect(shown, 3.0, theme::tint(theme::ACCENT, 0.10), Stroke::new(1.0, theme::tint(theme::ACCENT, 0.7)), egui::StrokeKind::Inside);
        // Scroll the list when the frame reaches its edge.
        if now.y > area.bottom() - 16.0 {
            ui.scroll_with_delta(egui::vec2(0.0, -8.0));
        } else if now.y < area.top() + 16.0 {
            ui.scroll_with_delta(egui::vec2(0.0, 8.0));
        }
        ui.ctx().request_repaint();
    }
}

/// Fill for a picked row.
pub fn paint_picked(ui: &egui::Ui, r: Rect) {
    ui.painter().rect(r, 6.0, theme::ACCENT_BG, Stroke::new(1.0, theme::tint(theme::ACCENT, 0.45)), egui::StrokeKind::Inside);
}
