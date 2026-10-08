//! ReelScript desktop editor.
//!
//! Fountain editor with syntax highlighting and SmartType autocomplete, a live
//! paginated preview, navigator (scenes / characters / locations / notes / goals),
//! index cards, focus + typewriter modes, Light / Night / Midnight themes, writing
//! goals and sprints, auto-backup, find & replace, and PDF / FDX / Fountain export.

mod highlight;
mod settings;
mod smarttype;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{
    self, Align, FontId, Galley, Id, Key, KeyboardShortcut, Modifiers, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    TextFormat, UiBuilder, Vec2,
};
use reelscript_core::fountain::{Element, LineKind, Screenplay, plain};
use reelscript_core::layout::{self, CHAR_WIDTH, Layout, LayoutOptions, PAGE_HEIGHT, PAGE_WIDTH, format_eighths};
use reelscript_core::pdf::{self, PdfOptions};
use reelscript_core::report::{self, Report};
use reelscript_core::{document_title, fdx, fountain, load_as_fountain, parse};

use highlight::{Palette, palette};
use settings::{AppTheme, PdfColors, Settings, WritingStats, count_words};

const SAMPLE: &str = include_str!("../../../examples/sample.fountain");
const OPEN_EXTS: &[&str] = &["fountain", "spmd", "txt", "fdx"];
const EDITOR_ID: &str = "script-editor";
const NEW_SCRIPT: &str =
    "Title: Untitled\nCredit: written by\nAuthor: \nDraft date: \n\nFADE IN:\n\nINT. LOCATION - DAY\n\n";

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1500.0, 920.0]).with_min_inner_size([700.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("ReelScript", options, Box::new(move |cc| Ok(Box::new(App::new(&cc.egui_ctx, path)))))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavTab {
    Scenes,
    Characters,
    Locations,
    Notes,
    Goals,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Script,
    Cards,
}

/// An action that would discard unsaved changes, awaiting confirmation.
#[derive(Clone, PartialEq)]
enum Pending {
    New,
    Open,
    OpenPath(PathBuf),
    Restore(PathBuf),
    Quit,
}

struct Sprint {
    start: Instant,
    minutes: u64,
    start_words: usize,
}

struct App {
    path: Option<PathBuf>,
    /// Suggested file name when the document didn't come from a .fountain file.
    suggested_name: Option<String>,
    text: String,
    dirty: bool,

    script: Screenplay,
    layout: Layout,
    report: Report,
    /// Per scene: synopsis or first line of action, for index cards.
    card_text: Vec<String>,
    title: String,
    word_count: usize,

    settings: Settings,
    saved_settings: Settings,
    stats: WritingStats,
    status: String,

    nav: NavTab,
    view: View,
    focus_mode: bool,
    selected_character: Option<String>,

    // Editor state carried between frames.
    hl_cache: (String, Vec<LineKind>),
    cursor: usize,
    selection: (usize, usize),
    cursor_line: usize,
    pending_cursor: Option<CCursorRange>,
    pending_scroll: Option<f32>,
    reveal_cursor: bool,
    viewport_h: f32,
    scroll_to_page: Option<usize>,

    suggestions: Vec<String>,
    dismissed: Option<(usize, String)>,

    find_open: bool,
    find_focus: bool,
    find_query: String,
    replace_text: String,
    find_case: bool,

    export_open: bool,
    backups_open: bool,
    backups: Vec<PathBuf>,
    help_open: bool,
    confirm: Option<Pending>,
    allow_close: bool,

    last_backup: Instant,
    backup_pending: bool,
    last_autosave: Instant,
    last_stats_save: Instant,
    sprint: Option<Sprint>,
    sprint_minutes: u64,
    last_window_title: String,
    applied_theme: Option<AppTheme>,
    /// Dev/docs helper: `REELSCRIPT_SCREENSHOT=out.ppm` saves the window and exits.
    screenshot: Option<(PathBuf, u32)>,
}

// ---------------------------------------------------------------- text helpers

fn byte_of_char(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b)
}

fn char_of_byte(s: &str, b: usize) -> usize {
    s[..b.min(s.len())].chars().count()
}

/// Char index where 0-based `line` starts.
fn line_start_char(s: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut n = 0;
    for (i, c) in s.chars().enumerate() {
        if c == '\n' {
            n += 1;
            if n == line {
                return i + 1;
            }
        }
    }
    s.chars().count()
}

fn line_of_char(s: &str, ci: usize) -> usize {
    s.chars().take(ci).filter(|&c| c == '\n').count()
}

fn kind_name(k: LineKind) -> &'static str {
    match k {
        LineKind::Blank => "",
        LineKind::TitleKey | LineKind::TitleValue => "Title Page",
        LineKind::SceneHeading => "Scene Heading",
        LineKind::Action => "Action",
        LineKind::Character => "Character",
        LineKind::Parenthetical => "Parenthetical",
        LineKind::Dialogue => "Dialogue",
        LineKind::Transition => "Transition",
        LineKind::Centered => "Centered",
        LineKind::Lyrics => "Lyrics",
        LineKind::PageBreak => "Page Break",
        LineKind::Section => "Section",
        LineKind::Synopsis => "Synopsis",
        LineKind::Comment => "Note / Boneyard",
    }
}

impl App {
    fn new(ctx: &egui::Context, path: Option<PathBuf>) -> Self {
        let settings = Settings::load();
        let mut app = Self {
            path: None,
            suggested_name: None,
            text: SAMPLE.to_string(),
            dirty: false,
            script: Screenplay::default(),
            layout: Layout::default(),
            report: Report::default(),
            card_text: Vec::new(),
            title: String::new(),
            word_count: 0,
            saved_settings: settings.clone(),
            settings,
            stats: WritingStats::load(),
            status: "Welcome to ReelScript".into(),
            nav: NavTab::Scenes,
            view: View::Script,
            focus_mode: false,
            selected_character: None,
            hl_cache: (String::new(), Vec::new()),
            cursor: 0,
            selection: (0, 0),
            cursor_line: 0,
            pending_cursor: None,
            pending_scroll: None,
            reveal_cursor: false,
            viewport_h: 600.0,
            scroll_to_page: None,
            suggestions: Vec::new(),
            dismissed: None,
            find_open: false,
            find_focus: false,
            find_query: String::new(),
            replace_text: String::new(),
            find_case: false,
            export_open: false,
            backups_open: false,
            backups: Vec::new(),
            help_open: false,
            confirm: None,
            allow_close: false,
            last_backup: Instant::now(),
            backup_pending: false,
            last_autosave: Instant::now(),
            last_stats_save: Instant::now(),
            sprint: None,
            sprint_minutes: 25,
            last_window_title: String::new(),
            applied_theme: None,
            screenshot: std::env::var_os("REELSCRIPT_SCREENSHOT").map(|p| (PathBuf::from(p), 0)),
        };
        if let Some(p) = path.or_else(|| app.settings.recent.first().filter(|p| p.exists()).cloned()) {
            app.open_path(p);
        }
        app.reflow();
        app.word_count = count_words(&app.text);
        let _ = ctx;
        if app.screenshot.is_some() {
            // Deterministic look for docs: REELSCRIPT_SHOT_MODE=cards|focus|light|midnight|smart
            let mode = std::env::var("REELSCRIPT_SHOT_MODE").unwrap_or_default();
            app.settings.theme = AppTheme::Night;
            app.settings.show_navigator = true;
            app.settings.show_preview = true;
            match mode.as_str() {
                "cards" => app.view = View::Cards,
                "focus" => app.focus_mode = true,
                "light" => app.settings.theme = AppTheme::Light,
                "midnight" => {
                    app.settings.theme = AppTheme::Midnight;
                    app.nav = NavTab::Characters;
                }
                "goals" => app.nav = NavTab::Goals,
                "smart" => {
                    app.text.push_str("\nINT. PROJ");
                    app.on_text_changed();
                    let end = app.text.chars().count();
                    app.pending_cursor = Some(CCursorRange::one(CCursor::new(end)));
                    app.reveal_cursor = true;
                    app.cursor = end;
                    app.cursor_line = line_of_char(&app.text, end);
                    app.update_suggestions();
                }
                _ => {}
            }
            // Don't persist screenshot-mode settings.
            app.saved_settings = app.settings.clone();
        }
        app
    }

