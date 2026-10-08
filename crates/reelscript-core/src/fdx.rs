//! Final Draft `.fdx` import and export.
//!
//! FDX is XML: `<FinalDraft><Content><Paragraph Type="Scene Heading"><Text>…</Text>
//! </Paragraph>…</Content><TitlePage>…</TitlePage></FinalDraft>`. Inline styles live
//! in the `Style` attribute of each `<Text>` run (e.g. `Bold+Italic+Underline`), and
//! dual dialogue is wrapped in `<Paragraph><DualDialogue>…</DualDialogue></Paragraph>`.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::fountain::{Dialogue, DialoguePart, Element, RichText, Screenplay, Span, Style, plain};

#[derive(Debug)]
pub struct FdxError(pub String);

impl std::fmt::Display for FdxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid FDX: {}", self.0)
    }
}

impl std::error::Error for FdxError {}

// ---------------------------------------------------------------- export

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

fn style_attr(st: Style) -> String {
    let mut parts = Vec::new();
    if st.bold {
        parts.push("Bold");
    }
    if st.italic {
        parts.push("Italic");
    }
    if st.underline {
        parts.push("Underline");
    }
    if parts.is_empty() { String::new() } else { format!(" Style=\"{}\"", parts.join("+")) }
}

fn runs(rt: &RichText) -> String {
    if rt.is_empty() {
        return "<Text></Text>".into();
    }
    rt.iter().map(|s| format!("<Text{}>{}</Text>", style_attr(s.style), esc(&s.text))).collect()
}

fn para(out: &mut String, kind: &str, extra: &str, body: &str) {
    out.push_str(&format!("    <Paragraph Type=\"{kind}\"{extra}>{body}</Paragraph>\n"));
}

fn dialogue(out: &mut String, d: &Dialogue) {
    para(out, "Character", "", &runs(&plain_rt(&d.character)));
    for p in &d.parts {
        match p {
            DialoguePart::Parenthetical(t) => para(out, "Parenthetical", "", &runs(&plain_rt(t))),
            DialoguePart::Line(l) => para(out, "Dialogue", "", &runs(l)),
        }
    }
}

fn plain_rt(s: &str) -> RichText {
    if s.is_empty() { vec![] } else { vec![Span { text: s.to_string(), style: Style::default() }] }
}

/// Export a screenplay as Final Draft XML.
pub fn export(script: &Screenplay) -> String {
    let mut o = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>\n\
         <FinalDraft DocumentType=\"Script\" Template=\"No\" Version=\"5\">\n  <Content>\n",
    );
    let mut page_break_next = false;
    for el in &script.elements {
        let sp = if std::mem::take(&mut page_break_next) { " StartsNewPage=\"Yes\"" } else { "" };
        match el {
            Element::SceneHeading { text, number } => {
                let n = number.as_ref().map(|n| format!(" Number=\"{}\"", esc(n))).unwrap_or_default();
                para(&mut o, "Scene Heading", &format!("{n}{sp}"), &runs(&plain_rt(text)));
            }
            Element::Action(lines) => {
                // Final Draft keeps one paragraph per line break inside action; we
                // join with newlines in a single paragraph, which FD also accepts.
                let mut rt: RichText = Vec::new();
                for (i, l) in lines.iter().enumerate() {
                    if i > 0 {
                        rt.push(Span { text: "\n".into(), style: Style::default() });
                    }
                    rt.extend(l.iter().cloned());
                }
                para(&mut o, "Action", sp, &runs(&rt));
            }
            Element::Dialogue(d) => dialogue(&mut o, d),
            Element::DualDialogue(a, b) => {
                o.push_str("    <Paragraph><DualDialogue>\n");
                dialogue(&mut o, a);
                dialogue(&mut o, b);
                o.push_str("    </DualDialogue></Paragraph>\n");
            }
            Element::Transition(t) => para(&mut o, "Transition", sp, &runs(&plain_rt(t))),
            Element::Centered(lines) => {
                for l in lines {
                    para(&mut o, "Action", " Alignment=\"Center\"", &runs(l));
                }
            }
            Element::Lyrics(lines) => {
                for l in lines {
                    para(&mut o, "Lyrics", "", &runs(l));
                }
            }
            Element::PageBreak => page_break_next = true,
            Element::Section { level, text } => {
                para(&mut o, "Outline", &format!(" Level=\"{level}\""), &runs(&plain_rt(text)))
            }
            Element::Synopsis(s) => para(&mut o, "Synopsis", "", &runs(&plain_rt(s))),
        }
    }
    o.push_str("  </Content>\n");

    if !script.title_page.is_empty() {
        o.push_str("  <TitlePage>\n    <Content>\n");
        for (key, values) in &script.title_page {
            let (align, kind) = match key.as_str() {
                "title" | "credit" | "author" | "authors" | "source" => ("Center", "Title"),
                "draft date" | "date" | "revision" => ("Right", "Draft"),
                _ => ("Left", "Contact"),
            };
            for v in values {
                o.push_str(&format!(
                    "      <Paragraph Alignment=\"{align}\" Type=\"Text\" Key=\"{}\" Role=\"{kind}\">{}</Paragraph>\n",
                    esc(key),
                    runs(&plain_rt(v))
                ));
            }
        }
        o.push_str("    </Content>\n  </TitlePage>\n");
    }
    o.push_str("</FinalDraft>\n");
    o
}

