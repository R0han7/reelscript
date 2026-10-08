//! Fountain parser. Follows the spec at <https://fountain.io/syntax>.
//!
//! Supported: title page, scene headings (natural + forced `.`, scene numbers `#1#`),
//! action (forced `!`), characters (forced `@`, extensions, dual dialogue `^`),
//! parentheticals, dialogue, transitions (natural + forced `>`), centered `>text<`,
//! lyrics `~`, page breaks `===`, sections `#`, synopses `=`, notes `[[ ]]`,
//! boneyard `/* */`, and emphasis `*italic*`, `**bold**`, `***both***`, `_underline_`.

/// Inline text style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// A run of text sharing one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// A line of styled text.
pub type RichText = Vec<Span>;

/// Plain text of a rich line.
pub fn plain(rt: &RichText) -> String {
    rt.iter().map(|s| s.text.as_str()).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum DialoguePart {
    Parenthetical(String),
    Line(RichText),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dialogue {
    /// Character cue as written, including any extension, e.g. `MARY (V.O.)`.
    pub character: String,
    pub parts: Vec<DialoguePart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    SceneHeading {
        text: String,
        number: Option<String>,
    },
    /// One entry per source line; empty entries are intentional blank lines.
    Action(Vec<RichText>),
    Dialogue(Dialogue),
    DualDialogue(Dialogue, Dialogue),
    Transition(String),
    Centered(Vec<RichText>),
    Lyrics(Vec<RichText>),
    PageBreak,
    /// Outline only; not printed.
    Section {
        level: usize,
        text: String,
    },
    /// Outline only; not printed.
    Synopsis(String),
}

/// What a source line is, for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Blank,
    TitleKey,
    TitleValue,
    SceneHeading,
    Action,
    Character,
    Parenthetical,
    Dialogue,
    Transition,
    Centered,
    Lyrics,
    PageBreak,
    Section,
    Synopsis,
    /// Note-only or boneyard line.
    Comment,
}

/// A `[[note]]` with the 0-based source line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Screenplay {
    /// Title page entries in source order: (lower-cased key, value lines).
    pub title_page: Vec<(String, Vec<String>)>,
    pub elements: Vec<Element>,
    /// 0-based source line where each element starts (parallel to `elements`).
    /// Empty for screenplays not parsed from Fountain.
    pub element_lines: Vec<usize>,
    pub notes: Vec<Note>,
    /// Kind of every source line.
    pub line_kinds: Vec<LineKind>,
}

