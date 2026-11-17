//! A small slippy map with OpenStreetMap tiles, for places in an investigation.
//! Tiles load in the background and stay in memory for the session.

use crate::theme;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};

type TileId = (u8, u32, u32);
const TILE: f32 = 256.0;
const UA: &str = "QuietNote/0.4 (+https://github.com/aristocratonly/quiet-note)";

pub struct MapView {
    /// Centre in world pixels at zoom 0 (0..256).
    center: (f64, f64),
    zoom: f64,
    tiles: HashMap<TileId, TextureHandle>,
    pending: HashSet<TileId>,
    failed: HashSet<TileId>,
    tx: Sender<(TileId, Option<Vec<u8>>)>,
    rx: Receiver<(TileId, Option<Vec<u8>>)>,
    pub fitted: bool,
}

pub struct Marker {
    pub lat: f64,
    pub lon: f64,
    pub label: String,
    pub color: Color32,
}

fn project(lat: f64, lon: f64) -> (f64, f64) {
    let x = (lon + 180.0) / 360.0 * 256.0;
    let s = lat.to_radians().sin().clamp(-0.9999, 0.9999);
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * 256.0;
    (x, y)
}

impl Default for MapView {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { center: (128.0, 128.0), zoom: 2.0, tiles: HashMap::new(), pending: HashSet::new(), failed: HashSet::new(), tx, rx, fitted: false }
    }
}

