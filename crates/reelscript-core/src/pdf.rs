//! Minimal dependency-free PDF writer using the 14 standard Courier fonts
//! (no font embedding required; every PDF viewer ships them).

use std::fmt::Write as _;

use crate::layout::{CHAR_WIDTH, PAGE_HEIGHT, PAGE_WIDTH, Page};

const PT: f32 = 72.0;
const FONT_SIZE: f32 = 12.0;
/// Distance from the top of a text line to its baseline, in points.
const BASELINE: f32 = 9.5;

/// Page and text colours (RGB, 0–255).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub paper: [u8; 3],
    pub ink: [u8; 3],
}

impl Theme {
    pub const PAPER: Theme = Theme { paper: [255, 255, 255], ink: [0, 0, 0] };
    pub const NIGHT: Theme = Theme { paper: [38, 40, 46], ink: [222, 222, 214] };
    pub const MIDNIGHT: Theme = Theme { paper: [0, 0, 0], ink: [120, 200, 255] };

    pub fn by_name(name: &str) -> Option<Theme> {
        match name.to_ascii_lowercase().as_str() {
            "paper" | "light" => Some(Self::PAPER),
            "night" | "dark" => Some(Self::NIGHT),
            "midnight" => Some(Self::MIDNIGHT),
            _ => None,
        }
    }

    /// Parse `#rrggbb` (or `rrggbb`).
    pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
        let h = s.trim().trim_start_matches('#');
        if h.len() != 6 {
            return None;
        }
        let b = |i| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some([b(0)?, b(2)?, b(4)?])
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::PAPER
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PdfOptions {
    /// Document title metadata.
    pub title: String,
    /// Large diagonal text behind every page, e.g. a recipient's name or "DRAFT".
    pub watermark: Option<String>,
    pub theme: Theme,
}

/// Render laid-out pages to PDF bytes with default options.
pub fn render(pages: &[Page], title: &str) -> Vec<u8> {
    render_with(pages, &PdfOptions { title: title.to_string(), ..Default::default() })
}

/// Render laid-out pages to PDF bytes.
pub fn render_with(pages: &[Page], opts: &PdfOptions) -> Vec<u8> {
    let title = opts.title.as_str();
    let mut w = Writer::default();
    w.buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    // Fixed object ids: 1 catalog, 2 pages, 3..=6 Courier fonts, 7 info,
    // 8 Helvetica-Bold (watermark), 9 transparency state; pages follow.
    let fonts = ["Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique", "Helvetica-Bold"];
    let first_page_id = 10;
    let kids: String = (0..pages.len()).map(|i| format!("{} 0 R ", first_page_id + i * 2)).collect();

    w.obj(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    w.obj(2, format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).as_bytes());
    for (i, f) in fonts.iter().enumerate() {
        w.obj(
            if i < 4 { 3 + i } else { 8 },
            format!("<< /Type /Font /Subtype /Type1 /BaseFont /{f} /Encoding /WinAnsiEncoding >>").as_bytes(),
        );
    }
    let mut info = Vec::from(&b"<< /Producer (ReelScript) /Title "[..]);
    info.extend(pdf_string(title));
    info.extend_from_slice(b" >>");
    w.obj(7, &info);
    w.obj(9, b"<< /Type /ExtGState /ca 0.12 /CA 0.12 >>");

    for (i, page) in pages.iter().enumerate() {
        let page_id = first_page_id + i * 2;
        let content_id = page_id + 1;
        w.obj(
            page_id,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {content_id} 0 R \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R /F4 6 0 R /F5 8 0 R >> \
                 /ExtGState << /GS1 9 0 R >> >> >>",
                PAGE_WIDTH * PT,
                PAGE_HEIGHT * PT
            )
            .as_bytes(),
        );
        let content = page_content(page, opts);
        let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        stream.extend(content);
        stream.extend_from_slice(b"\nendstream");
        w.obj(content_id, &stream);
    }

    w.finish(first_page_id + pages.len() * 2)
}

fn rgb(c: [u8; 3]) -> String {
    format!("{:.3} {:.3} {:.3}", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0)
}

