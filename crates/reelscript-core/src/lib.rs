//! ReelScript core: parse [Fountain](https://fountain.io) screenplays, paginate them
//! to the standard US screenplay format, export PDF, import/export Final Draft
//! `.fdx`, and produce production reports.
//!
//! ```
//! let script = reelscript_core::parse("INT. HOUSE - DAY\n\nA door creaks.\n");
//! let layout = reelscript_core::layout::paginate(&script);
//! let pdf = reelscript_core::pdf::render(&layout.pages, "Untitled");
//! assert!(pdf.starts_with(b"%PDF"));
//! ```

pub mod fdx;
pub mod fountain;
pub mod layout;
pub mod pdf;
pub mod report;

pub use fountain::{Element, Screenplay, parse};

/// Title for document metadata, from the title page if present.
pub fn document_title(script: &Screenplay) -> String {
    script
        .title_value("title")
        .map(|v| v.iter().map(|l| fountain::plain(&fountain::parse_inline(l))).collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Screenplay".to_string())
}

/// Parse Fountain source and render it straight to PDF bytes.
pub fn fountain_to_pdf(source: &str) -> Vec<u8> {
    let script = parse(source);
    let layout = layout::paginate(&script);
    pdf::render(&layout.pages, &document_title(&script))
}

/// Load a document as Fountain text: `.fdx` files are converted, anything else is
/// returned as-is.
pub fn load_as_fountain(path: &std::path::Path) -> std::io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    let is_fdx = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fdx"));
    if is_fdx {
        let script = fdx::import(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(fountain::write(&script))
    } else {
        Ok(text)
    }
}