    fn layout_options(&self) -> LayoutOptions {
        LayoutOptions { number_scenes: self.settings.number_scenes, bold_headings: self.settings.bold_headings }
    }

    fn reflow(&mut self) {
        self.script = parse(&self.text);
        self.layout = layout::paginate_with(&self.script, &self.layout_options());
        self.report = report::build(&self.script, &self.layout);
        self.title = document_title(&self.script);
        self.card_text = card_texts(&self.script);
    }

    fn on_text_changed(&mut self) {
        self.dirty = true;
        self.backup_pending = true;
        let words = count_words(&self.text);
        self.stats.record(words as isize - self.word_count as isize);
        self.word_count = words;
        self.reflow();
    }

    // ------------------------------------------------------------ files

    fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| self.suggested_name.clone())
            .unwrap_or_else(|| "Untitled".into())
    }

    fn request(&mut self, action: Pending) {
        if self.dirty {
            self.confirm = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: Pending) {
        match action {
            Pending::New => self.new_file(),
            Pending::Open => {
                if let Some(p) = rfd::FileDialog::new().add_filter("Screenplay", OPEN_EXTS).pick_file() {
                    self.open_path(p);
                }
            }
            Pending::OpenPath(p) => self.open_path(p),
            Pending::Restore(p) => match std::fs::read_to_string(&p) {
                Ok(t) => {
                    self.text = t;
                    self.on_text_changed();
                    self.status = format!("Restored backup {}", p.file_name().unwrap_or_default().to_string_lossy());
                }
                Err(e) => self.status = format!("Could not read backup: {e}"),
            },
            Pending::Quit => self.allow_close = true,
        }
    }

    fn reset_document(&mut self, text: String) {
        self.text = text;
        self.word_count = count_words(&self.text);
        self.dirty = false;
        self.backup_pending = false;
        self.pending_cursor = Some(CCursorRange::one(CCursor::new(0)));
        self.pending_scroll = Some(0.0);
        self.suggestions.clear();
        self.reflow();
    }

    fn new_file(&mut self) {
        self.path = None;
        self.suggested_name = None;
        self.reset_document(NEW_SCRIPT.into());
        self.status = "New screenplay".into();
    }

    fn open_path(&mut self, p: PathBuf) {
        match load_as_fountain(&p) {
            Ok(t) => {
                let is_fdx = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("fdx"));
                self.settings.add_recent(&p);
                if is_fdx {
                    // Never write Fountain over the original .fdx.
                    self.suggested_name = p.with_extension("fountain").file_name().map(|n| n.to_string_lossy().into());
                    self.path = None;
                    self.status =
                        format!("Imported Final Draft file {} — Save will create a .fountain file", p.display());
                } else {
                    self.suggested_name = None;
                    self.path = Some(p.clone());
                    self.status = format!("Opened {}", p.display());
                }
                self.reset_document(t);
                self.dirty = is_fdx;
            }
            Err(e) => self.status = format!("Could not open {}: {e}", p.display()),
        }
    }

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => self.write_to(p),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let name = self.suggested_name.clone().unwrap_or_else(|| format!("{}.fountain", self.title));
        if let Some(p) = rfd::FileDialog::new().add_filter("Fountain", &["fountain"]).set_file_name(name).save_file() {
            self.write_to(p);
        }
    }

    fn write_to(&mut self, p: PathBuf) {
        match std::fs::write(&p, &self.text) {
            Ok(()) => {
                self.status = format!("Saved {}", p.display());
                self.settings.add_recent(&p);
                self.path = Some(p);
                self.suggested_name = None;
                self.dirty = false;
                self.stats.save_if_dirty();
            }
            Err(e) => self.status = format!("Could not save {}: {e}", p.display()),
        }
    }

    fn export_name(&self, ext: &str) -> String {
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| self.suggested_name.as_ref().map(|n| n.trim_end_matches(".fountain").to_string()))
            .unwrap_or_else(|| self.title.clone());
        format!("{stem}.{ext}")
    }

    fn export(&mut self, kind: &str) {
        let (filter, ext) = match kind {
            "pdf" => ("PDF", "pdf"),
            "fdx" => ("Final Draft", "fdx"),
            "fountain" => ("Fountain", "fountain"),
            _ => ("Text report", "txt"),
        };
        let Some(p) =
            rfd::FileDialog::new().add_filter(filter, &[ext]).set_file_name(self.export_name(ext)).save_file()
        else {
            return;
        };
        let bytes = match kind {
            "pdf" => {
                let opts = PdfOptions {
                    title: self.title.clone(),
                    watermark: Some(self.settings.watermark.clone()).filter(|w| !w.trim().is_empty()),
                    theme: self.settings.pdf_theme(),
                };
                pdf::render_with(&self.layout.pages, &opts)
            }
            "fdx" => fdx::export(&self.script).into_bytes(),
            "fountain" => fountain::write(&self.script).into_bytes(),
            _ => self.report.to_text().into_bytes(),
        };
        self.status = match std::fs::write(&p, bytes) {
            Ok(()) => format!("Exported {}", p.display()),
            Err(e) => format!("Could not export {}: {e}", p.display()),
        };
    }

    fn backup_now(&mut self) {
        match settings::write_backup(self.path.as_deref(), &self.text) {
            Ok(_) => {
                self.backup_pending = false;
                self.last_backup = Instant::now();
            }
            Err(e) => self.status = format!("Backup failed: {e}"),
        }
    }

    fn timers(&mut self, ctx: &egui::Context) {
        let mins = self.settings.backup_minutes.max(1);
        if self.backup_pending && self.last_backup.elapsed() >= Duration::from_secs(mins * 60) {
            self.backup_now();
        }
        if self.settings.autosave
            && self.dirty
            && self.path.is_some()
            && self.last_autosave.elapsed() >= Duration::from_secs(30)
        {
            self.last_autosave = Instant::now();
            self.save();
        }
        if self.last_stats_save.elapsed() >= Duration::from_secs(30) {
            self.last_stats_save = Instant::now();
            self.stats.save_if_dirty();
        }
        if self.settings != self.saved_settings && self.screenshot.is_none() {
            if let Err(e) = self.settings.save() {
                self.status = format!("Could not save settings: {e}");
            }
            if self.settings.number_scenes != self.saved_settings.number_scenes
                || self.settings.bold_headings != self.saved_settings.bold_headings
            {
                self.reflow();
            }
            self.saved_settings = self.settings.clone();
        }
        if let Some(s) = &self.sprint
            && s.start.elapsed() >= Duration::from_secs(s.minutes * 60)
        {
            let words = self.word_count.saturating_sub(s.start_words);
            self.status = format!("Sprint finished: {words} words in {} minutes", s.minutes);
            self.sprint = None;
        }
        ctx.request_repaint_after(Duration::from_secs(if self.sprint.is_some() { 1 } else { 15 }));
    }

    // ------------------------------------------------------------ editing actions

    fn jump_to_line(&mut self, line: usize) {
        let ci = line_start_char(&self.text, line);
        self.pending_cursor = Some(CCursorRange::one(CCursor::new(ci)));
        self.reveal_cursor = true;
        self.view = View::Script;
    }

    fn apply_suggestion(&mut self, i: usize) {
        let Some(s) = self.suggestions.get(i).cloned() else { return };
        let start_c = line_start_char(&self.text, self.cursor_line);
        let start = byte_of_char(&self.text, start_c);
        let end = self.text[start..].find('\n').map_or(self.text.len(), |e| start + e);
        self.text.replace_range(start..end, &s);
        let ci = start_c + s.chars().count();
        self.pending_cursor = Some(CCursorRange::one(CCursor::new(ci)));
        self.suggestions.clear();
        self.on_text_changed();
    }

    fn update_suggestions(&mut self) {
        self.suggestions.clear();
        if !self.settings.smart_type || self.selection.0 != self.selection.1 {
            return;
        }
        let lines: Vec<&str> = self.text.split('\n').collect();
        let Some(line) = lines.get(self.cursor_line) else { return };
        // Only complete at the end of the line.
        let line_end = line_start_char(&self.text, self.cursor_line) + line.chars().count();
        if self.cursor != line_end {
            return;
        }
        if self.dismissed.as_ref().is_some_and(|(l, t)| *l == self.cursor_line && t == line) {
            return;
        }
        let prev_blank = self.cursor_line == 0 || lines[self.cursor_line - 1].trim().is_empty();
        self.suggestions = smarttype::suggest(line, prev_blank, &self.report);
    }

    fn find_next(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        let (hay, needle) = if self.find_case {
            (self.text.clone(), self.find_query.clone())
        } else {
            (self.text.to_ascii_lowercase(), self.find_query.to_ascii_lowercase())
        };
        let from = byte_of_char(&self.text, self.selection.1);
        let found = hay[from..].find(&needle).map(|i| i + from).or_else(|| hay.find(&needle));
        match found {
            Some(b) => {
                let s = char_of_byte(&self.text, b);
                let e = s + self.find_query.chars().count();
                self.pending_cursor = Some(CCursorRange::two(CCursor::new(s), CCursor::new(e)));
                self.selection = (s, e);
                self.reveal_cursor = true;
                self.view = View::Script;
            }
            None => self.status = format!("\"{}\" not found", self.find_query),
        }
    }

    fn selection_matches(&self) -> bool {
        let (s, e) = self.selection;
        let sel: String = self.text.chars().skip(s).take(e.saturating_sub(s)).collect();
        if self.find_case { sel == self.find_query } else { sel.eq_ignore_ascii_case(&self.find_query) }
    }

    fn replace_one(&mut self) {
        if !self.find_query.is_empty() && self.selection_matches() {
            let (s, e) = self.selection;
            let (bs, be) = (byte_of_char(&self.text, s), byte_of_char(&self.text, e));
            self.text.replace_range(bs..be, &self.replace_text);
            let end = s + self.replace_text.chars().count();
            self.selection = (end, end);
            self.on_text_changed();
        }
        self.find_next();
    }

    fn replace_all(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        let (hay, needle) = if self.find_case {
            (self.text.clone(), self.find_query.clone())
        } else {
            (self.text.to_ascii_lowercase(), self.find_query.to_ascii_lowercase())
        };
        let mut out = String::with_capacity(self.text.len());
        let mut last = 0;
        let mut n = 0;
        for (i, _) in hay.match_indices(&needle) {
            out.push_str(&self.text[last..i]);
            out.push_str(&self.replace_text);
            last = i + needle.len();
            n += 1;
        }
        out.push_str(&self.text[last..]);
        if n > 0 {
            self.text = out;
            self.on_text_changed();
        }
        self.status = format!("Replaced {n} occurrence(s)");
    }

    /// Move scene `i` by `delta` (-1 / +1) in the text, as on an index-card board.
    fn move_scene(&mut self, i: usize, delta: isize) {
        let j = i as isize + delta;
        let n = self.layout.scenes.len();
        if j < 0 || j as usize >= n {
            return;
        }
        let starts: Vec<usize> = self.layout.scenes.iter().filter_map(|s| s.source_line).collect();
        if starts.len() != n {
            return;
        }
        let lines: Vec<&str> = self.text.split('\n').collect();
        let mut chunks: Vec<Vec<&str>> =
            (0..n).map(|k| lines[starts[k]..starts.get(k + 1).copied().unwrap_or(lines.len())].to_vec()).collect();
        for c in &mut chunks {
            while c.last().is_some_and(|l| l.trim().is_empty()) {
                c.pop();
            }
        }
        chunks.swap(i, j as usize);
        let mut out = String::new();
        if starts[0] > 0 {
            out.push_str(&lines[..starts[0]].join("\n"));
            out.push('\n');
        }
        out.push_str(&chunks.iter().map(|c| c.join("\n")).collect::<Vec<_>>().join("\n\n"));
        out.push('\n');
        self.text = out;
        self.on_text_changed();
        self.status = format!("Moved scene {} to position {}", i + 1, j + 1);
    }

    // ------------------------------------------------------------ input

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |key, shift: bool| {
            let mods = if shift { Modifiers::COMMAND | Modifiers::SHIFT } else { Modifiers::COMMAND };
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(mods, key)))
        };
        if cmd(Key::S, true) {
            self.save_as();
        } else if cmd(Key::S, false) {
            self.save();
        }
        if cmd(Key::O, false) {
            self.request(Pending::Open);
        }
        if cmd(Key::N, false) {
            self.request(Pending::New);
        }
        if cmd(Key::E, false) {
            self.export_open = true;
        }
        if cmd(Key::P, false) {
            self.export("pdf");
        }
        if cmd(Key::F, true) {
            self.focus_mode = !self.focus_mode;
        } else if cmd(Key::F, false) {
            self.find_open = true;
            self.find_focus = true;
        }
        if cmd(Key::T, false) {
            self.settings.typewriter = !self.settings.typewriter;
        }
        if cmd(Key::Num1, false) {
            self.view = View::Script;
        }
        if cmd(Key::Num2, false) {
            self.view = View::Cards;
        }
        if cmd(Key::Equals, false) || cmd(Key::Plus, false) {
            self.settings.font_size = (self.settings.font_size + 1.0).min(32.0);
        }
        if cmd(Key::Minus, false) {
            self.settings.font_size = (self.settings.font_size - 1.0).max(9.0);
        }
        if self.suggestions.is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if self.find_open {
                self.find_open = false;
            } else {
                self.focus_mode = false;
            }
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let t = format!("{}{} — ReelScript", if self.dirty { "• " } else { "" }, self.file_name());
        if t != self.last_window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.clone()));
            self.last_window_title = t;
        }
    }

    // ------------------------------------------------------------ UI: chrome

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New                Ctrl+N").clicked() {
                    self.request(Pending::New);
                }
                if ui.button("Open…  (.fountain / .fdx)  Ctrl+O").clicked() {
                    self.request(Pending::Open);
                }
                ui.menu_button("Open Recent", |ui| {
                    if self.settings.recent.is_empty() {
                        ui.label(RichText::new("No recent files").weak());
                    }
                    for p in self.settings.recent.clone() {
                        if ui.button(p.display().to_string()).clicked() {
                            self.request(Pending::OpenPath(p));
                        }
                    }
                });
                ui.separator();
                if ui.button("Save               Ctrl+S").clicked() {
                    self.save();
                }
                if ui.button("Save As…           Ctrl+Shift+S").clicked() {
                    self.save_as();
                }
                ui.checkbox(&mut self.settings.autosave, "Autosave every 30 s");
                ui.separator();
                if ui.button("Export…            Ctrl+E").clicked() {
                    self.export_open = true;
                }
                if ui.button("Quick PDF Export   Ctrl+P").clicked() {
                    self.export("pdf");
                }
                ui.separator();
                if ui.button("Backups…").clicked() {
                    self.backups = settings::list_backups(self.path.as_deref());
                    self.backups_open = true;
                }
                if ui.button("Back Up Now").clicked() {
                    self.backup_now();
                    self.status = "Backup written".into();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Find & Replace     Ctrl+F").clicked() {
                    self.find_open = true;
                    self.find_focus = true;
                }
                ui.checkbox(&mut self.settings.smart_type, "SmartType autocomplete");
            });
            ui.menu_button("View", |ui| {
                ui.radio_value(&mut self.view, View::Script, "Script             Ctrl+1");
                ui.radio_value(&mut self.view, View::Cards, "Index Cards        Ctrl+2");
                ui.separator();
                ui.checkbox(&mut self.settings.show_navigator, "Navigator");
                ui.checkbox(&mut self.settings.show_preview, "Page Preview");
                ui.checkbox(&mut self.focus_mode, "Focus Mode         Ctrl+Shift+F");
                ui.checkbox(&mut self.settings.typewriter, "Typewriter Mode    Ctrl+T");
                ui.separator();
                for t in AppTheme::ALL {
                    ui.radio_value(&mut self.settings.theme, t, format!("{} theme", t.name()));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Font size");
                    ui.add(egui::Slider::new(&mut self.settings.font_size, 9.0..=32.0).step_by(1.0));
                });
            });
            ui.menu_button("Format", |ui| {
                ui.checkbox(&mut self.settings.number_scenes, "Number scenes automatically");
                ui.checkbox(&mut self.settings.bold_headings, "Bold scene headings");
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Fountain Syntax").clicked() {
                    self.help_open = true;
                }
            });
            ui.separator();
            ui.selectable_value(&mut self.view, View::Script, "📝 Script");
            ui.selectable_value(&mut self.view, View::Cards, "🗂 Cards");
            ui.toggle_value(&mut self.focus_mode, "🎯 Focus");
            ui.toggle_value(&mut self.settings.typewriter, "⌨ Typewriter");
            egui::ComboBox::from_id_salt("theme").selected_text(self.settings.theme.name()).width(90.0).show_ui(
                ui,
                |ui| {
                    for t in AppTheme::ALL {
                        ui.selectable_value(&mut self.settings.theme, t, t.name());
                    }
                },
            );
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let kind = self.script.line_kinds.get(self.cursor_line).copied().unwrap_or(LineKind::Blank);
            ui.label(format!("Ln {}", self.cursor_line + 1));
            if !kind_name(kind).is_empty() {
                ui.label(RichText::new(kind_name(kind)).strong());
            }
            ui.separator();
            ui.label(format!(
                "{} pages · {} scenes · {} words · ~{:.0} min",
                self.layout.body_pages(),
                self.layout.scenes.len(),
                self.word_count,
                self.report.stats.est_minutes
            ));
            ui.separator();
            let goal = self.settings.daily_goal.max(1);
            let today = self.stats.today_words();
            ui.add(
                egui::ProgressBar::new((today as f32 / goal as f32).min(1.0))
                    .desired_width(110.0)
                    .text(format!("{today}/{goal} today")),
            );
            if let Some(s) = &self.sprint {
                let left = (s.minutes * 60).saturating_sub(s.start.elapsed().as_secs());
                ui.label(
                    RichText::new(format!(
                        "⏱ {:02}:{:02} · {} words",
                        left / 60,
                        left % 60,
                        self.word_count.saturating_sub(s.start_words)
                    ))
                    .strong(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&self.status).weak());
            });
        });
    }

    // ------------------------------------------------------------ UI: navigator

    fn navigator(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (NavTab::Scenes, "Scenes"),
                (NavTab::Characters, "Characters"),
                (NavTab::Locations, "Locations"),
                (NavTab::Notes, "Notes"),
                (NavTab::Goals, "Goals"),
            ] {
                ui.selectable_value(&mut self.nav, tab, label);
            }
        });
        ui.separator();
        ScrollArea::vertical().id_salt("nav").auto_shrink([false, false]).show(ui, |ui| match self.nav {
            NavTab::Scenes => self.nav_scenes(ui),
            NavTab::Characters => self.nav_characters(ui),
            NavTab::Locations => self.nav_locations(ui),
            NavTab::Notes => self.nav_notes(ui),
            NavTab::Goals => self.nav_goals(ui),
        });
    }

    fn nav_scenes(&mut self, ui: &mut egui::Ui) {
        if self.layout.scenes.is_empty() {
            ui.label(RichText::new("No scenes yet. Start a line with INT. or EXT.").weak());
        }
        let current = self.layout.scenes.iter().rposition(|s| s.source_line.is_some_and(|l| l <= self.cursor_line));
        let mut jump = None;
        for (i, sc) in self.layout.scenes.iter().enumerate() {
            let label = format!(
                "{:>3}  {}\n      p.{} · {}",
                sc.number.clone().unwrap_or_else(|| (i + 1).to_string()),
                sc.heading,
                self.layout.page_number(sc.page),
                format_eighths(sc.eighths)
            );
            if ui.selectable_label(current == Some(i), RichText::new(label).monospace()).clicked() {
                jump = Some((sc.source_line, sc.page));
            }
        }
        if let Some((line, page)) = jump {
            if let Some(l) = line {
                self.jump_to_line(l);
            }
            self.scroll_to_page = Some(page);
        }
    }

    fn nav_characters(&mut self, ui: &mut egui::Ui) {
        if self.report.characters.is_empty() {
            ui.label(RichText::new("No speaking characters yet.").weak());
            return;
        }
        let accent = palette(self.settings.theme).accent;
        let mut select = None;
        for c in &self.report.characters {
            let selected = self.selected_character.as_deref() == Some(c.name.as_str());
            ui.horizontal(|ui| {
                if ui.selectable_label(selected, RichText::new(&c.name).strong()).clicked() {
                    select = Some(if selected { None } else { Some(c.name.clone()) });
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.0}%", c.dialogue_share)).weak());
                });
            });
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, accent.gamma_multiply(0.15));
            let w = rect.width() * c.dialogue_share / 100.0;
            ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 4.0)), 2.0, accent);
            ui.label(
                RichText::new(format!(
                    "{} speeches · {} words · {} scenes · {} pages on screen",
                    c.speeches,
                    c.words,
                    c.scenes,
                    format_eighths(c.eighths)
                ))
                .small()
                .weak(),
            );
            if selected {
                ui.indent("char-detail", |ui| {
                    ui.label(RichText::new("Appears in:").small());
                    for (i, sc) in self.report.scenes.iter().enumerate() {
                        if sc.characters.contains(&c.name)
                            && ui.link(format!("{}. {}", i + 1, sc.heading)).clicked()
                            && let Some(l) = sc.source_line
                        {
                            self.pending_cursor = Some(CCursorRange::one(CCursor::new(line_start_char(&self.text, l))));
                            self.reveal_cursor = true;
                            self.view = View::Script;
                        }
                    }
                    let with: Vec<String> = self
                        .report
                        .interactions
                        .iter()
                        .filter_map(|(a, b, n)| {
                            if *a == c.name {
                                Some(format!("{b} ({n})"))
                            } else if *b == c.name {
                                Some(format!("{a} ({n})"))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !with.is_empty() {
                        ui.label(RichText::new(format!("Shares scenes with: {}", with.join(", "))).small());
                    }
                });
            }
            ui.add_space(6.0);
        }
        if let Some(s) = select {
            self.selected_character = s;
        }
    }

    fn nav_locations(&mut self, ui: &mut egui::Ui) {
        if self.report.locations.is_empty() {
            ui.label(RichText::new("No locations yet.").weak());
        }
        let s = &self.report.stats;
        ui.label(
            RichText::new(format!("INT {} · EXT {} · DAY {} · NIGHT {}", s.interior, s.exterior, s.day, s.night))
                .weak(),
        );
        ui.add_space(4.0);
        for l in &self.report.locations {
            ui.label(RichText::new(&l.name).strong());
            ui.label(
                RichText::new(format!(
                    "{} scene(s) · {} pages · {}",
                    l.scenes,
                    format_eighths(l.eighths),
                    l.times.join(", ")
                ))
                .small()
                .weak(),
            );
            ui.add_space(4.0);
        }
    }

    fn nav_notes(&mut self, ui: &mut egui::Ui) {
        if self.script.notes.is_empty() {
            ui.label(RichText::new("No notes. Add one anywhere with [[your note]].").weak());
        }
        let mut jump = None;
        for n in &self.script.notes {
            if ui.selectable_label(false, format!("Ln {}: {}", n.line + 1, n.text)).clicked() {
                jump = Some(n.line);
            }
        }
        if let Some(l) = jump {
            self.jump_to_line(l);
        }
    }

    fn nav_goals(&mut self, ui: &mut egui::Ui) {
        let p = palette(self.settings.theme);
        let goal = self.settings.daily_goal.max(1);
        let today = self.stats.today_words();
        ui.heading("Today");
        ui.add(egui::ProgressBar::new((today as f32 / goal as f32).min(1.0)).text(format!("{today} / {goal} words")));
        ui.horizontal(|ui| {
            ui.label("Daily goal");
            ui.add(egui::DragValue::new(&mut self.settings.daily_goal).range(50..=20000).speed(10));
        });
        ui.label(format!("This session: {} words", self.stats.session_words));
        let streak = self.stats.streak(goal);
        ui.label(format!("Streak: {streak} day{}", if streak == 1 { "" } else { "s" }));

        ui.add_space(6.0);
        ui.label(RichText::new("Last 14 days").strong());
        let days = self.stats.last_days(14);
        let max = days.iter().map(|d| d.1).max().unwrap_or(0).max(goal) as f32;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 70.0), Sense::hover());
        let bw = rect.width() / days.len() as f32;
        let goal_y = rect.bottom() - rect.height() * goal as f32 / max;
        for (i, (_, n)) in days.iter().enumerate() {
            let h = rect.height() * *n as f32 / max;
            let r = Rect::from_min_max(
                Pos2::new(rect.left() + i as f32 * bw + 2.0, rect.bottom() - h),
                Pos2::new(rect.left() + (i + 1) as f32 * bw - 2.0, rect.bottom()),
            );
            let c = if *n >= goal { p.accent } else { p.accent.gamma_multiply(0.45) };
            ui.painter().rect_filled(r, 2.0, c);
        }
        ui.painter().hline(rect.x_range(), goal_y, Stroke::new(1.0, p.transition));

        ui.add_space(8.0);
        ui.heading("Writing Sprint");
        match &self.sprint {
            None => {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.sprint_minutes).range(1..=180).suffix(" min"));
                    if ui.button("▶ Start").clicked() {
                        self.sprint = Some(Sprint {
                            start: Instant::now(),
                            minutes: self.sprint_minutes,
                            start_words: self.word_count,
                        });
                        self.focus_mode = true;
                    }
                });
            }
            Some(s) => {
                let left = (s.minutes * 60).saturating_sub(s.start.elapsed().as_secs());
                ui.label(format!(
                    "{:02}:{:02} left · {} words",
                    left / 60,
                    left % 60,
                    self.word_count.saturating_sub(s.start_words)
                ));
                if ui.button("■ Stop").clicked() {
                    self.sprint = None;
                }
            }
        }

        ui.add_space(8.0);
        ui.heading("Script");
        let s = &self.report.stats;
        egui::Grid::new("script-stats").num_columns(2).show(ui, |ui| {
            for (k, v) in [
                ("Pages", s.pages.to_string()),
                ("Scenes", s.scenes.to_string()),
                ("Running time", format!("~{:.0} min", s.est_minutes)),
                ("Words", s.words.to_string()),
                ("Dialogue", format!("{}%", s.dialogue_words * 100 / s.words.max(1))),
                ("Speaking roles", s.speaking_characters.to_string()),
            ] {
                ui.label(k);
                ui.label(RichText::new(v).strong());
                ui.end_row();
            }
        });
    }

    // ------------------------------------------------------------ UI: editor

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.find_query).hint_text("Find").desired_width(180.0));
            if std::mem::take(&mut self.find_focus) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                self.find_next();
                r.request_focus();
            }
            ui.add(egui::TextEdit::singleline(&mut self.replace_text).hint_text("Replace with").desired_width(160.0));
            ui.checkbox(&mut self.find_case, "Aa");
            if ui.button("Next").clicked() {
                self.find_next();
            }
            if ui.button("Replace").clicked() {
                self.replace_one();
            }
            if ui.button("Replace All").clicked() {
                self.replace_all();
            }
            if ui.button("✖").clicked() {
                self.find_open = false;
            }
        });
        ui.separator();
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        if self.find_open {
            self.find_bar(ui);
        }
        let ctx = ui.ctx().clone();
        let id = Id::new(EDITOR_ID);
        let p = palette(self.settings.theme);
        let font = FontId::monospace(self.settings.font_size);

        // SmartType keys must be taken before the TextEdit sees them.
        let editor_focused = ctx.memory(|m| m.has_focus(id));
        if editor_focused && !self.suggestions.is_empty() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                self.apply_suggestion(0);
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                let line = self.text.split('\n').nth(self.cursor_line).unwrap_or("").to_string();
                self.dismissed = Some((self.cursor_line, line));
                self.suggestions.clear();
            }
        }

        if let Some(range) = self.pending_cursor.take() {
            let mut st = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
            st.cursor.set_char_range(Some(range));
            st.store(&ctx, id);
            ctx.memory_mut(|m| m.request_focus(id));
        }

        let col_w = (self.settings.font_size * 0.6 * 66.0).min(ui.available_width() - 16.0).max(200.0);
        let mut area = ScrollArea::vertical().id_salt("editor-scroll").auto_shrink([false, false]);
        if let Some(off) = self.pending_scroll.take() {
            area = area.vertical_scroll_offset(off.max(0.0));
        }
        let typewriter = self.settings.typewriter;
        let viewport_h = self.viewport_h;
        let cache = &mut self.hl_cache;
        let text = &mut self.text;
        let out = area.show(ui, |ui| {
            let content_top = ui.min_rect().top();
            let margin = ((ui.available_width() - col_w) / 2.0).max(0.0);
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                let s = buf.as_str();
                if cache.0 != s {
                    cache.0 = s.to_string();
                    cache.1 = parse(s).line_kinds;
                }
                let job = highlight::highlight(s, &cache.1, &p, font.clone(), wrap);
                ui.painter().layout_job(job)
            };
            let output = ui
                .horizontal_top(|ui| {
                    ui.add_space(margin);
                    egui::TextEdit::multiline(text)
                        .id(id)
                        .font(FontId::monospace(self.settings.font_size))
                        .desired_width(col_w)
                        .desired_rows(30)
                        .lock_focus(true)
                        .frame(egui::Frame::NONE)
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner;
            if typewriter {
                ui.add_space(viewport_h / 2.0);
            }
            (output, content_top)
        });
        self.viewport_h = out.inner_rect.height();
        let (output, content_top) = out.inner;

        let changed = output.response.changed();
        if changed {
            self.on_text_changed();
        }
        let old_cursor = self.cursor;
        if let Some(r) = output.cursor_range {
            self.cursor = r.primary.index.0;
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            self.selection = (a.min(b), a.max(b));
        }
        let moved = self.cursor != old_cursor;
        if moved || changed {
            self.cursor_line = line_of_char(&self.text, self.cursor);
            self.update_suggestions();
        }

        // Reveal / centre the cursor (typewriter mode) using this frame's galley.
        let galley: &Arc<Galley> = &output.galley;
        let galley_top = output.galley_pos.y - content_top;
        let cur_rect = galley.pos_from_cursor(CCursor::new(self.cursor));
        if self.reveal_cursor || (typewriter && (moved || changed)) {
            let anchor = if typewriter { self.viewport_h / 2.0 } else { self.viewport_h * 0.25 };
            self.pending_scroll = Some(galley_top + cur_rect.center().y - anchor);
            self.reveal_cursor = false;
            ctx.request_repaint();
        }

        // SmartType popup just below the cursor.
        if output.response.response.has_focus() && !self.suggestions.is_empty() {
            let pos = output.galley_pos + cur_rect.left_bottom().to_vec2() + Vec2::new(0.0, 4.0);
            let mut chosen = None;
            egui::Area::new(Id::new("smarttype")).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    for (i, s) in self.suggestions.iter().enumerate() {
                        let label =
                            RichText::new(s.trim_end()).monospace().color(if i == 0 { p.accent } else { p.text });
                        if ui.selectable_label(i == 0, label).clicked() {
                            chosen = Some(i);
                        }
                    }
                    ui.label(RichText::new("Tab to accept · Esc to dismiss").small().weak());
                });
            });
            if let Some(i) = chosen {
                self.apply_suggestion(i);
            }
        }
    }

    // ------------------------------------------------------------ UI: index cards

    fn cards(&mut self, ui: &mut egui::Ui) {
        let p = palette(self.settings.theme);
        if self.layout.scenes.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No scenes yet — write a scene heading (INT. / EXT.) to create a card.")
            });
            return;
        }
        let card = Vec2::new(250.0, 165.0);
        let mut action: Option<(usize, isize)> = None;
        let mut open: Option<usize> = None;
        ScrollArea::vertical().id_salt("cards").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Index cards — click a card to open the scene, use ◀ ▶ to reorder scenes in the script.")
                    .weak(),
            );
            ui.add_space(6.0);
            let gap = 12.0;
            let cols = (((ui.available_width() + gap) / (card.x + gap)).floor() as usize).max(1);
            let mut row_rect = Rect::NOTHING;
            {
                for (i, sc) in self.layout.scenes.iter().enumerate() {
                    if i % cols == 0 {
                        row_rect =
                            ui.allocate_exact_size(Vec2::new(ui.available_width(), card.y + gap), Sense::hover()).0;
                    }
                    let col = (i % cols) as f32;
                    let rect = Rect::from_min_size(row_rect.min + Vec2::new(col * (card.x + gap), 0.0), card);
                    let resp = ui.interact(rect, Id::new(("card", i)), Sense::click());
                    let (_, _, time) = report::split_heading(&sc.heading);
                    let stripe = if time.contains("NIGHT") {
                        p.heading
                    } else if time.contains("DAY") {
                        p.transition
                    } else {
                        p.section
                    };
                    let hovered = resp.hovered();
                    ui.painter().rect_filled(rect, 6.0, if hovered { p.page.gamma_multiply(0.92) } else { p.page });
                    ui.painter().rect_stroke(
                        rect,
                        6.0,
                        Stroke::new(1.0, p.comment.gamma_multiply(0.6)),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width(), 5.0)), 3.0, stripe);
                    let mut child = ui.new_child(
                        UiBuilder::new()
                            .max_rect(rect.shrink2(Vec2::new(10.0, 10.0)))
                            .layout(egui::Layout::top_down(Align::Min)),
                    );
                    child.set_clip_rect(rect);
                    child.add_space(2.0);
                    let inner_w = child.available_width();
                    child.allocate_ui_with_layout(
                        Vec2::new(inner_w, 20.0),
                        egui::Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.label(
                                RichText::new(sc.number.clone().unwrap_or_else(|| (i + 1).to_string()))
                                    .strong()
                                    .color(stripe),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "p.{} · {}",
                                    self.layout.page_number(sc.page),
                                    format_eighths(sc.eighths)
                                ))
                                .small()
                                .color(p.parenthetical),
                            );
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("▶").on_hover_text("Move later").clicked() {
                                    action = Some((i, 1));
                                }
                                if ui.small_button("◀").on_hover_text("Move earlier").clicked() {
                                    action = Some((i, -1));
                                }
                            });
                        },
                    );
                    child.label(RichText::new(&sc.heading).strong().color(p.heading));
                    let body = self.card_text.get(i).map(String::as_str).unwrap_or("");
                    child.label(RichText::new(truncate(body, 150)).color(p.ink));
                    if let Some(r) = self.report.scenes.get(i)
                        && !r.characters.is_empty()
                    {
                        child.label(RichText::new(r.characters.join(", ")).small().color(p.character));
                    }
                    if resp.clicked() {
                        open = Some(i);
                    }
                }
            }
        });
        if let Some((i, d)) = action {
            self.move_scene(i, d);
        } else if let Some(i) = open
            && let Some(l) = self.layout.scenes[i].source_line
        {
            self.scroll_to_page = Some(self.layout.scenes[i].page);
            self.jump_to_line(l);
        }
    }

    // ------------------------------------------------------------ UI: preview

    fn preview(&mut self, ui: &mut egui::Ui) {
        let p = palette(self.settings.theme);
        let scroll_to = self.scroll_to_page.take();
        let watermark = self.settings.watermark.trim().to_string();
        ScrollArea::vertical().id_salt("preview").auto_shrink([false, false]).show(ui, |ui| {
            let ppi = ((ui.available_width() - 16.0) / PAGE_WIDTH).clamp(30.0, 140.0);
            let size = Vec2::new(PAGE_WIDTH * ppi, PAGE_HEIGHT * ppi);
            ui.vertical_centered(|ui| {
                for (i, page) in self.layout.pages.iter().enumerate() {
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    if scroll_to == Some(i) {
                        ui.scroll_to_rect(rect, Some(Align::TOP));
                    }
                    if ui.is_rect_visible(rect) {
                        draw_page(ui.painter(), rect, page, ppi, &p, &watermark);
                    }
                    ui.add_space(12.0);
                }
            });
        });
    }

    // ------------------------------------------------------------ UI: windows

    fn windows(&mut self, ctx: &egui::Context) {
        let mut open = self.export_open;
        egui::Window::new("Export").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.heading("Formatting");
            ui.checkbox(&mut self.settings.number_scenes, "Number scenes automatically");
            ui.checkbox(&mut self.settings.bold_headings, "Bold scene headings");
            ui.add_space(6.0);
            ui.heading("PDF");
            ui.horizontal(|ui| {
                ui.label("Watermark");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.watermark).hint_text("e.g. DRAFT or recipient name"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Colours");
                egui::ComboBox::from_id_salt("pdf-colors").selected_text(self.settings.pdf_colors.name()).show_ui(
                    ui,
                    |ui| {
                        for c in PdfColors::ALL {
                            ui.selectable_value(&mut self.settings.pdf_colors, c, c.name());
                        }
                    },
                );
            });
            if self.settings.pdf_colors == PdfColors::Custom {
                ui.horizontal(|ui| {
                    ui.label("Paper");
                    egui::color_picker::color_edit_button_srgb(ui, &mut self.settings.custom_paper);
                    ui.label("Ink");
                    egui::color_picker::color_edit_button_srgb(ui, &mut self.settings.custom_ink);
                });
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("📄 PDF…").clicked() {
                    self.export("pdf");
                }
                if ui.button("🎬 Final Draft (.fdx)…").clicked() {
                    self.export("fdx");
                }
                if ui.button("⛲ Fountain…").clicked() {
                    self.export("fountain");
                }
                if ui.button("📊 Reports (.txt)…").clicked() {
                    self.export("report");
                }
            });
        });
        self.export_open = open;

        let mut open = self.backups_open;
        let mut restore = None;
        egui::Window::new("Backups").open(&mut open).default_width(420.0).show(ctx, |ui| {
            ui.label(format!(
                "Automatic backups every {} min while you write. Restoring replaces the editor text (you can still undo by not saving).",
                self.settings.backup_minutes
            ));
            ui.horizontal(|ui| {
                ui.label("Interval");
                ui.add(egui::DragValue::new(&mut self.settings.backup_minutes).range(1..=120).suffix(" min"));
            });
            ui.separator();
            if self.backups.is_empty() {
                ui.label(RichText::new("No backups yet for this script.").weak());
            }
            ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for b in &self.backups {
                    ui.horizontal(|ui| {
                        ui.label(b.file_name().unwrap_or_default().to_string_lossy());
                        if ui.button("Restore").clicked() {
                            restore = Some(b.clone());
                        }
                    });
                }
            });
        });
        self.backups_open = open;
        if let Some(b) = restore {
            self.backups_open = false;
            self.request(Pending::Restore(b));
        }

        egui::Window::new("Fountain Syntax").open(&mut self.help_open).default_width(460.0).show(ctx, |ui| {
            ui.label(RichText::new(HELP).monospace());
        });

        if let Some(action) = self.confirm.clone() {
            let mut decided = None;
            egui::Modal::new(Id::new("confirm")).show(ctx, |ui| {
                ui.heading("Unsaved changes");
                ui.label(format!("Save changes to {} first?", self.file_name()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Don't Save").clicked() {
                        decided = Some(false);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm = None;
                    }
                });
            });
            if let Some(save) = decided {
                self.confirm = None;
                if save {
                    self.save();
                }
                if !save || !self.dirty {
                    self.dirty = false;
                    self.perform(action.clone());
                    if action == Pending::Quit {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
    }
}

const HELP: &str = "\
INT. HOUSE - DAY        Scene heading (INT/EXT/EST/I/E)
.FLASHBACK              Forced scene heading
INT. HOUSE - DAY #12#   Scene number

MARY                    Character (all caps, blank line before)
MARY (V.O.)             Extension
@McCLANE                Forced character
(quietly)               Parenthetical
BOB ^                   Dual dialogue (second speaker)

CUT TO:                 Transition (caps, ends in TO:)
> FADE OUT.             Forced transition
> THE END <             Centered
!LOUD NOISES            Forced action
~Sung lyric             Lyrics
===                     Page break

# Act One               Section (outline only)
= Synopsis              Synopsis (shown on index cards)
[[note]]                Note (Notes tab)
/* ... */               Boneyard (hidden)

*italic*  **bold**  ***bold italic***  _underline_

Title page (top of file):
Title: / Credit: / Author: / Source: / Draft date: / Contact:";

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>().trim_end())
    }
}

