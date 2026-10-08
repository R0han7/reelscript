//! Syntax highlighting for the Fountain editor, plus the app colour palettes.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId, Stroke, Visuals};
use reelscript_core::fountain::LineKind;

use crate::settings::AppTheme;

#[derive(Clone, Copy)]
pub struct Palette {
    pub text: Color32,
    pub heading: Color32,
    pub character: Color32,
    pub dialogue: Color32,
    pub parenthetical: Color32,
    pub transition: Color32,
    pub section: Color32,
    pub synopsis: Color32,
    pub comment: Color32,
    pub title: Color32,
    pub lyrics: Color32,
    pub editor_bg: Color32,
    pub page: Color32,
    pub ink: Color32,
    pub desk: Color32,
    pub accent: Color32,
}

pub fn palette(theme: AppTheme) -> Palette {
    match theme {
        AppTheme::Light => Palette {
            text: Color32::from_gray(30),
            heading: Color32::from_rgb(20, 70, 170),
            character: Color32::from_rgb(140, 40, 120),
            dialogue: Color32::from_gray(20),
            parenthetical: Color32::from_gray(110),
            transition: Color32::from_rgb(170, 90, 0),
            section: Color32::from_rgb(0, 120, 90),
            synopsis: Color32::from_rgb(90, 130, 90),
            comment: Color32::from_gray(150),
            title: Color32::from_rgb(120, 70, 20),
            lyrics: Color32::from_rgb(110, 60, 150),
            editor_bg: Color32::from_rgb(252, 251, 247),
            page: Color32::WHITE,
            ink: Color32::from_gray(15),
            desk: Color32::from_gray(200),
            accent: Color32::from_rgb(40, 110, 220),
        },
        AppTheme::Night => Palette {
            text: Color32::from_gray(210),
            heading: Color32::from_rgb(120, 180, 255),
            character: Color32::from_rgb(230, 150, 220),
            dialogue: Color32::from_gray(235),
            parenthetical: Color32::from_gray(150),
            transition: Color32::from_rgb(250, 180, 90),
            section: Color32::from_rgb(110, 210, 170),
            synopsis: Color32::from_rgb(140, 180, 140),
            comment: Color32::from_gray(110),
            title: Color32::from_rgb(230, 190, 130),
            lyrics: Color32::from_rgb(190, 160, 240),
            editor_bg: Color32::from_rgb(30, 32, 37),
            page: Color32::from_rgb(38, 40, 46),
            ink: Color32::from_rgb(222, 222, 214),
            desk: Color32::from_gray(18),
            accent: Color32::from_rgb(90, 160, 255),
        },
        AppTheme::Midnight => Palette {
            text: Color32::from_rgb(120, 200, 255),
            heading: Color32::from_rgb(255, 210, 90),
            character: Color32::from_rgb(120, 255, 200),
            dialogue: Color32::from_rgb(170, 220, 255),
            parenthetical: Color32::from_rgb(90, 140, 180),
            transition: Color32::from_rgb(255, 140, 120),
            section: Color32::from_rgb(200, 140, 255),
            synopsis: Color32::from_rgb(120, 170, 140),
            comment: Color32::from_rgb(70, 90, 110),
            title: Color32::from_rgb(255, 210, 90),
            lyrics: Color32::from_rgb(200, 160, 255),
            editor_bg: Color32::BLACK,
            page: Color32::BLACK,
            ink: Color32::from_rgb(120, 200, 255),
            desk: Color32::from_rgb(8, 10, 16),
            accent: Color32::from_rgb(255, 210, 90),
        },
    }
}

pub fn visuals(theme: AppTheme) -> Visuals {
    let p = palette(theme);
    let mut v = if theme == AppTheme::Light { Visuals::light() } else { Visuals::dark() };
    if theme == AppTheme::Midnight {
        v.panel_fill = Color32::from_rgb(6, 8, 12);
        v.window_fill = Color32::from_rgb(10, 12, 18);
        v.extreme_bg_color = Color32::BLACK;
        v.faint_bg_color = Color32::from_rgb(14, 18, 26);
    }
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.hyperlink_color = p.accent;
    v
}

fn color_for(kind: LineKind, p: &Palette) -> Color32 {
    match kind {
        LineKind::Blank | LineKind::Action | LineKind::Centered => p.text,
        LineKind::TitleKey | LineKind::TitleValue => p.title,
        LineKind::SceneHeading => p.heading,
        LineKind::Character => p.character,
        LineKind::Parenthetical => p.parenthetical,
        LineKind::Dialogue => p.dialogue,
        LineKind::Transition | LineKind::PageBreak => p.transition,
        LineKind::Lyrics => p.lyrics,
        LineKind::Section => p.section,
        LineKind::Synopsis => p.synopsis,
        LineKind::Comment => p.comment,
    }
}

/// Build the editor layout job. `kinds` are per source line; `[[notes]]` and
/// `/* boneyard */` spans inside lines are dimmed too.
pub fn highlight(text: &str, kinds: &[LineKind], p: &Palette, font: FontId, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut in_comment: Option<&str> = None; // closing marker while inside a comment
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let kind = kinds.get(i).copied().unwrap_or(LineKind::Action);
        let base = color_for(kind, p);
        let fmt = |color: Color32| {
            let mut f = TextFormat::simple(font.clone(), color);
            if kind == LineKind::SceneHeading {
                f.underline = Stroke::new(1.0, color.gamma_multiply(0.35));
            }
            if matches!(kind, LineKind::Synopsis | LineKind::Lyrics) {
                f.italics = true;
            }
            f
        };
        let mut rest = line;
        while !rest.is_empty() {
            if let Some(close) = in_comment {
                match rest.find(close) {
                    Some(e) => {
                        job.append(&rest[..e + close.len()], 0.0, fmt(p.comment));
                        rest = &rest[e + close.len()..];
                        in_comment = None;
                    }
                    None => {
                        job.append(rest, 0.0, fmt(p.comment));
                        rest = "";
                    }
                }
                continue;
            }
            let next = [("[[", "]]"), ("/*", "*/")]
                .iter()
                .filter_map(|(o, c)| rest.find(o).map(|i| (i, *c)))
                .min_by_key(|(i, _)| *i);
            match next {
                Some((s, close)) => {
                    if s > 0 {
                        job.append(&rest[..s], 0.0, fmt(base));
                    }
                    in_comment = Some(close);
                    // Consume the opener so `find(close)` can't match inside it.
                    job.append(&rest[s..s + 2], 0.0, fmt(p.comment));
                    rest = &rest[s + 2..];
                }
                None => {
                    job.append(rest, 0.0, fmt(base));
                    rest = "";
                }
            }
        }
    }
    job
}