impl Screenplay {
    pub fn title_value(&self, key: &str) -> Option<&Vec<String>> {
        let key = key.to_ascii_lowercase();
        self.title_page.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// Scene headings in order, for outlines/navigators.
    pub fn scenes(&self) -> impl Iterator<Item = &str> {
        self.elements.iter().filter_map(|e| match e {
            Element::SceneHeading { text, .. } => Some(text.as_str()),
            _ => None,
        })
    }
}

const TITLE_KEYS: &[&str] = &[
    "title",
    "credit",
    "author",
    "authors",
    "source",
    "draft date",
    "date",
    "contact",
    "copyright",
    "notes",
    "revision",
];

const SCENE_PREFIXES: &[&str] = &["INT./EXT", "INT/EXT", "I/E", "INT", "EXT", "EST"];

/// Parse a Fountain document.
pub fn parse(source: &str) -> Screenplay {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    let stripped = strip_notes_and_boneyard(&source);
    let lines: Vec<&str> = stripped.text.split('\n').collect();
    let orig = &stripped.orig_line;

    let mut script = Screenplay {
        notes: stripped.notes,
        line_kinds: vec![LineKind::Comment; source.split('\n').count()],
        ..Default::default()
    };
    let body_start = parse_title_page(&lines, &mut script.title_page);

    let mut kinds = vec![LineKind::Blank; lines.len()];
    for (i, line) in lines.iter().enumerate().take(body_start) {
        if !line.trim().is_empty() {
            let indented = line.starts_with("   ") || line.starts_with('\t');
            kinds[i] = if !indented && split_key(line).is_some() { LineKind::TitleKey } else { LineKind::TitleValue };
        }
    }
    let (elements, starts) = parse_body(&lines[body_start..], &mut kinds[body_start..]);
    script.elements = elements;
    script.element_lines = starts.into_iter().map(|i| orig[body_start + i]).collect();
    for (i, k) in kinds.into_iter().enumerate() {
        script.line_kinds[orig[i]] = k;
    }
    script
}

struct Stripped {
    text: String,
    /// Original 0-based line number of each line in `text`.
    orig_line: Vec<usize>,
    notes: Vec<Note>,
}

/// Remove `/* boneyard */` and `[[notes]]`. Lines left empty *only* because of a
/// removal are dropped, so a note on its own line doesn't split a paragraph.
fn strip_notes_and_boneyard(src: &str) -> Stripped {
    const MARK: char = '\u{E000}';
    /// Stands in for a newline that was removed along with its comment.
    const GONE_NL: char = '\u{E001}';
    let mut out = String::with_capacity(src.len());
    let mut notes = Vec::new();
    let mut line = 0;
    let mut rest = src;
    while !rest.is_empty() {
        let next_bone = rest.find("/*");
        let next_note = rest.find("[[");
        let (start, open, close) = match (next_bone, next_note) {
            (Some(b), Some(n)) if b < n => (b, "/*", "*/"),
            (Some(b), None) => (b, "/*", "*/"),
            (_, Some(n)) => (n, "[[", "]]"),
            (None, None) => {
                out.push_str(rest);
                break;
            }
        };
        out.push_str(&rest[..start]);
        line += rest[..start].matches('\n').count();
        let after = &rest[start + open.len()..];
        match after.find(close) {
            Some(end) => {
                let inner = &after[..end];
                if open == "[[" {
                    notes.push(Note { line, text: inner.trim().to_string() });
                }
                let gone = inner.matches('\n').count();
                line += gone;
                out.push(MARK);
                out.extend(std::iter::repeat_n(GONE_NL, gone));
                rest = &after[end + close.len()..];
            }
            None => {
                // Unclosed: treat literally.
                out.push_str(open);
                rest = after;
            }
        }
    }
    let mut text = Vec::new();
    let mut orig_line = Vec::new();
    let mut n = 0;
    for l in out.split('\n') {
        let this = n;
        n += 1 + l.matches(GONE_NL).count();
        if l.contains(MARK) {
            let kept: String = l.chars().filter(|&c| c != MARK && c != GONE_NL).collect();
            if !kept.trim().is_empty() {
                text.push(kept);
                orig_line.push(this);
            }
        } else {
            text.push(l.to_string());
            orig_line.push(this);
        }
    }
    Stripped { text: text.join("\n"), orig_line, notes }
}

fn parse_title_page(lines: &[&str], out: &mut Vec<(String, Vec<String>)>) -> usize {
    // Only treat the start as a title page if the first key is a known one,
    // so a script opening with "FADE IN:" isn't swallowed.
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let Some(first) = first else { return 0 };
    match split_key(lines[first]) {
        Some((k, _)) if TITLE_KEYS.contains(&k.as_str()) => {}
        _ => return 0,
    }

    let mut i = first;
    while i < lines.len() && !lines[i].trim().is_empty() {
        let line = lines[i];
        let indented = line.starts_with("   ") || line.starts_with('\t');
        if !indented && let Some((key, value)) = split_key(line) {
            let mut values = Vec::new();
            if !value.is_empty() {
                values.push(value);
            }
            out.push((key, values));
        } else if let Some((_, values)) = out.last_mut() {
            values.push(line.trim().to_string());
        }
        i += 1;
    }
    i
}

fn split_key(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_alphabetic() || c == ' ') {
        return None;
    }
    Some((key.to_ascii_lowercase(), value.trim().to_string()))
}

fn is_blank(line: &str) -> bool {
    // A line of exactly two (or more) spaces is an intentional blank inside a block.
    line.trim().is_empty() && line.len() < 2
}

fn is_empty_line(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_upper(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic()) && !s.chars().any(|c| c.is_lowercase())
}

fn scene_heading(trimmed: &str) -> Option<(String, Option<String>)> {
    let text = if let Some(forced) = trimmed.strip_prefix('.') {
        if forced.starts_with('.') {
            return None;
        }
        forced.trim().to_string()
    } else {
        let upper = trimmed.to_uppercase();
        let ok = SCENE_PREFIXES
            .iter()
            .any(|p| upper.starts_with(p) && matches!(upper[p.len()..].chars().next(), Some('.') | Some(' ')));
        if !ok {
            return None;
        }
        trimmed.to_string()
    };
    // Optional trailing scene number: "INT. HOUSE - DAY #1A#"
    if text.ends_with('#')
        && let Some(open) = text[..text.len() - 1].rfind('#')
    {
        let number = text[open + 1..text.len() - 1].trim().to_string();
        let heading = text[..open].trim().to_string();
        if !number.is_empty() && !number.contains(char::is_whitespace) {
            return Some((heading, Some(number)));
        }
    }
    Some((text, None))
}