// ---------------------------------------------------------------- import

#[derive(Default)]
struct Para {
    kind: String,
    number: Option<String>,
    new_page: bool,
    centered: bool,
    level: usize,
    key: Option<String>,
    role: Option<String>,
    text: RichText,
    in_dual: bool,
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    #[allow(deprecated)]
    e.try_get_attribute(name).ok().flatten().and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Parse a Final Draft `.fdx` document.
pub fn import(xml: &str) -> Result<Screenplay, FdxError> {
    let mut reader = Reader::from_str(xml);
    let mut paras: Vec<Para> = Vec::new();
    let mut title_paras: Vec<Para> = Vec::new();

    let mut in_title = false;
    let mut depth_para = 0usize; // nesting of <Paragraph>
    let mut dual = false;
    let mut cur: Option<Para> = None;
    let mut text_style: Option<Style> = None;
    let mut saw_root = false;

    let push_text = |cur: &mut Option<Para>, style: Option<Style>, s: &str| {
        if let (Some(p), Some(st)) = (cur.as_mut(), style) {
            match p.text.last_mut() {
                Some(last) if last.style == st => last.text.push_str(s),
                _ => p.text.push(Span { text: s.to_string(), style: st }),
            }
        }
    };

    loop {
        let ev = reader.read_event().map_err(|e| FdxError(e.to_string()))?;
        match ev {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == "FinalDraft" => saw_root = true,
            Event::Start(e) => match e.name().as_ref() {
                "TitlePage" => in_title = true,
                "DualDialogue" => dual = true,
                "Paragraph" => {
                    depth_para += 1;
                    if let Some(kind) = attr(&e, "Type") {
                        cur = Some(Para {
                            kind,
                            number: attr(&e, "Number"),
                            new_page: attr(&e, "StartsNewPage").as_deref() == Some("Yes"),
                            centered: attr(&e, "Alignment").as_deref() == Some("Center"),
                            level: attr(&e, "Level").and_then(|l| l.parse().ok()).unwrap_or(1),
                            key: attr(&e, "Key"),
                            role: attr(&e, "Alignment"),
                            text: vec![],
                            in_dual: dual,
                        });
                    }
                }
                "Text" => {
                    let s = attr(&e, "Style").unwrap_or_default();
                    text_style = Some(Style {
                        bold: s.contains("Bold"),
                        italic: s.contains("Italic"),
                        underline: s.contains("Underline"),
                    });
                }
                _ => {}
            },
            Event::Text(t) => push_text(&mut cur, text_style, &t.xml_content(quick_xml::XmlVersion::Implicit1_0)),
            Event::CData(t) => push_text(&mut cur, text_style, &t.into_inner()),
            Event::GeneralRef(r) => {
                let s = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match r.into_inner().as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => String::new(),
                    },
                };
                push_text(&mut cur, text_style, &s);
            }
            Event::End(e) => match e.name().as_ref() {
                "TitlePage" => in_title = false,
                "DualDialogue" => dual = false,
                "Text" => text_style = None,
                "Paragraph" => {
                    depth_para = depth_para.saturating_sub(1);
                    if let Some(p) = cur.take() {
                        if in_title { title_paras.push(p) } else { paras.push(p) }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_root {
        return Err(FdxError("missing <FinalDraft> root element".into()));
    }
    let _ = depth_para;

    let mut script = Screenplay { elements: build_elements(paras), ..Default::default() };
    script.title_page = build_title(title_paras);
    Ok(script)
}

fn trim_rt(mut rt: RichText) -> RichText {
    if let Some(f) = rt.first_mut() {
        f.text = f.text.trim_start().to_string();
    }
    if let Some(l) = rt.last_mut() {
        l.text = l.text.trim_end().to_string();
    }
    rt.retain(|s| !s.text.is_empty());
    rt
}

/// Split a rich text on `\n` into lines.
fn split_lines(rt: RichText) -> Vec<RichText> {
    let mut lines: Vec<RichText> = vec![vec![]];
    for span in rt {
        for (i, piece) in span.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(vec![]);
            }
            if !piece.is_empty() {
                lines.last_mut().unwrap().push(Span { text: piece.to_string(), style: span.style });
            }
        }
    }
    lines.into_iter().map(trim_rt).collect()
}

fn build_elements(paras: Vec<Para>) -> Vec<Element> {
    let mut els: Vec<Element> = Vec::new();
    let mut dual_buf: Vec<Dialogue> = Vec::new();
    let flush_dual = |els: &mut Vec<Element>, buf: &mut Vec<Dialogue>| {
        let mut it = std::mem::take(buf).into_iter();
        match (it.next(), it.next()) {
            (Some(a), Some(b)) => els.push(Element::DualDialogue(a, b)),
            (Some(a), None) => els.push(Element::Dialogue(a)),
            _ => {}
        }
        els.extend(it.map(Element::Dialogue));
    };

    for p in paras {
        if !p.in_dual && !dual_buf.is_empty() {
            flush_dual(&mut els, &mut dual_buf);
        }
        if p.new_page && !els.is_empty() {
            els.push(Element::PageBreak);
        }
        let text = plain(&p.text).trim().to_string();
        match p.kind.as_str() {
            "Scene Heading" => els.push(Element::SceneHeading { text, number: p.number }),
            "Action" | "General" | "Shot" | "Cast List" if p.centered => {
                let lines = split_lines(p.text);
                match els.last_mut() {
                    Some(Element::Centered(prev)) => prev.extend(lines),
                    _ => els.push(Element::Centered(lines)),
                }
            }
            "Action" | "General" | "Cast List" => els.push(Element::Action(split_lines(p.text))),
            "Shot" => els.push(Element::SceneHeading { text, number: p.number }),
            "Character" => {
                let d = Dialogue { character: text, parts: vec![] };
                if p.in_dual {
                    dual_buf.push(d);
                } else {
                    els.push(Element::Dialogue(d));
                }
            }
            "Parenthetical" | "Dialogue" => {
                let part = if p.kind == "Parenthetical" {
                    let t = if text.starts_with('(') { text } else { format!("({text})") };
                    DialoguePart::Parenthetical(t)
                } else {
                    DialoguePart::Line(trim_rt(p.text))
                };
                let target = if p.in_dual {
                    dual_buf.last_mut()
                } else {
                    match els.last_mut() {
                        Some(Element::Dialogue(d)) => Some(d),
                        Some(Element::DualDialogue(_, d)) => Some(d),
                        _ => None,
                    }
                };
                match (target, part) {
                    (Some(d), part) => d.parts.push(part),
                    (None, DialoguePart::Line(l)) => els.push(Element::Action(vec![l])),
                    (None, DialoguePart::Parenthetical(t)) => els.push(Element::Action(vec![plain_rt(&t)])),
                }
            }
            "Transition" => els.push(Element::Transition(text)),
            "Lyrics" => {
                let lines = split_lines(p.text);
                match els.last_mut() {
                    Some(Element::Lyrics(prev)) => prev.extend(lines),
                    _ => els.push(Element::Lyrics(lines)),
                }
            }
            "Outline" | "Act Break" | "New Act" | "End of Act" => {
                if !text.is_empty() {
                    els.push(Element::Section { level: p.level.max(1), text })
                }
            }
            "Synopsis" | "Summary" => els.push(Element::Synopsis(text)),
            _ if !text.is_empty() => els.push(Element::Action(split_lines(p.text))),
            _ => {}
        }
    }
    flush_dual(&mut els, &mut dual_buf);
    els
}

fn build_title(paras: Vec<Para>) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    // Files written by ReelScript carry explicit keys.
    if paras.iter().any(|p| p.key.is_some()) {
        for p in paras {
            let Some(k) = p.key else { continue };
            let v = plain(&p.text).trim().to_string();
            match out.last_mut() {
                Some((lk, vals)) if *lk == k => vals.push(v),
                _ => out.push((k, vec![v])),
            }
        }
        return out;
    }
    // Final Draft files: infer from alignment. First centered line is the title,
    // the rest of the centered block is credit/author; left = contact; right = date.
    let mut centered: Vec<String> = Vec::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for p in paras {
        let v = plain(&p.text).trim().to_string();
        if v.is_empty() {
            continue;
        }
        match p.role.as_deref() {
            Some("Center") => centered.push(v),
            Some("Right") => right.push(v),
            _ => left.push(v),
        }
    }
    let mut c = centered.into_iter();
    if let Some(t) = c.next() {
        out.push(("title".into(), vec![t]));
    }
    let rest: Vec<String> = c.collect();
    if let Some(pos) = rest.iter().position(|l| l.to_lowercase().contains("by")) {
        out.push(("credit".into(), vec![rest[pos].clone()]));
        let authors: Vec<String> = rest[pos + 1..].to_vec();
        if !authors.is_empty() {
            out.push(("author".into(), authors));
        }
    } else if !rest.is_empty() {
        out.push(("author".into(), rest));
    }
    if !left.is_empty() {
        out.push(("contact".into(), left));
    }
    if !right.is_empty() {
        out.push(("draft date".into(), right));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn round_trip_sample() {
        let src = include_str!("../../../examples/sample.fountain");
        let a = parse(src);
        let xml = export(&a);
        let b = import(&xml).unwrap();
        assert_eq!(a.title_page, b.title_page);
        assert_eq!(a.elements, b.elements);
    }

    #[test]
    fn imports_final_draft_style_file() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FinalDraft DocumentType="Script" Version="5">
<Content>
<Paragraph Type="Scene Heading" Number="1"><SceneProperties Length="1/8"/><Text>INT. LAB - NIGHT</Text></Paragraph>
<Paragraph Type="Action"><Text>A </Text><Text Style="Bold">huge</Text><Text> machine &amp; a cat.</Text></Paragraph>
<Paragraph Type="Character"><Text>DR. VOSS</Text></Paragraph>
<Paragraph Type="Parenthetical"><Text>(tired)</Text></Paragraph>
<Paragraph Type="Dialogue"><Text>It works.</Text></Paragraph>
<Paragraph><DualDialogue>
<Paragraph Type="Character"><Text>A</Text></Paragraph>
<Paragraph Type="Dialogue"><Text>Yes.</Text></Paragraph>
<Paragraph Type="Character"><Text>B</Text></Paragraph>
<Paragraph Type="Dialogue"><Text>No.</Text></Paragraph>
</DualDialogue></Paragraph>
<Paragraph Type="Transition" StartsNewPage="Yes"><Text>CUT TO:</Text></Paragraph>
</Content>
<TitlePage><Content>
<Paragraph Alignment="Center" Type="Text"><Text>MY FILM</Text></Paragraph>
<Paragraph Alignment="Center" Type="Text"><Text>Written by</Text></Paragraph>
<Paragraph Alignment="Center" Type="Text"><Text>Jo Smith</Text></Paragraph>
<Paragraph Alignment="Left" Type="Text"><Text>jo@example.com</Text></Paragraph>
</Content></TitlePage>
</FinalDraft>"#;
        let s = import(xml).unwrap();
        assert_eq!(s.elements[0], Element::SceneHeading { text: "INT. LAB - NIGHT".into(), number: Some("1".into()) });
        let Element::Action(a) = &s.elements[1] else { panic!() };
        assert_eq!(plain(&a[0]), "A huge machine & a cat.");
        assert!(a[0][1].style.bold);
        let Element::Dialogue(d) = &s.elements[2] else { panic!() };
        assert_eq!(d.character, "DR. VOSS");
        assert_eq!(d.parts.len(), 2);
        assert!(matches!(&s.elements[3], Element::DualDialogue(a, b) if a.character == "A" && b.character == "B"));
        assert_eq!(s.elements[4], Element::PageBreak);
        assert_eq!(s.elements[5], Element::Transition("CUT TO:".into()));
        assert_eq!(s.title_value("title").unwrap(), &vec!["MY FILM".to_string()]);
        assert_eq!(s.title_value("author").unwrap(), &vec!["Jo Smith".to_string()]);
        assert_eq!(s.title_value("contact").unwrap(), &vec!["jo@example.com".to_string()]);
    }

    #[test]
    fn rejects_non_fdx() {
        assert!(import("<html></html>").is_err());
        assert!(import("not xml <<<").is_err());
    }
}