/// Per scene: its first synopsis, or else its first action paragraph.
fn card_texts(script: &Screenplay) -> Vec<String> {
    let mut out: Vec<(Option<String>, Option<String>)> = Vec::new();
    for el in &script.elements {
        match el {
            Element::SceneHeading { .. } => out.push((None, None)),
            Element::Synopsis(s) => {
                if let Some(last) = out.last_mut()
                    && last.0.is_none()
                {
                    last.0 = Some(s.clone());
                }
            }
            Element::Action(lines) => {
                if let Some(last) = out.last_mut()
                    && last.1.is_none()
                {
                    last.1 = Some(lines.iter().map(plain).collect::<Vec<_>>().join(" "));
                }
            }
            _ => {}
        }
    }
    out.into_iter().map(|(s, a)| s.or(a).unwrap_or_default()).collect()
}

/// Paint one page. egui's monospace font has Courier's 0.6em advance, so a font
/// size of 1/6 inch gives exactly 10 characters per inch.
fn draw_page(painter: &egui::Painter, rect: Rect, page: &layout::Page, ppi: f32, p: &Palette, watermark: &str) {
    painter.rect_filled(rect, 2.0, p.page);
    if !watermark.is_empty() {
        let len = watermark.chars().count().max(1) as f32;
        let size = (rect.width() * 1.1 / (len * 0.62)).min(rect.width() * 0.22);
        let galley =
            painter.layout_no_wrap(watermark.to_string(), FontId::proportional(size), p.ink.gamma_multiply(0.10));
        let offset = galley.size() / 2.0;
        let angle = -std::f32::consts::FRAC_PI_4;
        // Rotate around the galley centre so it sits in the middle of the page.
        let rot = egui::emath::Rot2::from_angle(angle);
        let pos = rect.center() - rot * offset;
        painter.add(egui::epaint::TextShape::new(pos, galley, p.ink).with_angle(angle));
    }
    let size = ppi / 6.0;
    for item in &page.items {
        let mut x = rect.left() + item.x * ppi;
        let y = rect.top() + item.y * ppi;
        for span in &item.spans {
            let mut f = TextFormat::simple(FontId::monospace(size), p.ink);
            f.italics = span.style.italic;
            if span.style.underline {
                f.underline = Stroke::new((ppi / 96.0).max(0.8), p.ink);
            }
            let job = egui::text::LayoutJob::single_section(span.text.clone(), f);
            let galley = painter.layout_job(job);
            if span.style.bold {
                painter.galley(Pos2::new(x + (ppi / 160.0).max(0.5), y), galley.clone(), p.ink);
            }
            painter.galley(Pos2::new(x, y), galley, p.ink);
            x += span.text.chars().count() as f32 * CHAR_WIDTH * ppi;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != Some(self.settings.theme) {
            ctx.set_visuals(highlight::visuals(self.settings.theme));
            self.applied_theme = Some(self.settings.theme);
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Pending::Quit);
        }
        self.handle_shortcuts(&ctx);
        self.update_window_title(&ctx);
        self.timers(&ctx);
        let p = palette(self.settings.theme);

        if !self.focus_mode {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
            if self.settings.show_navigator {
                egui::Panel::left("navigator").resizable(true).default_size(280.0).show(ui, |ui| self.navigator(ui));
            }
            if self.settings.show_preview && self.view == View::Script {
                egui::Panel::right("preview")
                    .resizable(true)
                    .default_size(ctx.content_rect().width() * 0.36)
                    .frame(egui::Frame::new().fill(p.desk).inner_margin(8.0))
                    .show(ui, |ui| self.preview(ui));
            }
        } else {
            egui::Panel::bottom("focus-hint").frame(egui::Frame::new().fill(p.editor_bg).inner_margin(4.0)).show(
                ui,
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} words today · Esc or Ctrl+Shift+F to leave focus mode",
                                self.stats.today_words()
                            ))
                            .small()
                            .color(p.comment),
                        );
                        if let Some(s) = &self.sprint {
                            let left = (s.minutes * 60).saturating_sub(s.start.elapsed().as_secs());
                            ui.label(
                                RichText::new(format!("⏱ {:02}:{:02}", left / 60, left % 60)).small().color(p.accent),
                            );
                        }
                    });
                },
            );
        }

        let fill = if self.view == View::Cards { p.desk } else { p.editor_bg };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill).inner_margin(8.0)).show(ui, |ui| match self
            .view
        {
            View::Script => self.editor(ui),
            View::Cards => self.cards(ui),
        });

        self.windows(&ctx);
        self.screenshot_step(&ctx);
    }
}