fn parse_body(lines: &[&str], kinds: &mut [LineKind]) -> (Vec<Element>, Vec<usize>) {
    let mut elements: Vec<Element> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let n = lines.len();
    let mut i = 0;

    while i < n {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        let start = i;
        let prev_blank = i == 0 || is_empty_line(lines[i - 1]);
        let next_blank = i + 1 >= n || is_empty_line(lines[i + 1]);
        let mut push = |el: Element, elements: &mut Vec<Element>| {
            elements.push(el);
            starts.push(start);
        };

        // Page break
        if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '=') {
            kinds[i] = LineKind::PageBreak;
            push(Element::PageBreak, &mut elements);
            i += 1;
            continue;
        }
        // Section
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            kinds[i] = LineKind::Section;
            push(Element::Section { level, text: trimmed[level..].trim().to_string() }, &mut elements);
            i += 1;
            continue;
        }
        // Synopsis
        if let Some(s) = trimmed.strip_prefix('=') {
            kinds[i] = LineKind::Synopsis;
            push(Element::Synopsis(s.trim().to_string()), &mut elements);
            i += 1;
            continue;
        }
        // Centered (merge consecutive lines)
        if is_centered(trimmed) {
            let mut out = Vec::new();
            while i < n && is_centered(lines[i].trim()) {
                let t = lines[i].trim();
                out.push(parse_inline(t[1..t.len() - 1].trim()));
                kinds[i] = LineKind::Centered;
                i += 1;
            }
            push(Element::Centered(out), &mut elements);
            continue;
        }
        // Forced transition
        if let Some(t) = trimmed.strip_prefix('>') {
            kinds[i] = LineKind::Transition;
            push(Element::Transition(t.trim().to_string()), &mut elements);
            i += 1;
            continue;
        }
        // Lyrics (merge consecutive lines)
        if trimmed.starts_with('~') {
            let mut out = Vec::new();
            while i < n && lines[i].trim().starts_with('~') {
                out.push(parse_inline(lines[i].trim()[1..].trim()));
                kinds[i] = LineKind::Lyrics;
                i += 1;
            }
            push(Element::Lyrics(out), &mut elements);
            continue;
        }
        // Scene heading
        if prev_blank
            && !trimmed.starts_with('!')
            && let Some((text, number)) = scene_heading(trimmed)
        {
            kinds[i] = LineKind::SceneHeading;
            push(Element::SceneHeading { text, number }, &mut elements);
            i += 1;
            continue;
        }
        // Natural transition
        if prev_blank && next_blank && is_upper(trimmed) && trimmed.ends_with("TO:") {
            kinds[i] = LineKind::Transition;
            push(Element::Transition(trimmed.to_string()), &mut elements);
            i += 1;
            continue;
        }
        // Character + dialogue
        if let Some((name, dual)) = character_cue(trimmed, prev_blank, next_blank) {
            kinds[i] = LineKind::Character;
            i += 1;
            let mut parts = Vec::new();
            while i < n && !is_blank(lines[i]) {
                let t = lines[i].trim();
                if t.starts_with('(') {
                    kinds[i] = LineKind::Parenthetical;
                    parts.push(DialoguePart::Parenthetical(t.to_string()));
                } else {
                    kinds[i] = LineKind::Dialogue;
                    parts.push(DialoguePart::Line(parse_inline(t)));
                }
                i += 1;
            }
            let d = Dialogue { character: name, parts };
            if dual && let Some(Element::Dialogue(_)) = elements.last() {
                let Some(Element::Dialogue(left)) = elements.pop() else { unreachable!() };
                elements.push(Element::DualDialogue(left, d));
            } else {
                push(Element::Dialogue(d), &mut elements);
            }
            continue;
        }
        // Action (default). Gather until a true blank line.
        let mut out = Vec::new();
        while i < n && !is_blank(lines[i]) {
            let l = lines[i].trim_end();
            let l = l.strip_prefix('!').unwrap_or(l);
            let l = l.replace('\t', "    ");
            out.push(parse_inline(&l));
            kinds[i] = LineKind::Action;
            i += 1;
        }
        push(Element::Action(out), &mut elements);
    }
    (elements, starts)
}

fn is_centered(t: &str) -> bool {
    t.len() >= 2 && t.starts_with('>') && t.ends_with('<')
}