fn page_content(page: &Page, opts: &PdfOptions) -> Vec<u8> {
    let mut out = Vec::new();
    let (w, h) = (PAGE_WIDTH * PT, PAGE_HEIGHT * PT);
    if opts.theme.paper != [255, 255, 255] {
        out.extend(format!("{} rg 0 0 {w} {h} re f\n", rgb(opts.theme.paper)).into_bytes());
    }
    let ink = rgb(opts.theme.ink);
    if let Some(mark) = opts.watermark.as_deref().filter(|m| !m.trim().is_empty()) {
        // Helvetica-Bold averages ~0.65em per uppercase glyph.
        let len = mark.chars().count().max(1) as f32;
        let size = (820.0 / (len * 0.68)).min(130.0);
        let text_w = len * size * 0.68;
        let (c, s) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let x = cx - text_w / 2.0 * c + size * 0.35 * s;
        let y = cy - text_w / 2.0 * s - size * 0.35 * c;
        out.extend(
            format!("q /GS1 gs {ink} rg BT /F5 {size:.1} Tf {c:.4} {s:.4} {:.4} {c:.4} {x:.2} {y:.2} Tm ", -s)
                .into_bytes(),
        );
        out.extend(pdf_string(mark));
        out.extend_from_slice(b" Tj ET Q\n");
    }
    out.extend(format!("{ink} rg {ink} RG\n").into_bytes());
    let mut lines = String::new();
    for item in &page.items {
        let mut x = item.x * PT;
        let y = (PAGE_HEIGHT - item.y) * PT - BASELINE;
        for span in &item.spans {
            let font = match (span.style.bold, span.style.italic) {
                (false, false) => "F1",
                (true, false) => "F2",
                (false, true) => "F3",
                (true, true) => "F4",
            };
            out.extend(format!("BT /{font} {FONT_SIZE} Tf {x:.2} {y:.2} Td ").into_bytes());
            out.extend(pdf_string(&span.text));
            out.extend_from_slice(b" Tj ET\n");
            let width = span.text.chars().count() as f32 * CHAR_WIDTH * PT;
            if span.style.underline {
                let _ = writeln!(lines, "{x:.2} {:.2} m {:.2} {:.2} l S", y - 1.5, x + width, y - 1.5);
            }
            x += width;
        }
    }
    if !lines.is_empty() {
        out.extend(format!("0.6 w\n{lines}").into_bytes());
    }
    out
}

/// Encode text as a PDF literal string in WinAnsi (CP1252). Unmappable chars become `?`.
fn pdf_string(text: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in text.chars() {
        let b = winansi(c).unwrap_or(b'?');
        match b {
            b'(' | b')' | b'\\' => out.extend_from_slice(&[b'\\', b]),
            0x20..=0x7E => out.push(b),
            _ => out.extend(format!("\\{b:03o}").into_bytes()),
        }
    }
    out.push(b')');
    out
}

fn winansi(c: char) -> Option<u8> {
    let u = c as u32;
    Some(match u {
        0x20..=0x7E | 0xA0..=0xFF => u as u8,
        _ => match c {
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8A,
            '‹' => 0x8B,
            'Œ' => 0x8C,
            'Ž' => 0x8E,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9A,
            '›' => 0x9B,
            'œ' => 0x9C,
            'ž' => 0x9E,
            'Ÿ' => 0x9F,
            _ => return None,
        },
    })
}

#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
    offsets: Vec<(usize, usize)>,
}

impl Writer {
    fn obj(&mut self, id: usize, body: &[u8]) {
        self.offsets.push((id, self.buf.len()));
        self.buf.extend(format!("{id} 0 obj\n").into_bytes());
        self.buf.extend_from_slice(body);
        self.buf.extend_from_slice(b"\nendobj\n");
    }

    fn finish(mut self, size: usize) -> Vec<u8> {
        self.offsets.sort();
        let xref = self.buf.len();
        let mut s = format!("xref\n0 {size}\n0000000000 65535 f \n");
        for (_, off) in &self.offsets {
            let _ = writeln!(s, "{off:010} 00000 n ");
        }
        let _ = write!(s, "trailer\n<< /Size {size} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n");
        self.buf.extend(s.into_bytes());
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_encodes() {
        assert_eq!(pdf_string("a(b)\\"), b"(a\\(b\\)\\\\)".to_vec());
        assert_eq!(pdf_string("’é"), b"(\\222\\351)".to_vec());
        assert_eq!(pdf_string("日"), b"(?)".to_vec());
    }

    #[test]
    fn xref_offsets_point_at_objects() {
        let pdf = crate::fountain_to_pdf("Title: T\n\nINT. A - DAY\n\nHi *there*.\n");
        let find = |needle: &[u8]| pdf.windows(needle.len()).rposition(|w| w == needle).unwrap();
        let after = &pdf[find(b"startxref\n") + 10..];
        let xref_pos: usize = String::from_utf8_lossy(after).lines().next().unwrap().parse().unwrap();
        let xref = String::from_utf8_lossy(&pdf[xref_pos..]).to_string();
        let entries: Vec<&str> = xref.lines().skip(3).take_while(|l| l.ends_with(" n ")).collect();
        assert_eq!(entries.len(), 9 + 2 * 2); // fixed objects + title page + 1 body page
        for (id, line) in entries.iter().enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(pdf[off..].starts_with(format!("{} 0 obj", id + 1).as_bytes()), "object {}", id + 1);
        }
    }

    #[test]
    fn watermark_and_theme() {
        let script = crate::parse("INT. A - DAY\n\nHi.\n");
        let pages = crate::layout::paginate(&script).pages;
        let opts = PdfOptions { title: "T".into(), watermark: Some("DRAFT".into()), theme: Theme::MIDNIGHT };
        let pdf = String::from_utf8_lossy(&render_with(&pages, &opts)).to_string();
        assert!(pdf.contains("/F5") && pdf.contains("(DRAFT) Tj"));
        assert!(pdf.contains("0.000 0.000 0.000 rg 0 0 612 792 re f"));
        assert_eq!(Theme::parse_hex("#1a2B3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(Theme::parse_hex("zz"), None);
    }
}
