use std::path::PathBuf;
use std::process::ExitCode;

use reelscript_core::layout::{self, LayoutOptions, format_eighths};
use reelscript_core::pdf::{self, PdfOptions, Theme};
use reelscript_core::{document_title, fdx, load_as_fountain, parse, report};

const USAGE: &str = "\
Usage: reelscript <input.fountain|input.fdx> [options]

Output (default: PDF next to the input):
  -o, --output <file>     Output path. Format follows the extension:
                          .pdf, .fdx (Final Draft) or .fountain
      --fdx               Write Final Draft .fdx instead of PDF
      --fountain          Write Fountain text (e.g. to convert .fdx -> .fountain)
      --outline           Print scenes with page numbers and lengths
      --report            Print statistics, scene, character, location and notes reports

PDF options:
      --number-scenes     Number every scene automatically
      --bold-headings     Print scene headings in bold
      --watermark <text>  Diagonal watermark on every page (e.g. \"DRAFT\" or a name)
      --theme <name>      paper (default), night, midnight
      --paper <#rrggbb>   Custom page colour
      --ink <#rrggbb>     Custom text colour

  -h, --help              Show this help";

#[derive(PartialEq)]
enum Format {
    Pdf,
    Fdx,
    Fountain,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut format: Option<Format> = None;
    let (mut outline, mut want_report) = (false, false);
    let mut lopts = LayoutOptions::default();
    let mut popts = PdfOptions::default();

    let mut args = std::env::args_os().skip(1);
    let value = |args: &mut std::iter::Skip<std::env::ArgsOs>, flag: &str| -> Result<String, String> {
        args.next().and_then(|v| v.into_string().ok()).ok_or_else(|| format!("missing value for {flag}"))
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return Ok(());
            }
            Some("-o" | "--output") => output = Some(value(&mut args, "--output")?.into()),
            Some("--fdx") => format = Some(Format::Fdx),
            Some("--fountain") => format = Some(Format::Fountain),
            Some("--outline") => outline = true,
            Some("--report") => want_report = true,
            Some("--number-scenes") => lopts.number_scenes = true,
            Some("--bold-headings") => lopts.bold_headings = true,
            Some("--watermark") => popts.watermark = Some(value(&mut args, "--watermark")?),
            Some("--theme") => {
                let v = value(&mut args, "--theme")?;
                popts.theme = Theme::by_name(&v).ok_or(format!("unknown theme {v} (paper, night, midnight)"))?;
            }
            Some("--paper") => {
                let v = value(&mut args, "--paper")?;
                popts.theme.paper = Theme::parse_hex(&v).ok_or(format!("bad colour {v}, expected #rrggbb"))?;
            }
            Some("--ink") => {
                let v = value(&mut args, "--ink")?;
                popts.theme.ink = Theme::parse_hex(&v).ok_or(format!("bad colour {v}, expected #rrggbb"))?;
            }
            Some(s) if s.starts_with('-') => return Err(format!("unknown option {s}\n\n{USAGE}")),
            _ if input.is_none() => input = Some(arg.into()),
            _ => return Err(format!("unexpected argument {}\n\n{USAGE}", arg.to_string_lossy())),
        }
    }
    let input = input.ok_or(USAGE)?;

    let source = load_as_fountain(&input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    let script = parse(&source);
    let layout = layout::paginate_with(&script, &lopts);

    if outline || want_report {
        if outline {
            for sc in &layout.scenes {
                println!(
                    "{:>4}  {:>4}  {:<7} {}",
                    sc.number.as_deref().unwrap_or(""),
                    layout.page_number(sc.page),
                    format_eighths(sc.eighths),
                    sc.heading
                );
            }
        }
        if want_report {
            print!("{}", report::build(&script, &layout).to_text());
        }
        return Ok(());
    }

    let format = format.unwrap_or_else(|| {
        match output.as_ref().and_then(|o| o.extension()).and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
            Some(e) if e == "fdx" => Format::Fdx,
            Some(e) if e == "fountain" || e == "spmd" => Format::Fountain,
            _ => Format::Pdf,
        }
    });
    let ext = match format {
        Format::Pdf => "pdf",
        Format::Fdx => "fdx",
        Format::Fountain => "fountain",
    };
    let output = output.unwrap_or_else(|| input.with_extension(ext));
    if output == input {
        return Err(format!("refusing to overwrite the input file {}", input.display()));
    }
    let bytes = match format {
        Format::Pdf => {
            popts.title = document_title(&script);
            pdf::render_with(&layout.pages, &popts)
        }
        Format::Fdx => fdx::export(&script).into_bytes(),
        Format::Fountain => reelscript_core::fountain::write(&script).into_bytes(),
    };
    std::fs::write(&output, bytes).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    println!("Wrote {} ({} pages, {} scenes)", output.display(), layout.body_pages(), layout.scenes.len());
    Ok(())
}
