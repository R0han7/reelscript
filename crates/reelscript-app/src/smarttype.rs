//! SmartType: context-aware autocomplete for character names, scene headings,
//! locations, times of day, extensions and transitions — learned from the script.

use reelscript_core::report::{Report, split_heading};

const HEADING_PREFIXES: &[&str] = &["INT. ", "EXT. ", "INT./EXT. ", "EST. "];
const TIMES: &[&str] = &["DAY", "NIGHT", "CONTINUOUS", "LATER", "MOMENTS LATER", "MORNING", "EVENING", "DAWN", "DUSK"];
const EXTENSIONS: &[&str] = &["V.O.", "O.S.", "CONT'D", "O.C."];
const TRANSITIONS: &[&str] =
    &["CUT TO:", "FADE OUT.", "FADE TO:", "DISSOLVE TO:", "SMASH CUT TO:", "MATCH CUT TO:", "FADE IN:"];

/// Suggestions for the line being typed. Each suggestion is the full replacement
/// line. `prev_blank`: whether the line above is empty (start of a new element).
pub fn suggest(line: &str, prev_blank: bool, report: &Report) -> Vec<String> {
    let line = line.trim_start();
    if line.is_empty() || line.len() > 80 {
        return vec![];
    }
    let upper = line.to_uppercase();
    let mut out: Vec<String> = Vec::new();
    let add = |s: String, out: &mut Vec<String>| {
        if s.len() > line.len() && s.to_uppercase().starts_with(&upper) && !out.contains(&s) {
            out.push(s);
        }
    };

    // Scene headings: "INT. KIT" -> "INT. KITCHEN - ", "INT. KITCHEN - N" -> "... - NIGHT".
    if let Some(prefix) = HEADING_PREFIXES.iter().find(|p| upper.starts_with(*p)) {
        if let Some(dash) = line.rfind(" - ") {
            let base = &line[..dash + 3];
            let mut times: Vec<String> = TIMES.iter().map(|t| t.to_string()).collect();
            for l in &report.locations {
                for t in &l.times {
                    if !times.contains(t) {
                        times.push(t.clone());
                    }
                }
            }
            for t in times {
                add(format!("{base}{t}"), &mut out);
            }
        } else {
            let (_, loc, _) = split_heading(line);
            let p = &line[..prefix.trim_end().len()];
            for l in &report.locations {
                if l.name.starts_with(&loc) && l.name != loc {
                    add(format!("{p} {} - ", l.name), &mut out);
                }
            }
        }
        out.truncate(8);
        return out;
    }

    if !prev_blank {
        return vec![];
    }

    // Character extensions: "MARA (" -> "MARA (V.O.)".
    if let Some((name, ext)) = line.split_once('(') {
        let name = name.trim_end();
        if report.characters.iter().any(|c| c.name == name.to_uppercase()) {
            for e in EXTENSIONS {
                if e.starts_with(&ext.to_uppercase()) {
                    add(format!("{} ({e})", name.to_uppercase()), &mut out);
                }
            }
        }
        return out;
    }

    // Character names (most dialogue first), heading prefixes, transitions.
    for c in &report.characters {
        add(c.name.clone(), &mut out);
    }
    for p in HEADING_PREFIXES {
        add(p.to_string(), &mut out);
    }
    for t in TRANSITIONS {
        add(t.to_string(), &mut out);
    }
    out.truncate(8);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use reelscript_core::{layout::paginate, parse, report::build};

    fn report() -> Report {
        let s = parse("INT. KITCHEN - NIGHT\n\nMARA\nHi.\n\nMARCUS\nHey.\n\nEXT. KITE SHOP - DAY\n\nWind.\n");
        build(&s, &paginate(&s))
    }

    #[test]
    fn characters_and_prefixes() {
        let r = report();
        assert_eq!(suggest("MAR", true, &r), vec!["MARA", "MARCUS"]);
        assert_eq!(suggest("ma", true, &r), vec!["MARA", "MARCUS", "MATCH CUT TO:"]);
        assert!(suggest("MAR", false, &r).is_empty());
        assert_eq!(suggest("CU", true, &r), vec!["CUT TO:"]);
        assert_eq!(suggest("MARA (v", true, &r), vec!["MARA (V.O.)"]);
        assert_eq!(suggest("IN", true, &r), vec!["INT. ", "INT./EXT. "]);
    }

    #[test]
    fn headings() {
        let r = report();
        assert_eq!(suggest("INT. KIT", true, &r), vec!["INT. KITCHEN - ", "INT. KITE SHOP - "]);
        assert_eq!(suggest("INT. KITC", true, &r), vec!["INT. KITCHEN - "]);
        assert_eq!(suggest("INT. KITCHEN - N", true, &r), vec!["INT. KITCHEN - NIGHT"]);
        assert!(suggest("INT. KITCHEN - NIGHT", true, &r).is_empty());
    }
}
