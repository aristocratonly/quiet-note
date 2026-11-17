//! Small PDF writer for note export and case reports, set in the bundled Literata.
//! Each face ships as separate Latin and Cyrillic files, so every run of text
//! picks the file that has its glyphs.

use printpdf::{IndirectFontRef, Mm, PdfDocument, PdfDocumentReference, PdfLayerReference};

pub enum Block {
    Title(String),
    Heading(String),
    Para(String),
    Bullet(String),
    Quote(String),
    Small(String),
    Gap,
}

struct Face {
    lat: &'static [u8],
    cyr: &'static [u8],
}

const REGULAR: Face = Face {
    lat: include_bytes!("../assets/fonts/literata-latin-400-normal.ttf"),
    cyr: include_bytes!("../assets/fonts/literata-cyrillic-400-normal.ttf"),
};
const BOLD: Face = Face {
    lat: include_bytes!("../assets/fonts/literata-latin-600-normal.ttf"),
    cyr: include_bytes!("../assets/fonts/literata-cyrillic-600-normal.ttf"),
};
const ITALIC: Face = Face {
    lat: include_bytes!("../assets/fonts/literata-latin-400-italic.ttf"),
    cyr: include_bytes!("../assets/fonts/literata-cyrillic-400-italic.ttf"),
};

fn is_cyr(c: char) -> bool {
    matches!(c as u32, 0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F)
}

struct Loaded {
    lat: (IndirectFontRef, ttf_parser::Face<'static>),
    cyr: (IndirectFontRef, ttf_parser::Face<'static>),
}

impl Loaded {
    fn new(doc: &PdfDocumentReference, f: &Face) -> Result<Self, String> {
        let load = |b: &'static [u8]| -> Result<(IndirectFontRef, ttf_parser::Face<'static>), String> {
            let r = doc.add_external_font(b).map_err(|e| e.to_string())?;
            Ok((r, ttf_parser::Face::parse(b, 0).map_err(|e| e.to_string())?))
        };
        Ok(Self { lat: load(f.lat)?, cyr: load(f.cyr)? })
    }

    fn pick(&self, c: char) -> &(IndirectFontRef, ttf_parser::Face<'static>) {
        let cyr_has = self.cyr.1.glyph_index(c).is_some();
        if (is_cyr(c) && cyr_has) || self.lat.1.glyph_index(c).is_none() && cyr_has {
            &self.cyr
        } else {
            &self.lat
        }
    }

    /// Width of text in mm at the given size in pt.
    fn width(&self, text: &str, pt: f32) -> f32 {
        text.chars()
            .map(|c| {
                let (_, f) = self.pick(c);
                let adv = f.glyph_index(c).and_then(|g| f.glyph_hor_advance(g)).unwrap_or(f.units_per_em() / 2);
                adv as f32 / f.units_per_em() as f32 * pt
            })
            .sum::<f32>()
            * 0.3528
    }

    /// Splits text into runs of one font file.
    fn runs<'a>(&self, text: &'a str) -> Vec<(&IndirectFontRef, &'a str)> {
        let which = |c: char| std::ptr::eq(self.pick(c), &self.cyr);
        let mut out = vec![];
        let mut start = 0;
        let mut cur: Option<bool> = None;
        for (i, c) in text.char_indices() {
            if c == ' ' && cur.is_some() {
                continue; // spaces join the current run
            }
            let w = which(c);
            if cur != Some(w) {
                if let Some(p) = cur {
                    out.push((if p { &self.cyr.0 } else { &self.lat.0 }, &text[start..i]));
                }
                start = i;
                cur = Some(w);
            }
        }
        if let Some(p) = cur {
            out.push((if p { &self.cyr.0 } else { &self.lat.0 }, &text[start..]));
        }
        out
    }
}

const W: f32 = 210.0;
const H: f32 = 297.0;
const MARGIN: f32 = 22.0;

struct Writer {
    doc: PdfDocumentReference,
    layer: PdfLayerReference,
    y: f32,
}

impl Writer {
    fn new_page(&mut self) {
        let (p, l) = self.doc.add_page(Mm(W), Mm(H), "text");
        self.layer = self.doc.get_page(p).get_layer(l);
        self.y = H - MARGIN;
    }

    fn line(&mut self, font: &Loaded, text: &str, pt: f32, x: f32) {
        let mut cx = x;
        for (f, run) in font.runs(text) {
            self.layer.use_text(run, pt, Mm(cx), Mm(self.y), f);
            cx += font.width(run, pt);
        }
    }

