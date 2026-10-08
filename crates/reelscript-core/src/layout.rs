//! Pagination to the standard US screenplay format: US Letter, Courier 12pt
//! (10 characters per inch, 6 lines per inch), 54 lines per page.
//!
//! All coordinates are in inches from the top-left corner of the page.

use std::collections::VecDeque;

use crate::fountain::{Dialogue, DialoguePart, Element, RichText, Screenplay, Span, Style, parse_inline, plain};

pub const PAGE_WIDTH: f32 = 8.5;
pub const PAGE_HEIGHT: f32 = 11.0;
pub const CHAR_WIDTH: f32 = 0.1;
pub const LINE_HEIGHT: f32 = 1.0 / 6.0;
pub const TOP_MARGIN: f32 = 1.0;
pub const LINES_PER_PAGE: usize = 54;

const ACTION_X: f32 = 1.5;
const ACTION_WIDTH: usize = 60;
const RIGHT_EDGE: f32 = 7.5;

/// Positioned text on a page. `y` is the top of the text line.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub x: f32,
    pub y: f32,
    pub spans: RichText,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    pub items: Vec<Item>,
    /// Script page number (`None` for the title page).
    pub number: Option<u32>,
}

/// Formatting choices that change pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutOptions {
    /// Number every scene 1, 2, 3… (overrides `#n#` numbers in the source).
    pub number_scenes: bool,
    /// Print scene headings in bold.
    pub bold_headings: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneInfo {
    pub heading: String,
    /// Printed scene number, if any.
    pub number: Option<String>,
    /// Index into [`Layout::pages`] where the scene starts.
    pub page: usize,
    /// 0-based source line of the heading (if the script came from Fountain).
    pub source_line: Option<usize>,
    /// Length in eighths of a page (minimum 1), the production-standard unit.
    pub eighths: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub pages: Vec<Page>,
    pub scenes: Vec<SceneInfo>,
}

impl Layout {
    /// Printed page number of the page at `index` (0 for the title page).
    pub fn page_number(&self, index: usize) -> u32 {
        self.pages.get(index).and_then(|p| p.number).unwrap_or(0)
    }

    pub fn body_pages(&self) -> usize {
        self.pages.iter().filter(|p| p.number.is_some()).count()
    }
}

/// Format eighths of a page the way production reports do: `1 3/8`, `5/8`, `2`.
pub fn format_eighths(e: u32) -> String {
    match (e / 8, e % 8) {
        (0, r) => format!("{r}/8"),
        (w, 0) => w.to_string(),
        (w, r) => format!("{w} {r}/8"),
    }
}

#[derive(Debug, Clone)]
struct Piece {
    x: f32,
    spans: RichText,
}

/// One printed line; empty = blank line.
type Row = Vec<Piece>;

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Heading,
    Action,
    Dialogue { cue: String, cue_x: f32 },
    Atomic,
    PageBreak,
}

#[derive(Debug, Clone)]
struct SceneMeta {
    heading: String,
    number: Option<String>,
    source_line: Option<usize>,
}

#[derive(Debug, Clone)]
struct Block {
    rows: Vec<Row>,
    kind: Kind,
    space_before: usize,
    scene: Option<SceneMeta>,
}

/// Dialogue column geometry.
#[derive(Clone, Copy)]
struct Col {
    cue_x: f32,
    dia_x: f32,
    dia_w: usize,
    par_x: f32,
    par_w: usize,
}

const NORMAL: Col = Col { cue_x: 3.7, dia_x: 2.5, dia_w: 35, par_x: 3.1, par_w: 26 };
const DUAL_LEFT: Col = Col { cue_x: 2.3, dia_x: 1.5, dia_w: 28, par_x: 1.8, par_w: 22 };
const DUAL_RIGHT: Col = Col { cue_x: 5.3, dia_x: 4.5, dia_w: 28, par_x: 4.8, par_w: 22 };

pub fn paginate(script: &Screenplay) -> Layout {
    paginate_with(script, &LayoutOptions::default())
}

