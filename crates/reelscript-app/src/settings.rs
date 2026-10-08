//! Persistent settings, writing statistics/goals, and automatic backups.
//!
//! Stored as small `key=value` text files so there are no extra dependencies:
//! - `~/.config/reelscript/settings.txt`
//! - `~/.config/reelscript/stats.txt` (one `YYYY-MM-DD words` line per day)
//! - `~/.local/share/reelscript/backups/<script>/<script>-<timestamp>.fountain`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{Days, Local, NaiveDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    Light,
    Night,
    Midnight,
}

impl AppTheme {
    pub const ALL: [AppTheme; 3] = [AppTheme::Light, AppTheme::Night, AppTheme::Midnight];

    pub fn name(self) -> &'static str {
        match self {
            AppTheme::Light => "Light",
            AppTheme::Night => "Night",
            AppTheme::Midnight => "Midnight",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfColors {
    Paper,
    Night,
    Midnight,
    Custom,
}

impl PdfColors {
    pub const ALL: [PdfColors; 4] = [PdfColors::Paper, PdfColors::Night, PdfColors::Midnight, PdfColors::Custom];

    pub fn name(self) -> &'static str {
        match self {
            PdfColors::Paper => "Paper (white)",
            PdfColors::Night => "Night",
            PdfColors::Midnight => "Midnight",
            PdfColors::Custom => "Custom",
        }
    }

    fn key(self) -> &'static str {
        match self {
            PdfColors::Paper => "paper",
            PdfColors::Night => "night",
            PdfColors::Midnight => "midnight",
            PdfColors::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: AppTheme,
    pub font_size: f32,
    pub typewriter: bool,
    pub show_preview: bool,
    pub show_navigator: bool,
    pub smart_type: bool,
    pub number_scenes: bool,
    pub bold_headings: bool,
    pub watermark: String,
    pub pdf_colors: PdfColors,
    pub custom_paper: [u8; 3],
    pub custom_ink: [u8; 3],
    pub daily_goal: usize,
    pub backup_minutes: u64,
    pub autosave: bool,
    pub recent: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: AppTheme::Night,
            font_size: 15.0,
            typewriter: false,
            show_preview: true,
            show_navigator: true,
            smart_type: true,
            number_scenes: false,
            bold_headings: false,
            watermark: String::new(),
            pdf_colors: PdfColors::Paper,
            custom_paper: [250, 246, 235],
            custom_ink: [30, 30, 60],
            daily_goal: 500,
            backup_minutes: 5,
            autosave: false,
            recent: Vec::new(),
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".config")))
        .map(|d| d.join("reelscript"))
}

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/share")))
        .map(|d| d.join("reelscript"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Self::default();
        let Some(text) = config_dir().and_then(|d| std::fs::read_to_string(d.join("settings.txt")).ok()) else {
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let b = v == "true";
            match k {
                "theme" => s.theme = AppTheme::parse(v).unwrap_or(s.theme),
                "font_size" => s.font_size = v.parse().unwrap_or(s.font_size).clamp(9.0, 32.0),
                "typewriter" => s.typewriter = b,
                "show_preview" => s.show_preview = b,
                "show_navigator" => s.show_navigator = b,
                "smart_type" => s.smart_type = b,
                "number_scenes" => s.number_scenes = b,
                "bold_headings" => s.bold_headings = b,
                "watermark" => s.watermark = v.to_string(),
                "pdf_colors" => {
                    s.pdf_colors = PdfColors::ALL.into_iter().find(|p| p.key() == v).unwrap_or(s.pdf_colors)
                }
                "custom_paper" => s.custom_paper = reelscript_core::pdf::Theme::parse_hex(v).unwrap_or(s.custom_paper),
                "custom_ink" => s.custom_ink = reelscript_core::pdf::Theme::parse_hex(v).unwrap_or(s.custom_ink),
                "daily_goal" => s.daily_goal = v.parse().unwrap_or(s.daily_goal),
                "backup_minutes" => s.backup_minutes = v.parse().unwrap_or(s.backup_minutes),
                "autosave" => s.autosave = b,
                "recent" => s.recent.push(PathBuf::from(v)),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(dir) = config_dir() else { return Ok(()) };
        std::fs::create_dir_all(&dir)?;
        let mut o = String::new();
        let mut kv = |k: &str, v: String| o.push_str(&format!("{k}={v}\n"));
        kv("theme", self.theme.name().into());
        kv("font_size", self.font_size.to_string());
        kv("typewriter", self.typewriter.to_string());
        kv("show_preview", self.show_preview.to_string());
        kv("show_navigator", self.show_navigator.to_string());
        kv("smart_type", self.smart_type.to_string());
        kv("number_scenes", self.number_scenes.to_string());
        kv("bold_headings", self.bold_headings.to_string());
        kv("watermark", self.watermark.replace('\n', " "));
        kv("pdf_colors", self.pdf_colors.key().into());
        kv("custom_paper", hex(self.custom_paper));
        kv("custom_ink", hex(self.custom_ink));
        kv("daily_goal", self.daily_goal.to_string());
        kv("backup_minutes", self.backup_minutes.to_string());
        kv("autosave", self.autosave.to_string());
        for r in &self.recent {
            kv("recent", r.display().to_string());
        }
        std::fs::write(dir.join("settings.txt"), o)
    }

    pub fn add_recent(&mut self, p: &Path) {
        let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        self.recent.retain(|r| *r != p);
        self.recent.insert(0, p);
        self.recent.truncate(10);
    }

    pub fn pdf_theme(&self) -> reelscript_core::pdf::Theme {
        use reelscript_core::pdf::Theme;
        match self.pdf_colors {
            PdfColors::Paper => Theme::PAPER,
            PdfColors::Night => Theme::NIGHT,
            PdfColors::Midnight => Theme::MIDNIGHT,
            PdfColors::Custom => Theme { paper: self.custom_paper, ink: self.custom_ink },
        }
    }
}

/// Words written per day, plus the current session.
#[derive(Debug, Default)]
pub struct WritingStats {
    pub days: BTreeMap<NaiveDate, usize>,
    pub session_words: usize,
    dirty: bool,
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

impl WritingStats {
    pub fn load() -> Self {
        let mut s = Self::default();
        if let Some(text) = config_dir().and_then(|d| std::fs::read_to_string(d.join("stats.txt")).ok()) {
            for line in text.lines() {
                if let Some((d, n)) = line.split_once(' ')
                    && let (Ok(d), Ok(n)) = (NaiveDate::parse_from_str(d, "%Y-%m-%d"), n.trim().parse())
                {
                    s.days.insert(d, n);
                }
            }
        }
        s
    }

    /// Record a change in total word count. Only additions count as "written".
    pub fn record(&mut self, delta: isize) {
        if delta > 0 {
            let d = delta as usize;
            *self.days.entry(today()).or_default() += d;
            self.session_words += d;
            self.dirty = true;
        }
    }

    pub fn today_words(&self) -> usize {
        self.days.get(&today()).copied().unwrap_or(0)
    }

    /// Consecutive days (ending today or yesterday) that met `goal`.
    pub fn streak(&self, goal: usize) -> usize {
        let goal = goal.max(1);
        let mut day = today();
        if self.days.get(&day).copied().unwrap_or(0) < goal {
            day = day - Days::new(1);
        }
        let mut n = 0;
        while self.days.get(&day).copied().unwrap_or(0) >= goal {
            n += 1;
            day = day - Days::new(1);
        }
        n
    }

    /// The last `n` days, oldest first.
    pub fn last_days(&self, n: u64) -> Vec<(NaiveDate, usize)> {
        let t = today();
        (0..n).rev().map(|i| t - Days::new(i)).map(|d| (d, self.days.get(&d).copied().unwrap_or(0))).collect()
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(dir) = config_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let text: String = self.days.iter().map(|(d, n)| format!("{} {n}\n", d.format("%Y-%m-%d"))).collect();
        if std::fs::write(dir.join("stats.txt"), text).is_ok() {
            self.dirty = false;
        }
    }
}

pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

// ---------------------------------------------------------------- backups

const KEEP_BACKUPS: usize = 30;

fn backup_dir(script: Option<&Path>) -> Option<(PathBuf, String)> {
    let stem = script
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".into());
    let safe: String =
        stem.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    data_dir().map(|d| (d.join("backups").join(&safe), safe))
}

/// Write a timestamped backup and prune old ones. Returns the path written.
pub fn write_backup(script: Option<&Path>, text: &str) -> std::io::Result<PathBuf> {
    let (dir, stem) = backup_dir(script).ok_or_else(|| std::io::Error::other("no home directory"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{stem}-{}.fountain", Local::now().format("%Y%m%d-%H%M%S")));
    std::fs::write(&path, text)?;
    let mut all = list_backups(script);
    while all.len() > KEEP_BACKUPS {
        if let Some(old) = all.pop() {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(path)
}

/// Backups for a script, newest first.
pub fn list_backups(script: Option<&Path>) -> Vec<PathBuf> {
    let Some((dir, _)) = backup_dir(script) else { return vec![] };
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "fountain"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_and_record() {
        let mut s = WritingStats::default();
        let t = today();
        s.days.insert(t - Days::new(1), 600);
        s.days.insert(t - Days::new(2), 700);
        s.days.insert(t - Days::new(4), 900);
        assert_eq!(s.streak(500), 2);
        s.record(520);
        s.record(-100);
        assert_eq!(s.today_words(), 520);
        assert_eq!(s.streak(500), 3);
        assert_eq!(s.last_days(3).len(), 3);
    }
}