impl App {
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some((path, frame)) = &mut self.screenshot else { return };
        *frame += 1;
        if *frame == 40 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            for px in &img.pixels {
                ppm.extend_from_slice(&[px.r(), px.g(), px.b()]);
            }
            let _ = std::fs::write(&*path, ppm);
            self.dirty = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_helpers() {
        let s = "ab\nçd\n\nxyz";
        assert_eq!(line_start_char(s, 0), 0);
        assert_eq!(line_start_char(s, 1), 3);
        assert_eq!(line_start_char(s, 3), 7);
        assert_eq!(line_of_char(s, 4), 1);
        assert_eq!(byte_of_char(s, 4), 5);
        assert_eq!(char_of_byte(s, 5), 4);
    }

    #[test]
    fn card_text_prefers_synopsis() {
        let s = parse("INT. A - DAY\n\nSome action.\n\n= The point.\n\nEXT. B - NIGHT\n\nRain.\n");
        assert_eq!(card_texts(&s), vec!["The point.".to_string(), "Rain.".to_string()]);
    }

    #[test]
    fn move_scene_reorders_text() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, None);
        app.path = None;
        app.text =
            "FADE IN:\n\nINT. A - DAY\n\nAlpha.\n\nEXT. B - NIGHT\n\nBravo.\n= note\n\nINT. C - DAY\n\nCharlie.".into();
        app.reflow();
        app.move_scene(2, -1);
        assert_eq!(
            app.text,
            "FADE IN:\n\nINT. A - DAY\n\nAlpha.\n\nINT. C - DAY\n\nCharlie.\n\nEXT. B - NIGHT\n\nBravo.\n= note\n"
        );
        let headings: Vec<_> = app.layout.scenes.iter().map(|s| s.heading.clone()).collect();
        assert_eq!(headings, vec!["INT. A - DAY", "INT. C - DAY", "EXT. B - NIGHT"]);
        app.move_scene(0, -1); // no-op at the edge
        assert_eq!(app.layout.scenes[0].heading, "INT. A - DAY");
    }
}