pub fn paginate_with(script: &Screenplay, opts: &LayoutOptions) -> Layout {
    let mut layout = Layout::default();
    if !script.title_page.is_empty() {
        layout.pages.push(title_page(script));
    }
    let offset = layout.pages.len();

    let mut queue: VecDeque<Block> = build_blocks(script, opts).into();
    let mut body: Vec<Vec<Row>> = Vec::new();
    let mut cur: Vec<Row> = Vec::new();
    // Absolute line position (body page * LINES_PER_PAGE + row) where each scene starts.
    let mut scene_starts: Vec<usize> = Vec::new();

    let mut place = |cur: &mut Vec<Row>, body_len: usize, block: Block, space: usize, scenes: &mut Vec<SceneInfo>| {
        if let Some(s) = block.scene {
            scene_starts.push(body_len * LINES_PER_PAGE + cur.len() + space);
            scenes.push(SceneInfo {
                heading: s.heading,
                number: s.number,
                page: offset + body_len,
                source_line: s.source_line,
                eighths: 0,
            });
        }
        cur.extend(std::iter::repeat_with(Vec::new).take(space));
        cur.extend(block.rows);
    };

    while let Some(mut block) = queue.pop_front() {
        if block.kind == Kind::PageBreak {
            if !cur.is_empty() {
                body.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let space = if cur.is_empty() { 0 } else { block.space_before };
        let avail = LINES_PER_PAGE.saturating_sub(cur.len() + space);
        // A scene heading must be followed by at least two lines of the next block.
        let keep = match block.kind {
            Kind::Heading => queue
                .front()
                .filter(|b| b.kind != Kind::PageBreak)
                .map(|b| b.space_before + b.rows.len().min(2))
                .unwrap_or(0),
            _ => 0,
        };
        let len = block.rows.len();

        if len + keep <= avail || (cur.is_empty() && len <= avail) {
            place(&mut cur, body.len(), block, space, &mut layout.scenes);
            continue;
        }

        // Doesn't fit: try splitting across the page boundary.
        let split = match &block.kind {
            Kind::Action if avail >= 2 && len - avail >= 2 => Some(avail),
            Kind::Dialogue { .. } if avail >= 3 && len - (avail - 1) >= 1 => Some(avail - 1),
            // Oversized block on an empty page: must split somewhere.
            Kind::Dialogue { .. } if cur.is_empty() => Some(avail - 1),
            _ if cur.is_empty() => Some(avail),
            _ => None,
        };
        match split {
            Some(k) => {
                let mut rest_rows = block.rows.split_off(k);
                if let Kind::Dialogue { cue, cue_x } = &block.kind {
                    block.rows.push(vec![piece(*cue_x, "(MORE)")]);
                    rest_rows.insert(0, vec![piece(*cue_x, &format!("{cue} (CONT'D)"))]);
                }
                let rest = Block { rows: rest_rows, kind: block.kind.clone(), space_before: 0, scene: None };
                place(&mut cur, body.len(), block, space, &mut layout.scenes);
                body.push(std::mem::take(&mut cur));
                queue.push_front(rest);
            }
            None => {
                body.push(std::mem::take(&mut cur));
                queue.push_front(block);
            }
        }
    }
    if !cur.is_empty() || body.is_empty() {
        body.push(cur);
    }
    let end = (body.len() - 1) * LINES_PER_PAGE + body.last().map_or(0, |p| p.len());
    for i in 0..layout.scenes.len() {
        let stop = scene_starts.get(i + 1).copied().unwrap_or(end);
        let lines = stop.saturating_sub(scene_starts[i]);
        let e = (lines as f32 * 8.0 / LINES_PER_PAGE as f32).round() as u32;
        layout.scenes[i].eighths = e.max(1);
    }

    for (i, rows) in body.into_iter().enumerate() {
        let number = i as u32 + 1;
        let mut page = Page { items: Vec::new(), number: Some(number) };
        if number > 1 {
            let label = format!("{number}.");
            page.items.push(Item { x: right_align(&label), y: 0.5, spans: plain_spans(&label) });
        }
        for (r, row) in rows.into_iter().enumerate() {
            let y = TOP_MARGIN + r as f32 * LINE_HEIGHT;
            for p in row {
                page.items.push(Item { x: p.x, y, spans: p.spans });
            }
        }
        layout.pages.push(page);
    }
    layout
}

fn build_blocks(script: &Screenplay, opts: &LayoutOptions) -> Vec<Block> {
    let mut blocks = Vec::new();
    let block = |rows, kind| Block { rows, kind, space_before: 1, scene: None };
    let mut scene_count = 0;
    for (idx, el) in script.elements.iter().enumerate() {
        match el {
            Element::SceneHeading { text, number } => {
                scene_count += 1;
                let number = if opts.number_scenes { Some(scene_count.to_string()) } else { number.clone() };
                let upper = text.to_uppercase();
                let style = Style { bold: opts.bold_headings, ..Style::default() };
                let mut rows: Vec<Row> = wrap(&vec![Span { text: upper.clone(), style }], ACTION_WIDTH)
                    .into_iter()
                    .map(|l| vec![Piece { x: ACTION_X, spans: l }])
                    .collect();
                if let Some(num) = &number {
                    rows[0].push(piece(ACTION_X - (num.chars().count() as f32 + 2.0) * CHAR_WIDTH, num));
                    rows[0].push(piece(RIGHT_EDGE + 0.2, num));
                }
                let mut b = block(rows, Kind::Heading);
                b.scene =
                    Some(SceneMeta { heading: upper, number, source_line: script.element_lines.get(idx).copied() });
                blocks.push(b);
            }
            Element::Action(lines) => {
                let rows = lines
                    .iter()
                    .flat_map(|l| wrap(l, ACTION_WIDTH))
                    .map(|l| if l.is_empty() { vec![] } else { vec![Piece { x: ACTION_X, spans: l }] })
                    .collect();
                blocks.push(block(rows, Kind::Action));
            }
            Element::Lyrics(lines) => {
                let rows = lines
                    .iter()
                    .flat_map(|l| wrap(&italicize(l), ACTION_WIDTH))
                    .map(|l| vec![Piece { x: ACTION_X, spans: l }])
                    .collect();
                blocks.push(block(rows, Kind::Atomic));
            }
            Element::Dialogue(d) => {
                let cue = d.character.split('(').next().unwrap_or("").trim().to_string();
                blocks.push(block(dialogue_rows(d, NORMAL), Kind::Dialogue { cue, cue_x: NORMAL.cue_x }));
            }
            Element::DualDialogue(a, b) => {
                let left = dialogue_rows(a, DUAL_LEFT);
                let right = dialogue_rows(b, DUAL_RIGHT);
                let n = left.len().max(right.len());
                let mut li = left.into_iter();
                let mut ri = right.into_iter();
                let rows = (0..n)
                    .map(|_| {
                        let mut row = li.next().unwrap_or_default();
                        row.extend(ri.next().unwrap_or_default());
                        row
                    })
                    .collect();
                blocks.push(block(rows, Kind::Atomic));
            }
            Element::Transition(t) => {
                let t = t.to_uppercase();
                blocks.push(block(vec![vec![piece(right_align(&t), &t)]], Kind::Atomic));
            }
            Element::Centered(lines) => {
                let rows = lines
                    .iter()
                    .flat_map(|l| wrap(l, ACTION_WIDTH))
                    .map(|l| vec![Piece { x: center(&plain(&l)), spans: l }])
                    .collect();
                blocks.push(block(rows, Kind::Atomic));
            }
            Element::PageBreak => blocks.push(block(vec![], Kind::PageBreak)),
            Element::Section { .. } | Element::Synopsis(_) => {}
        }
    }
    blocks
}

fn dialogue_rows(d: &Dialogue, col: Col) -> Vec<Row> {
    let mut rows = vec![vec![piece(col.cue_x, &d.character)]];
    for part in &d.parts {
        let (x, lines) = match part {
            DialoguePart::Parenthetical(p) => (col.par_x, wrap(&plain_spans(p), col.par_w)),
            DialoguePart::Line(l) => (col.dia_x, wrap(l, col.dia_w)),
        };
        rows.extend(lines.into_iter().map(|l| if l.is_empty() { vec![] } else { vec![Piece { x, spans: l }] }));
    }
    rows
}

fn title_page(script: &Screenplay) -> Page {
    let mut items = Vec::new();
    let get = |k: &str| script.title_value(k).cloned().unwrap_or_default();

    // Centered block, starting about a third of the way down.
    let mut row = 18usize;
    let groups = [get("title"), get("credit"), [get("author"), get("authors")].concat(), get("source")];
    for group in groups.iter().filter(|g| !g.is_empty()) {
        for line in group {
            let spans = parse_inline(line);
            items.push(Item { x: center(&plain(&spans)), y: row_y(row), spans });
            row += 1;
        }
        row += 1;
    }

    // Bottom-left: contact details. Bottom-right: dates/revision.
    let left = [get("contact"), get("copyright"), get("notes")].concat();
    let right = [get("draft date"), get("date"), get("revision")].concat();
    for (lines, right_side) in [(left, false), (right, true)] {
        let start = LINES_PER_PAGE.saturating_sub(lines.len());
        for (i, line) in lines.iter().enumerate() {
            let spans = parse_inline(line);
            let x = if right_side { right_align(&plain(&spans)) } else { ACTION_X };
            items.push(Item { x, y: row_y(start + i), spans });
        }
    }
    Page { items, number: None }
}

fn row_y(row: usize) -> f32 {
    TOP_MARGIN + row as f32 * LINE_HEIGHT
}

fn piece(x: f32, text: &str) -> Piece {
    Piece { x, spans: plain_spans(text) }
}

fn plain_spans(text: &str) -> RichText {
    if text.is_empty() {
        return vec![];
    }
    vec![Span { text: text.to_string(), style: Style::default() }]
}

fn italicize(rt: &RichText) -> RichText {
    rt.iter().map(|s| Span { text: s.text.clone(), style: Style { italic: true, ..s.style } }).collect()
}

fn right_align(text: &str) -> f32 {
    RIGHT_EDGE - text.chars().count() as f32 * CHAR_WIDTH
}

fn center(text: &str) -> f32 {
    (PAGE_WIDTH / 2.0 - text.chars().count() as f32 * CHAR_WIDTH / 2.0).max(ACTION_X)
}

/// Greedy word wrap of styled text to `width` characters. Words longer than the
/// width are hard-broken. Always returns at least one (possibly empty) line.
pub fn wrap(rt: &RichText, width: usize) -> Vec<RichText> {
    let chars: Vec<(char, Style)> = rt.iter().flat_map(|s| s.text.chars().map(move |c| (c, s.style))).collect();
    let len = chars.len();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < len {
        if len - start <= width {
            lines.push(&chars[start..]);
            break;
        }
        let limit = start + width;
        match (start + 1..=limit).rev().find(|&j| chars[j].0 == ' ') {
            Some(j) => {
                lines.push(&chars[start..j]);
                start = j;
                while start < len && chars[start].0 == ' ' {
                    start += 1;
                }
            }
            None => {
                lines.push(&chars[start..limit]);
                start = limit;
            }
        }
    }
    if lines.is_empty() {
        return vec![vec![]];
    }
    lines.into_iter().map(|l| regroup(trim_end(l))).collect()
}

fn trim_end(mut l: &[(char, Style)]) -> &[(char, Style)] {
    while let Some(((' ', _), rest)) = l.split_last() {
        l = rest;
    }
    l
}

fn regroup(chars: &[(char, Style)]) -> RichText {
    let mut out: RichText = Vec::new();
    for &(c, style) in chars {
        match out.last_mut() {
            Some(s) if s.style == style => s.text.push(c),
            _ => out.push(Span { text: c.to_string(), style }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fountain::parse;

    fn texts(page: &Page) -> Vec<String> {
        page.items.iter().map(|i| plain(&i.spans)).collect()
    }

    #[test]
    fn wrap_words() {
        let w = wrap(&plain_spans("the quick brown fox jumps"), 10);
        let w: Vec<String> = w.iter().map(plain).collect();
        assert_eq!(w, vec!["the quick", "brown fox", "jumps"]);
        let w = wrap(&plain_spans("abcdefghijkl"), 5);
        assert_eq!(w.iter().map(plain).collect::<Vec<_>>(), vec!["abcde", "fghij", "kl"]);
    }

    #[test]
    fn standard_positions() {
        let l = paginate(&parse("INT. HOUSE - DAY\n\nShe waits.\n\nBOB\n(softly)\nHi.\n\nCUT TO:\n"));
        assert_eq!(l.pages.len(), 1);
        let items = &l.pages[0].items;
        let find = |t: &str| items.iter().find(|i| plain(&i.spans) == t).unwrap();
        assert_eq!(find("INT. HOUSE - DAY").x, 1.5);
        assert_eq!(find("BOB").x, 3.7);
        assert_eq!(find("(softly)").x, 3.1);
        assert_eq!(find("Hi.").x, 2.5);
        assert!((find("CUT TO:").x - 6.8).abs() < 1e-4);
        assert_eq!(l.scenes.len(), 1);
        assert_eq!(
            (l.scenes[0].heading.as_str(), l.scenes[0].page, l.scenes[0].source_line),
            ("INT. HOUSE - DAY", 0, Some(0))
        );
    }

    #[test]
    fn long_dialogue_gets_more_and_contd() {
        let lines: String = (0..80).map(|i| format!("Line {i}.\n")).collect();
        let l = paginate(&parse(&format!("\nBOB (V.O.)\n{lines}")));
        assert_eq!(l.pages.len(), 2);
        let p1 = texts(&l.pages[0]);
        assert_eq!(p1.last().unwrap(), "(MORE)");
        assert_eq!(p1.len(), LINES_PER_PAGE);
        let p2 = texts(&l.pages[1]);
        assert_eq!(p2[0], "2.");
        assert_eq!(p2[1], "BOB (CONT'D)");
    }

    #[test]
    fn heading_not_orphaned() {
        // Fill the page so only the heading would fit at the bottom.
        let filler: String = (0..52).map(|_| "x\n").collect();
        let l = paginate(&parse(&format!("{filler}\nINT. HOUSE - DAY\n\nAction.\n")));
        assert_eq!(l.pages.len(), 2);
        assert_eq!(texts(&l.pages[1])[1], "INT. HOUSE - DAY");
        assert_eq!(l.scenes[0].page, 1);
    }

    #[test]
    fn page_break_and_title() {
        let l = paginate(&parse("Title: Test\nAuthor: Me\n\nOne.\n\n===\n\nTwo.\n"));
        assert_eq!(l.pages.len(), 3);
        assert_eq!(l.pages[0].number, None);
        assert!(texts(&l.pages[0]).contains(&"Test".to_string()));
        assert_eq!(l.pages[2].number, Some(2));
    }

    #[test]
    fn dual_dialogue_side_by_side() {
        let l = paginate(&parse("\nBRICK\nScrew it.\n\nSTEEL ^\nScrew it.\n"));
        let items = &l.pages[0].items;
        let ys: Vec<f32> =
            items.iter().filter(|i| ["BRICK", "STEEL"].contains(&plain(&i.spans).as_str())).map(|i| i.y).collect();
        assert_eq!(ys.len(), 2);
        assert_eq!(ys[0], ys[1]);
    }

    #[test]
    fn options_and_eighths() {
        let filler: String = (0..25).map(|_| "x\n").collect();
        let src = format!("INT. A - DAY #9#\n\n{filler}\nEXT. B - NIGHT\n\nShort.\n");
        let opts = LayoutOptions { number_scenes: true, bold_headings: true };
        let l = paginate_with(&parse(&src), &opts);
        let nums: Vec<_> = l.scenes.iter().map(|s| s.number.clone().unwrap()).collect();
        assert_eq!(nums, vec!["1", "2"]);
        // Scene A: heading + blank + 25 lines + blank = 28 lines ≈ 4/8.
        assert_eq!(l.scenes[0].eighths, 4);
        assert_eq!(l.scenes[1].eighths, 1);
        let heading = l.pages[0].items.iter().find(|i| plain(&i.spans) == "INT. A - DAY").unwrap();
        assert!(heading.spans[0].style.bold);
        assert_eq!(format_eighths(11), "1 3/8");
        assert_eq!(format_eighths(5), "5/8");
        assert_eq!(format_eighths(16), "2");
    }
}
