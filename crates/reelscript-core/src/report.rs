//! Script reports: statistics, scene, character, location and notes reports,
//! plus character interactions (who shares scenes with whom).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use crate::fountain::{Dialogue, DialoguePart, Element, Note, Screenplay, plain};
use crate::layout::{Layout, format_eighths};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stats {
    pub pages: usize,
    pub scenes: usize,
    pub words: usize,
    pub dialogue_words: usize,
    pub action_words: usize,
    pub speaking_characters: usize,
    /// Rule of thumb: one page ≈ one minute of screen time.
    pub est_minutes: f32,
    pub interior: usize,
    pub exterior: usize,
    pub day: usize,
    pub night: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneRow {
    pub number: Option<String>,
    pub heading: String,
    pub page: u32,
    pub eighths: u32,
    pub characters: Vec<String>,
    pub source_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterRow {
    pub name: String,
    pub speeches: usize,
    pub words: usize,
    /// Number of scenes the character speaks in.
    pub scenes: usize,
    /// Screen time estimate: sum of the lengths of those scenes, in eighths.
    pub eighths: u32,
    /// Share of all spoken words, 0–100.
    pub dialogue_share: f32,
    /// Heading of the first scene they speak in.
    pub first_scene: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocationRow {
    pub name: String,
    pub scenes: usize,
    pub eighths: u32,
    pub times: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub stats: Stats,
    pub scenes: Vec<SceneRow>,
    pub characters: Vec<CharacterRow>,
    /// (character, character, number of scenes shared), most frequent first.
    pub interactions: Vec<(String, String, usize)>,
    pub locations: Vec<LocationRow>,
    pub notes: Vec<Note>,
}

/// Canonical character name from a cue: `MARY (V.O.)` → `MARY`.
pub fn character_name(cue: &str) -> String {
    cue.split('(').next().unwrap_or("").trim().to_uppercase()
}

/// Split a scene heading into (INT/EXT prefix, location, time of day).
/// `INT. HOUSE - KITCHEN - NIGHT` → (`INT.`, `HOUSE - KITCHEN`, `NIGHT`).
pub fn split_heading(heading: &str) -> (String, String, String) {
    let upper = heading.trim().to_uppercase();
    let prefixes =
        ["INT./EXT.", "INT/EXT.", "INT./EXT", "INT/EXT", "I/E.", "I/E", "INT.", "EXT.", "EST.", "INT", "EXT", "EST"];
    let (prefix, rest) = prefixes
        .iter()
        .find_map(|p| {
            upper
                .strip_prefix(p)
                .filter(|r| r.is_empty() || r.starts_with(' ') || p.ends_with('.'))
                .map(|r| (p.to_string(), r.trim().to_string()))
        })
        .unwrap_or_default();
    let rest = if prefix.is_empty() { upper } else { rest };
    match rest.rsplit_once(" - ") {
        Some((loc, time)) => (prefix, loc.trim().to_string(), time.trim().to_string()),
        None => (prefix, rest, String::new()),
    }
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

fn dialogue_words(d: &Dialogue) -> usize {
    d.parts
        .iter()
        .map(|p| match p {
            DialoguePart::Line(l) => words(&plain(l)),
            DialoguePart::Parenthetical(_) => 0,
        })
        .sum()
}

#[derive(Default)]
struct CharAcc {
    speeches: usize,
    words: usize,
    scenes: Vec<usize>,
    first_seen: usize,
}

pub fn build(script: &Screenplay, layout: &Layout) -> Report {
    let mut r = Report { notes: script.notes.clone(), ..Default::default() };
    let s = &mut r.stats;
    s.pages = layout.body_pages();
    s.scenes = layout.scenes.len();
    s.est_minutes = layout.scenes.iter().map(|sc| sc.eighths).sum::<u32>() as f32 / 8.0;
    if s.scenes == 0 {
        s.est_minutes = s.pages as f32;
    }

    let mut chars: HashMap<String, CharAcc> = HashMap::new();
    let mut scene_chars: Vec<Vec<String>> = vec![Vec::new(); layout.scenes.len()];
    // Index of the current scene; None before the first heading.
    let mut scene: Option<usize> = None;
    let mut order = 0;

    let mut speak =
        |d: &Dialogue, scene: Option<usize>, chars: &mut HashMap<String, CharAcc>, sc: &mut Vec<Vec<String>>| {
            let name = character_name(&d.character);
            if name.is_empty() {
                return 0;
            }
            let w = dialogue_words(d);
            let acc = chars.entry(name.clone()).or_insert_with(|| {
                order += 1;
                CharAcc { first_seen: order, ..Default::default() }
            });
            acc.speeches += 1;
            acc.words += w;
            if let Some(i) = scene {
                if !acc.scenes.contains(&i) {
                    acc.scenes.push(i);
                }
                if let Some(list) = sc.get_mut(i)
                    && !list.contains(&name)
                {
                    list.push(name);
                }
            }
            w
        };

    for el in &script.elements {
        match el {
            Element::SceneHeading { .. } => scene = Some(scene.map_or(0, |i| i + 1)),
            Element::Action(lines) | Element::Centered(lines) | Element::Lyrics(lines) => {
                let w: usize = lines.iter().map(|l| words(&plain(l))).sum();
                s.action_words += w;
            }
            Element::Transition(t) => s.action_words += words(t),
            Element::Dialogue(d) => s.dialogue_words += speak(d, scene, &mut chars, &mut scene_chars),
            Element::DualDialogue(a, b) => {
                s.dialogue_words += speak(a, scene, &mut chars, &mut scene_chars);
                s.dialogue_words += speak(b, scene, &mut chars, &mut scene_chars);
            }
            _ => {}
        }
    }
    s.words = s.action_words + s.dialogue_words;
    s.speaking_characters = chars.len();

    // Scenes and locations.
    let mut locs: BTreeMap<String, LocationRow> = BTreeMap::new();
    for (i, sc) in layout.scenes.iter().enumerate() {
        let (prefix, loc, time) = split_heading(&sc.heading);
        if prefix.contains("INT") || prefix.starts_with("I/E") {
            s.interior += 1;
        }
        if prefix.contains("EXT") || prefix.starts_with("I/E") {
            s.exterior += 1;
        }
        if time.contains("NIGHT") {
            s.night += 1;
        } else if time.contains("DAY") || time.contains("MORNING") || time.contains("AFTERNOON") {
            s.day += 1;
        }
        let row =
            locs.entry(loc.clone()).or_insert_with(|| LocationRow { name: loc, scenes: 0, eighths: 0, times: vec![] });
        row.scenes += 1;
        row.eighths += sc.eighths;
        if !time.is_empty() && !row.times.contains(&time) {
            row.times.push(time);
        }
        r.scenes.push(SceneRow {
            number: sc.number.clone(),
            heading: sc.heading.clone(),
            page: layout.page_number(sc.page),
            eighths: sc.eighths,
            characters: scene_chars[i].clone(),
            source_line: sc.source_line,
        });
    }
    r.locations = locs.into_values().collect();
    r.locations.sort_by(|a, b| b.scenes.cmp(&a.scenes).then(b.eighths.cmp(&a.eighths)));

    // Characters, most dialogue first.
    let total_words = s.dialogue_words.max(1) as f32;
    let mut rows: Vec<(usize, CharacterRow)> = chars
        .into_iter()
        .map(|(name, acc)| {
            let first_scene = acc.scenes.iter().min().map(|&i| layout.scenes[i].heading.clone());
            let eighths = acc.scenes.iter().map(|&i| layout.scenes[i].eighths).sum();
            (
                acc.first_seen,
                CharacterRow {
                    name,
                    speeches: acc.speeches,
                    words: acc.words,
                    scenes: acc.scenes.len(),
                    eighths,
                    dialogue_share: acc.words as f32 * 100.0 / total_words,
                    first_scene,
                },
            )
        })
        .collect();
    rows.sort_by(|(fa, a), (fb, b)| b.words.cmp(&a.words).then(b.speeches.cmp(&a.speeches)).then(fa.cmp(fb)));
    r.characters = rows.into_iter().map(|(_, c)| c).collect();

    // Interactions: pairs speaking in the same scene.
    let mut pairs: BTreeMap<(String, String), usize> = BTreeMap::new();
    for list in &scene_chars {
        for (i, a) in list.iter().enumerate() {
            for b in &list[i + 1..] {
                let key = if a < b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
                *pairs.entry(key).or_default() += 1;
            }
        }
    }
    r.interactions = pairs.into_iter().map(|((a, b), n)| (a, b, n)).collect();
    r.interactions.sort_by_key(|i| std::cmp::Reverse(i.2));
    r
}

impl Report {
    /// Plain-text rendering of every report section.
    pub fn to_text(&self) -> String {
        let mut o = String::new();
        let s = &self.stats;
        let _ = writeln!(o, "SCRIPT STATISTICS");
        let _ = writeln!(o, "  Pages:               {}", s.pages);
        let _ = writeln!(o, "  Scenes:              {}", s.scenes);
        let _ = writeln!(o, "  Est. running time:   {:.0} min", s.est_minutes);
        let _ = writeln!(
            o,
            "  Words:               {} ({} dialogue, {} action)",
            s.words, s.dialogue_words, s.action_words
        );
        let _ = writeln!(o, "  Speaking characters: {}", s.speaking_characters);
        let _ = writeln!(o, "  INT / EXT:           {} / {}", s.interior, s.exterior);
        let _ = writeln!(o, "  DAY / NIGHT:         {} / {}", s.day, s.night);

        let _ = writeln!(o, "\nSCENE REPORT");
        let _ = writeln!(o, "  {:<5} {:<4} {:<7} {:<40} CHARACTERS", "#", "PAGE", "LENGTH", "HEADING");
        for sc in &self.scenes {
            let _ = writeln!(
                o,
                "  {:<5} {:<4} {:<7} {:<40} {}",
                sc.number.as_deref().unwrap_or("-"),
                sc.page,
                format_eighths(sc.eighths),
                sc.heading,
                sc.characters.join(", ")
            );
        }

        let _ = writeln!(o, "\nCHARACTER REPORT");
        let _ = writeln!(
            o,
            "  {:<24} {:>8} {:>6} {:>6} {:>8} {:>7}",
            "NAME", "SPEECHES", "WORDS", "SCENES", "PAGES", "SHARE"
        );
        for c in &self.characters {
            let _ = writeln!(
                o,
                "  {:<24} {:>8} {:>6} {:>6} {:>8} {:>6.1}%",
                c.name,
                c.speeches,
                c.words,
                c.scenes,
                format_eighths(c.eighths),
                c.dialogue_share
            );
        }

        if !self.interactions.is_empty() {
            let _ = writeln!(o, "\nCHARACTER INTERACTIONS (shared scenes)");
            for (a, b, n) in &self.interactions {
                let _ = writeln!(o, "  {a} & {b}: {n}");
            }
        }

        let _ = writeln!(o, "\nLOCATION REPORT");
        for l in &self.locations {
            let _ = writeln!(
                o,
                "  {:<36} {:>3} scene(s) {:>8}  {}",
                l.name,
                l.scenes,
                format_eighths(l.eighths),
                l.times.join(", ")
            );
        }

        if !self.notes.is_empty() {
            let _ = writeln!(o, "\nNOTES");
            for n in &self.notes {
                let _ = writeln!(o, "  line {:<5} {}", n.line + 1, n.text);
            }
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{layout::paginate, parse};

    #[test]
    fn heading_split() {
        assert_eq!(
            split_heading("INT. HOUSE - KITCHEN - NIGHT"),
            ("INT.".into(), "HOUSE - KITCHEN".into(), "NIGHT".into())
        );
        assert_eq!(split_heading("int/ext car - day"), ("INT/EXT".into(), "CAR".into(), "DAY".into()));
        assert_eq!(split_heading("FLASHBACK"), ("".into(), "FLASHBACK".into(), "".into()));
    }

    #[test]
    fn report_counts() {
        let src = "INT. HOUSE - DAY\n\nBOB\nHello there friend.\n\nMARY (V.O.)\nHi.\n\nEXT. STREET - NIGHT\n\nBOB\nBye. [[check]]\n\nINT. HOUSE - NIGHT\n\nMARY\nBack.\n";
        let s = parse(src);
        let r = build(&s, &paginate(&s));
        assert_eq!(r.stats.scenes, 3);
        assert_eq!(r.stats.dialogue_words, 6);
        assert_eq!((r.stats.interior, r.stats.exterior, r.stats.day, r.stats.night), (2, 1, 1, 2));
        assert_eq!(r.characters[0].name, "BOB");
        assert_eq!((r.characters[0].speeches, r.characters[0].words, r.characters[0].scenes), (2, 4, 2));
        assert_eq!(r.characters[1].name, "MARY");
        assert_eq!(r.interactions, vec![("BOB".into(), "MARY".into(), 1)]);
        assert_eq!(r.locations[0].name, "HOUSE");
        assert_eq!(r.locations[0].scenes, 2);
        assert_eq!(r.locations[0].times, vec!["DAY".to_string(), "NIGHT".to_string()]);
        assert_eq!(r.scenes[0].characters, vec!["BOB".to_string(), "MARY".to_string()]);
        assert_eq!(r.notes.len(), 1);
        assert!(r.to_text().contains("CHARACTER REPORT"));
    }
}