fn character_cue(trimmed: &str, prev_blank: bool, next_blank: bool) -> Option<(String, bool)> {
    let (forced, body) = match trimmed.strip_prefix('@') {
        Some(b) => (true, b.trim()),
        None => (false, trimmed),
    };
    if next_blank || (!forced && !prev_blank) || trimmed.starts_with('!') {
        return None;
    }
    let (body, dual) = match body.strip_suffix('^') {
        Some(b) => (b.trim_end(), true),
        None => (body, false),
    };
    let name_part = body.split('(').next().unwrap_or("").trim();
    if name_part.is_empty() {
        return None;
    }
    if forced || is_upper(name_part) { Some((body.to_string(), dual)) } else { None }
}

/// Parse inline emphasis. Markers only open if a matching closer exists later on the
/// line, so stray `*` or `_` print literally. `\*` and `\_` escape.
pub fn parse_inline(s: &str) -> RichText {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut style = Style::default();
    let mut spans: RichText = Vec::new();
    let mut buf = String::new();

    let flush = |buf: &mut String, spans: &mut RichText, style: Style| {
        if buf.is_empty() {
            return;
        }
        match spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(buf),
            _ => spans.push(Span { text: std::mem::take(buf), style }),
        }
        buf.clear();
    };
    // Does an unescaped `marker` appear at or after `from`?
    let rest_has = |from: usize, marker: &str| -> bool {
        let mut rest = String::new();
        let mut j = from;
        while j < n {
            if chars[j] == '\\' && j + 1 < n {
                rest.push('\u{E000}');
                j += 2;
            } else {
                rest.push(chars[j]);
                j += 1;
            }
        }
        rest.contains(marker)
    };

    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\\' && i + 1 < n && matches!(chars[i + 1], '*' | '_' | '\\') {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '*' {
            let k = chars[i..].iter().take_while(|&&c| c == '*').count();
            if k <= 3 {
                let marker = "*".repeat(k);
                let (b, it) = match k {
                    3 => (true, true),
                    2 => (true, false),
                    _ => (false, true),
                };
                let closing = (!b || style.bold) && (!it || style.italic);
                if closing || rest_has(i + k, &marker) {
                    flush(&mut buf, &mut spans, style);
                    if b {
                        style.bold = !closing;
                    }
                    if it {
                        style.italic = !closing;
                    }
                    i += k;
                    continue;
                }
            }
            for _ in 0..k {
                buf.push('*');
            }
            i += k;
            continue;
        }
        if c == '_' && (style.underline || rest_has(i + 1, "_")) {
            flush(&mut buf, &mut spans, style);
            style.underline = !style.underline;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut spans, style);
    spans
}

/// Serialize a screenplay back to Fountain. Elements that would be misread are
/// forced with `!`, `.`, `@` or `>`, so `parse(&write(s))` reproduces `s.elements`.
pub fn write(script: &Screenplay) -> String {
    let mut out = String::new();
    for (key, values) in &script.title_page {
        let key: String = key
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        match values.as_slice() {
            [one] => out.push_str(&format!("{key}: {one}\n")),
            many => {
                out.push_str(&format!("{key}:\n"));
                for v in many {
                    out.push_str(&format!("    {v}\n"));
                }
            }
        }
    }
    if !script.title_page.is_empty() {
        out.push('\n');
    }

    let mut blocks: Vec<String> = Vec::new();
    for el in &script.elements {
        let block = match el {
            Element::SceneHeading { text, number } => {
                let mut s = match scene_heading(text) {
                    Some((t, None)) if t == *text => text.clone(),
                    _ => format!(".{text}"),
                };
                if let Some(n) = number {
                    s.push_str(&format!(" #{n}#"));
                }
                s
            }
            Element::Action(lines) => {
                let mut ls: Vec<String> =
                    lines.iter().map(|l| if l.is_empty() { "  ".into() } else { write_inline(l) }).collect();
                if let Some(first) = ls.first_mut()
                    && action_needs_force(first, lines.len() > 1)
                {
                    first.insert(0, '!');
                }
                ls.join("\n")
            }
            Element::Dialogue(d) => write_dialogue(d, false),
            Element::DualDialogue(a, b) => format!("{}\n\n{}", write_dialogue(a, false), write_dialogue(b, true)),
            Element::Transition(t) => {
                if is_upper(t) && t.ends_with("TO:") {
                    t.clone()
                } else {
                    format!("> {t}")
                }
            }
            Element::Centered(lines) => {
                lines.iter().map(|l| format!("> {} <", write_inline(l))).collect::<Vec<_>>().join("\n")
            }
            Element::Lyrics(lines) => {
                lines.iter().map(|l| format!("~{}", write_inline(l))).collect::<Vec<_>>().join("\n")
            }
            Element::PageBreak => "===".into(),
            Element::Section { level, text } => format!("{} {text}", "#".repeat(*level)),
            Element::Synopsis(s) => format!("= {s}"),
        };
        blocks.push(block);
    }
    out.push_str(&blocks.join("\n\n"));
    out.push('\n');
    out
}