impl MapView {
    fn fit(&mut self, markers: &[Marker], size: Vec2) {
        if markers.is_empty() {
            return;
        }
        let pts: Vec<(f64, f64)> = markers.iter().map(|m| project(m.lat, m.lon)).collect();
        let (minx, maxx) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
        let (miny, maxy) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        self.center = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
        let span = (maxx - minx).max(maxy - miny).max(1e-6);
        let fit = ((size.x.min(size.y) as f64 * 0.6) / span).log2();
        self.zoom = if markers.len() == 1 { 12.0 } else { fit.clamp(2.0, 16.0) };
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((id, data)) = self.rx.try_recv() {
            self.pending.remove(&id);
            let img = data.and_then(|d| image::load_from_memory(&d).ok());
            match img {
                Some(img) => {
                    let rgba = img.to_rgba8();
                    let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba);
                    self.tiles.insert(id, ctx.load_texture(format!("tile-{id:?}"), ci, egui::TextureOptions::LINEAR));
                }
                None => {
                    self.failed.insert(id);
                }
            }
        }
    }

    fn request(&mut self, id: TileId, ctx: &egui::Context) {
        if self.tiles.contains_key(&id) || self.pending.contains(&id) || self.failed.contains(&id) || self.pending.len() > 8 {
            return;
        }
        self.pending.insert(id);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let url = format!("https://tile.openstreetmap.org/{}/{}/{}.png", id.0, id.1, id.2);
            let data = ureq::get(&url).set("User-Agent", UA).timeout(std::time::Duration::from_secs(15)).call().ok().and_then(|r| {
                let mut v = vec![];
                std::io::Read::read_to_end(&mut r.into_reader(), &mut v).ok().map(|_| v)
            });
            let _ = tx.send((id, data));
            ctx.request_repaint();
        });
    }

    /// Draws the map; returns the index of a clicked marker.
    pub fn show(&mut self, ui: &mut egui::Ui, markers: &[Marker]) -> Option<usize> {
        let ctx = ui.ctx().clone();
        self.receive(&ctx);
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if !self.fitted {
            self.fit(markers, rect.size());
            self.fitted = true;
        }
        let scale = 2f64.powf(self.zoom);
        if resp.dragged() {
            let d = resp.drag_delta();
            self.center.0 -= d.x as f64 / scale;
            self.center.1 -= d.y as f64 / scale;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
            if scroll != 0.0 {
                self.zoom = (self.zoom + scroll as f64 / 200.0).clamp(1.0, 18.0);
            }
        }
        let scale = 2f64.powf(self.zoom);
        let center = self.center;
        let to_screen = move |wx: f64, wy: f64| -> Pos2 {
            Pos2::new(rect.center().x + ((wx - center.0) * scale) as f32, rect.center().y + ((wy - center.1) * scale) as f32)
        };
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, Color32::from_rgb(0x10, 0x12, 0x15));

        // Tiles at the integer zoom, scaled to the fractional one.
        let z = self.zoom.floor().clamp(0.0, 18.0) as u8;
        let n = 1u32 << z;
        let tile_world = 256.0 / n as f64;
        let tile_px = (tile_world * scale) as f32;
        let tl = (center.0 - rect.width() as f64 / 2.0 / scale, center.1 - rect.height() as f64 / 2.0 / scale);
        let br = (self.center.0 + rect.width() as f64 / 2.0 / scale, self.center.1 + rect.height() as f64 / 2.0 / scale);
        let (x0, x1) = ((tl.0 / tile_world).floor() as i64, (br.0 / tile_world).floor() as i64);
        let (y0, y1) = (((tl.1 / tile_world).floor() as i64).max(0), ((br.1 / tile_world).floor() as i64).min(n as i64 - 1));
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let wrapped = tx.rem_euclid(n as i64) as u32;
                let id = (z, wrapped, ty as u32);
                let min = to_screen(tx as f64 * tile_world, ty as f64 * tile_world);
                let r = Rect::from_min_size(min, Vec2::splat(tile_px));
                match self.tiles.get(&id) {
                    Some(t) => {
                        // Dim and cool the tiles so they sit in the dark theme.
                        p.image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_rgb(0x8a, 0x92, 0xa0));
                    }
                    None => {
                        p.rect_stroke(r, 0.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
                        self.request(id, &ctx);
                    }
                }
            }
        }
        p.rect_filled(rect, 0.0, Color32::from_black_alpha(70));

        let mut clicked = None;
        let hover = resp.hover_pos();
        for (i, m) in markers.iter().enumerate() {
            let (wx, wy) = project(m.lat, m.lon);
            let c = to_screen(wx, wy);
            let near = hover.is_some_and(|h| h.distance(c) < 12.0);
            p.circle_filled(c, if near { 9.0 } else { 7.0 }, m.color);
            p.circle_stroke(c, if near { 9.0 } else { 7.0 }, Stroke::new(2.0, Color32::from_black_alpha(180)));
            let g = p.layout_no_wrap(m.label.clone(), theme::font(12.5), theme::TEXT);
            let lr = Rect::from_min_size(c + Vec2::new(12.0, -g.size().y / 2.0 - 3.0), g.size() + Vec2::new(12.0, 6.0));
            p.rect_filled(lr, 6.0, Color32::from_black_alpha(190));
            p.galley(lr.min + Vec2::new(6.0, 3.0), g, theme::TEXT);
            if near {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        }
        p.text(rect.right_bottom() - Vec2::new(8.0, 6.0), Align2::RIGHT_BOTTOM, "© OpenStreetMap", theme::font(10.5), theme::FAINT);
        clicked
    }
}

/// Address → coordinates through OpenStreetMap Nominatim. Sends the address to that service.
pub fn geocode(address: &str) -> Result<(f64, f64), String> {
    let url = format!("https://nominatim.openstreetmap.org/search?format=json&limit=1&q={}", crate::extract::enc(address));
    let body = ureq::get(&url).set("User-Agent", UA).timeout(std::time::Duration::from_secs(15)).call().map_err(|e| format!("Не удалось найти адрес: {e}"))?.into_string().map_err(|e| e.to_string())?;
    let num = |key: &str| -> Option<f64> {
        let i = body.find(&format!("\"{key}\":\""))? + key.len() + 4;
        body[i..].split('"').next()?.parse().ok()
    };
    match (num("lat"), num("lon")) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err("Адрес не найден на карте".into()),
    }
}