    /// Wrapped paragraph; `lead` is line height in mm.
    fn para(&mut self, font: &Loaded, text: &str, pt: f32, lead: f32, indent: f32) {
        let max = W - 2.0 * MARGIN - indent;
        for hard in text.split('\n') {
            let mut line = String::new();
            for word in hard.split(' ') {
                let probe = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if font.width(&probe, pt) > max && !line.is_empty() {
                    self.emit(font, &line, pt, lead, indent);
                    line = word.to_string();
                } else {
                    line = probe;
                }
            }
            self.emit(font, &line, pt, lead, indent);
        }
    }

    fn emit(&mut self, font: &Loaded, line: &str, pt: f32, lead: f32, indent: f32) {
        if self.y - lead < MARGIN {
            self.new_page();
        }
        self.y -= lead;
        self.line(font, line, pt, MARGIN + indent);
    }
}

/// Strips inline Markdown markers for print.
pub fn plain(s: &str) -> String {
    s.replace("**", "").replace("[[", "").replace("]]", "").replace('`', "")
}

pub fn write(path: &std::path::Path, title: &str, blocks: &[Block]) -> Result<(), String> {
    let (doc, p, l) = PdfDocument::new(title, Mm(W), Mm(H), "text");
    let layer = doc.get_page(p).get_layer(l);
    let reg = Loaded::new(&doc, &REGULAR)?;
    let bold = Loaded::new(&doc, &BOLD)?;
    let ital = Loaded::new(&doc, &ITALIC)?;
    let mut w = Writer { doc, layer, y: H - MARGIN };
    for b in blocks {
        match b {
            Block::Title(t) => {
                w.para(&bold, &plain(t), 22.0, 11.0, 0.0);
                w.y -= 4.0;
            }
            Block::Heading(t) => {
                w.y -= 3.0;
                w.para(&bold, &plain(t), 14.0, 7.5, 0.0);
                w.y -= 1.0;
            }
            Block::Para(t) => w.para(&reg, &plain(t), 11.0, 5.6, 0.0),
            Block::Bullet(t) => {
                if w.y - 5.6 < MARGIN {
                    w.new_page();
                }
                let y = w.y - 5.6;
                let save = w.y;
                w.y = y;
                w.line(&reg, "•", 11.0, MARGIN + 1.0);
                w.y = save;
                w.para(&reg, &plain(t), 11.0, 5.6, 6.0);
            }
            Block::Quote(t) => w.para(&ital, &plain(t), 11.0, 5.6, 6.0),
            Block::Small(t) => w.para(&reg, &plain(t), 8.5, 4.4, 0.0),
            Block::Gap => w.y -= 3.0,
        }
    }
    let file = std::fs::File::create(path).map_err(|e| format!("Не удалось сохранить PDF: {e}"))?;
    w.doc.save(&mut std::io::BufWriter::new(file)).map_err(|e| format!("Не удалось сохранить PDF: {e}"))
}

/// Turns a Markdown note into print blocks: the first line is the title.
pub fn from_markdown(text: &str) -> Vec<Block> {
    let mut out = vec![];
    let mut para = String::new();
    let mut first = true;
    let flush = |para: &mut String, out: &mut Vec<Block>| {
        if !para.trim().is_empty() {
            out.push(Block::Para(para.trim().to_string()));
        }
        para.clear();
    };
    for line in text.lines() {
        let t = line.trim();
        if first && !t.is_empty() {
            first = false;
            out.push(Block::Title(t.trim_start_matches('#').trim().to_string()));
            continue;
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
            out.push(Block::Gap);
        } else if t.starts_with('#') {
            flush(&mut para, &mut out);
            out.push(Block::Heading(t.trim_start_matches('#').trim().to_string()));
        } else if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush(&mut para, &mut out);
            out.push(Block::Bullet(r.replace("[ ] ", "☐ ").replace("[x] ", "✓ ")));
        } else if let Some(r) = t.strip_prefix("> ") {
            flush(&mut para, &mut out);
            out.push(Block::Quote(r.to_string()));
        } else if t.starts_with("![") {
            continue; // images are left out of print
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t);
        }
    }
    flush(&mut para, &mut out);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_mixed_script_pdf() {
        let p = std::env::temp_dir().join(format!("qn-{}.pdf", std::process::id()));
        super::write(&p, "t", &super::from_markdown("Заголовок Title\n\nТекст и text, **жирно**.\n- пункт\n> цитата")).unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() > 1000);
    }
}