fn write_dialogue(d: &Dialogue, dual: bool) -> String {
    let name = d.character.split('(').next().unwrap_or("").trim();
    let mut s = if is_upper(name) && scene_heading(&d.character).is_none() {
        d.character.clone()
    } else {
        format!("@{}", d.character)
    };
    if dual {
        s.push_str(" ^");
    }
    for part in &d.parts {
        s.push('\n');
        match part {
            DialoguePart::Parenthetical(p) => s.push_str(p),
            DialoguePart::Line(l) if l.is_empty() => s.push_str("  "),
            DialoguePart::Line(l) => s.push_str(&write_inline(l)),
        }
    }
    s
}

fn action_needs_force(first: &str, multi_line: bool) -> bool {
    let t = first.trim();
    let special = ['#', '=', '~', '>', '.', '@', '!', '['];
    t.starts_with(special) || scene_heading(t).is_some() || (is_upper(t) && (multi_line || t.ends_with("TO:")))
}

/// Inverse of [`parse_inline`].
pub fn write_inline(rt: &RichText) -> String {
    let mut out = String::new();
    let mut cur = Style::default();
    let close = |out: &mut String, cur: &mut Style, to: Style| {
        if cur.underline && !to.underline {
            out.push('_');
            cur.underline = false;
        }
        match (cur.bold && !to.bold, cur.italic && !to.italic) {
            (true, true) => out.push_str("***"),
            (true, false) => out.push_str("**"),
            (false, true) => out.push('*'),
            _ => {}
        }
        cur.bold &= to.bold;
        cur.italic &= to.italic;
    };
    for span in rt {
        let to = span.style;
        close(&mut out, &mut cur, to);
        match (to.bold && !cur.bold, to.italic && !cur.italic) {
            (true, true) => out.push_str("***"),
            (true, false) => out.push_str("**"),
            (false, true) => out.push('*'),
            _ => {}
        }
        if to.underline && !cur.underline {
            out.push('_');
        }
        cur = to;
        for c in span.text.chars() {
            if matches!(c, '*' | '_' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
    }
    close(&mut out, &mut cur, Style::default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Vec<Element> {
        parse(src).elements
    }

    #[test]
    fn title_page() {
        let s = parse("Title: Big Fish\nAuthor: John August\nContact:\n    Agent\n    555-1234\n\nINT. HOUSE - DAY\n");
        assert_eq!(s.title_value("title").unwrap(), &vec!["Big Fish".to_string()]);
        assert_eq!(s.title_value("contact").unwrap(), &vec!["Agent".to_string(), "555-1234".to_string()]);
        assert!(matches!(s.elements[0], Element::SceneHeading { .. }));
    }

    #[test]
    fn fade_in_is_not_title_page() {
        let s = parse("FADE IN:\n\nINT. HOUSE - DAY\n");
        assert!(s.title_page.is_empty());
        assert_eq!(s.elements.len(), 2);
    }

    #[test]
    fn scene_headings() {
        let e = p("INT. HOUSE - DAY #12A#\n\n.FLASHBACK\n\nint/ext car - night\n\nINTERIOR monologue here\n");
        assert_eq!(e[0], Element::SceneHeading { text: "INT. HOUSE - DAY".into(), number: Some("12A".into()) });
        assert_eq!(e[1], Element::SceneHeading { text: "FLASHBACK".into(), number: None });
        assert!(matches!(e[2], Element::SceneHeading { .. }));
        assert!(matches!(e[3], Element::Action(_)));
    }

    #[test]
    fn dialogue_and_parenthetical() {
        let e = p("\nSTEEL (O.S.)\n(quietly)\nWhere is it?\n");
        let Element::Dialogue(d) = &e[0] else { panic!("{e:?}") };
        assert_eq!(d.character, "STEEL (O.S.)");
        assert_eq!(d.parts[0], DialoguePart::Parenthetical("(quietly)".into()));
        assert_eq!(
            plain(match &d.parts[1] {
                DialoguePart::Line(l) => l,
                _ => panic!(),
            }),
            "Where is it?"
        );
    }

    #[test]
    fn lowercase_character_needs_at() {
        let e = p("\n@McCLANE\nYippee.\n");
        assert!(matches!(&e[0], Element::Dialogue(d) if d.character == "McCLANE"));
        let e = p("\nMcClane\nYippee.\n");
        assert!(matches!(e[0], Element::Action(_)));
    }

    #[test]
    fn dual_dialogue() {
        let e = p("BRICK\nScrew retirement.\n\nSTEEL ^\nScrew retirement.\n");
        assert!(matches!(&e[0], Element::DualDialogue(a, b) if a.character == "BRICK" && b.character == "STEEL"));
    }

    #[test]
    fn transitions_and_centered() {
        let e = p("Action.\n\nCUT TO:\n\n> Fade out.\n\n>THE END<\n");
        assert_eq!(e[1], Element::Transition("CUT TO:".into()));
        assert_eq!(e[2], Element::Transition("Fade out.".into()));
        assert!(matches!(&e[3], Element::Centered(l) if plain(&l[0]) == "THE END"));
    }

    #[test]
    fn page_break_sections_notes_boneyard() {
        let e = p(
            "# Act One\n\n= Setup\n\nHe runs. [[fix this]]\n[[whole line note]]\nHe stops.\n\n/* cut\nINT. GONE\n*/\n===\n",
        );
        assert_eq!(e[0], Element::Section { level: 1, text: "Act One".into() });
        assert_eq!(e[1], Element::Synopsis("Setup".into()));
        let Element::Action(lines) = &e[2] else { panic!("{e:?}") };
        assert_eq!(lines.len(), 2);
        assert_eq!(plain(&lines[0]), "He runs.");
        assert_eq!(e[3], Element::PageBreak);
        assert_eq!(e.len(), 4);
    }

    #[test]
    fn emphasis() {
        let r = parse_inline("a *it* **bo** ***bi*** _un_ 2*3 \\*lit\\*");
        let styles: Vec<(&str, Style)> = r.iter().map(|s| (s.text.as_str(), s.style)).collect();
        let st = |bold, italic, underline| Style { bold, italic, underline };
        assert_eq!(
            styles,
            vec![
                ("a ", st(false, false, false)),
                ("it", st(false, true, false)),
                (" ", st(false, false, false)),
                ("bo", st(true, false, false)),
                (" ", st(false, false, false)),
                ("bi", st(true, true, false)),
                (" ", st(false, false, false)),
                ("un", st(false, false, true)),
                (" 2*3 *lit*", st(false, false, false)),
            ]
        );
    }

    #[test]
    fn two_space_line_keeps_block_together() {
        let e = p("\nDEALER\nTen.\n  \nFour.\n");
        let Element::Dialogue(d) = &e[0] else { panic!() };
        assert_eq!(d.parts.len(), 3);
    }

    #[test]
    fn source_lines_kinds_and_notes() {
        let src = "Title: X\n\nINT. A - DAY\n\n/* gone\nINT. B\n*/\n[[note one]]\nBOB\n(beat)\nHi.\n";
        let s = parse(src);
        assert_eq!(s.element_lines, vec![2, 8]);
        use LineKind::*;
        assert_eq!(
            s.line_kinds,
            vec![
                TitleKey,
                Blank,
                SceneHeading,
                Blank,
                Comment,
                Comment,
                Comment,
                Comment,
                Character,
                Parenthetical,
                Dialogue,
                Blank
            ]
        );
        assert_eq!(s.notes, vec![Note { line: 7, text: "note one".into() }]);
    }

    #[test]
    fn writer_round_trips() {
        let src = include_str!("../../../examples/sample.fountain");
        let a = parse(src);
        let b = parse(&write(&a));
        assert_eq!(a.title_page, b.title_page);
        assert_eq!(a.elements, b.elements);

        // Things that need forcing.
        let tricky = "\n.FLASHBACK\n\n!INT. NOT A HEADING\n\n!LOUD NOISES\nKeep going.\n\n@McCLANE\nHi *there* **you** _u_ 2\\*3.\n\n> Fade out.\n";
        let a = parse(tricky);
        assert_eq!(a.elements, parse(&write(&a)).elements, "{}", write(&a));
    }
}
