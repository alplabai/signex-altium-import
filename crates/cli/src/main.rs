//! `signex-altium-import` -- one-way converter from Altium Designer files
//! to Signex native files.
//!
//! ```text
//! signex-altium-import Board.PrjPcb            # whole project -> .snxprj + siblings
//! signex-altium-import Sheet.SchDoc -o out/    # one sheet
//! signex-altium-import Board.PcbDoc --json     # machine-readable summary on stdout
//! ```
//!
//! Signex's File > Import > Altium Project runs this binary and reads the
//! `--json` summary; `docs/APP_INTEGRATION.md` is that contract.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use altium_format::pcb::{PcbDoc, PcbLib};
use altium_format::prj::Project;
use altium_format::sch::{SchDoc, SchLib};
use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use signex_altium_convert::Report;
use signex_altium_convert::sch::{PortScope, SchOptions, base_name, snx_sheet_name};
use signex_types::format::{SnxPcb, SnxSchematic};
use signex_types::project::{ProjectData, SheetEntry};

#[derive(Parser, Debug)]
#[command(
    name = "signex-altium-import",
    version,
    about = "Convert Altium Designer files (.PrjPcb / .SchDoc / .PcbDoc) to Signex native files"
)]
struct Cli {
    /// A .PrjPcb, .SchDoc or .PcbDoc file. .SchLib / .PcbLib are listed only.
    input: PathBuf,

    /// Output directory. Defaults to the input file's directory.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Replace output files that already exist.
    #[arg(long)]
    force: bool,

    /// How Altium ports become Signex labels.
    #[arg(long, value_enum, default_value_t = Ports::Auto)]
    ports: Ports,

    /// Print a JSON summary on stdout instead of human-readable progress.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Ports {
    /// Hierarchical when the design has sheet symbols, global otherwise.
    Auto,
    Hierarchical,
    Global,
}

/// What a run produced; also the `--json` contract with the app.
#[derive(Default)]
struct Summary {
    project: Option<PathBuf>,
    files: Vec<PathBuf>,
    report: Report,
}

fn main() {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(summary) if cli.json => println!("{}", summary_json(&summary, None)),
        Ok(summary) => print_summary(&summary),
        Err(e) => {
            if cli.json {
                println!(
                    "{}",
                    summary_json(&Summary::default(), Some(&format!("{e:#}")))
                );
            } else {
                eprintln!("error: {e:#}");
            }
            std::process::exit(1);
        }
    }
}

fn run(cli: &Cli) -> Result<Summary> {
    let input = cli
        .input
        .canonicalize()
        .with_context(|| format!("input not found: {}", cli.input.display()))?;
    let dir = match &cli.output {
        Some(o) => {
            std::fs::create_dir_all(o)
                .with_context(|| format!("creating output directory {}", o.display()))?;
            o.canonicalize()?
        }
        None => input.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    let mut out = Output {
        dir,
        force: cli.force,
        written: Vec::new(),
    };
    let mut summary = Summary::default();
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "prjpcb" => {
            summary.project = Some(convert_project(
                &input,
                cli.ports,
                &mut out,
                &mut summary.report,
            )?);
        }
        "schdoc" => {
            let doc = SchDoc::open(&input)?;
            let opts = SchOptions {
                port_scope: port_scope(cli.ports, !doc.sheet_symbols.is_empty()),
                ..Default::default()
            };
            let name = snx_sheet_name(&file_name(&input));
            out.check_free(std::slice::from_ref(&name))?;
            let sheet = signex_altium_convert::sch::convert(&doc, &opts, &mut summary.report);
            out.write(&name, &SnxSchematic::new(sheet).write_string()?)?;
        }
        "pcbdoc" => {
            let doc = PcbDoc::open(&input)?;
            let name = format!("{}.snxpcb", stem(&input));
            out.check_free(std::slice::from_ref(&name))?;
            let board = signex_altium_convert::pcb::convert(&doc, &mut summary.report);
            out.write(&name, &SnxPcb::new(board).write_string()?)?;
        }
        "schlib" => {
            let lib = SchLib::open(&input)?;
            list(
                "symbols",
                lib.components.iter().map(|c| c.lib_reference.as_str()),
                cli.json,
            );
            bail!(
                "library conversion is not implemented yet; import a design that uses \
                 these symbols and they come along embedded in its sheets"
            );
        }
        "pcblib" => {
            let lib = PcbLib::open(&input)?;
            list(
                "footprints",
                lib.footprints.iter().map(|f| f.name.as_str()),
                cli.json,
            );
            bail!("footprint library conversion is not implemented yet");
        }
        "" => bail!("input has no extension; expected .PrjPcb, .SchDoc or .PcbDoc"),
        other => bail!("unsupported input .{other}; expected .PrjPcb, .SchDoc or .PcbDoc"),
    }
    summary.files = out.written;
    Ok(summary)
}

fn port_scope(ports: Ports, has_sheet_symbols: bool) -> PortScope {
    match ports {
        Ports::Hierarchical => PortScope::Hierarchical,
        Ports::Global => PortScope::Global,
        Ports::Auto if has_sheet_symbols => PortScope::Hierarchical,
        Ports::Auto => PortScope::Global,
    }
}

fn list<'a>(what: &str, names: impl Iterator<Item = &'a str>, json: bool) {
    if json {
        return;
    }
    println!("{what}:");
    for n in names {
        println!("  {n}");
    }
}

/// Writes outputs, refusing to replace existing files unless `--force`.
struct Output {
    dir: PathBuf,
    force: bool,
    written: Vec<PathBuf>,
}

impl Output {
    /// Fail before anything is written if any target already exists.
    fn check_free(&self, names: &[String]) -> Result<()> {
        if self.force {
            return Ok(());
        }
        if let Some(p) = names.iter().map(|n| self.dir.join(n)).find(|p| p.exists()) {
            bail!("{} already exists; pass --force to replace it", p.display());
        }
        Ok(())
    }

    fn write(&mut self, name: &str, text: &str) -> Result<()> {
        let path = self.dir.join(name);
        signex_types::atomic_io::atomic_write(&path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        self.written.push(path);
        Ok(())
    }
}

fn convert_project(
    input: &Path,
    ports: Ports,
    out: &mut Output,
    report: &mut Report,
) -> Result<PathBuf> {
    let project_dir = input.parent().unwrap_or(Path::new("."));
    let prj = Project::open(input)?;

    // Read every document before writing anything.
    let mut sheets = Vec::new();
    for p in prj.documents_with_ext(project_dir, "SchDoc") {
        let doc = SchDoc::open(&p).with_context(|| format!("reading {}", p.display()))?;
        sheets.push((p, doc));
    }
    let mut boards = Vec::new();
    for p in prj.documents_with_ext(project_dir, "PcbDoc") {
        let doc = PcbDoc::open(&p).with_context(|| format!("reading {}", p.display()))?;
        boards.push((p, doc));
    }
    if sheets.is_empty() && boards.is_empty() {
        bail!("{} lists no .SchDoc or .PcbDoc documents", input.display());
    }

    // Output names, unique even when two documents share a stem.
    let mut used = HashSet::new();
    let mut unique = |base: String, ext: &str| {
        let mut name = format!("{base}.{ext}");
        let mut n = 2;
        while !used.insert(name.to_ascii_lowercase()) {
            name = format!("{base}_{n}.{ext}");
            n += 1;
        }
        name
    };
    let sheet_names: Vec<String> = sheets
        .iter()
        .map(|(p, _)| unique(stem(p), "snxsch"))
        .collect();
    let board_names: Vec<String> = boards
        .iter()
        .map(|(p, _)| unique(stem(p), "snxpcb"))
        .collect();
    let project_name = unique(stem(input), "snxprj");
    let mut targets: Vec<String> = sheet_names.iter().chain(&board_names).cloned().collect();
    targets.push(project_name.clone());
    out.check_free(&targets)?;

    let opts = SchOptions {
        port_scope: port_scope(
            ports,
            sheets.iter().any(|(_, d)| !d.sheet_symbols.is_empty()),
        ),
        sheet_files: sheets
            .iter()
            .zip(&sheet_names)
            .map(|((p, _), n)| (file_name(p).to_ascii_lowercase(), n.clone()))
            .collect::<HashMap<_, _>>(),
        project_parameters: prj.parameters.clone(),
    };
    // The root sheet is the one no sheet symbol points at.
    let children: HashSet<String> = sheets
        .iter()
        .flat_map(|(_, d)| &d.sheet_symbols)
        .map(|s| base_name(&s.file_name).to_ascii_lowercase())
        .collect();

    let mut entries = Vec::new();
    let mut root = None;
    for ((path, doc), name) in sheets.iter().zip(&sheet_names) {
        let sheet = signex_altium_convert::sch::convert(doc, &opts, report);
        entries.push(SheetEntry {
            name: stem(path),
            filename: name.clone(),
            symbols_count: sheet.symbols.len(),
            wires_count: sheet.wires.len(),
            labels_count: sheet.labels.len(),
            unknown: Default::default(),
        });
        if root.is_none() && !children.contains(&file_name(path).to_ascii_lowercase()) {
            root = Some(name.clone());
        }
        out.write(name, &SnxSchematic::new(sheet).write_string()?)?;
    }
    if boards.len() > 1 {
        report.warn("project has several PCB documents; the first is the project's board");
    }
    for ((_, doc), name) in boards.iter().zip(&board_names) {
        let board = signex_altium_convert::pcb::convert(doc, report);
        out.write(name, &SnxPcb::new(board).write_string()?)?;
    }

    let data = ProjectData {
        name: stem(input),
        dir: out.dir.to_string_lossy().into_owned(),
        schematic_root: root.or_else(|| sheet_names.first().cloned()),
        pcb_file: board_names.first().cloned(),
        sheets: entries,
        variant_definitions: prj.variants.clone(),
        active_variant: None,
        libraries: Vec::new(),
        enable_git: false,
        unknown: Default::default(),
    };
    let path = out.dir.join(&project_name);
    signex_types::project::write_project(&path, &data)
        .with_context(|| format!("writing {}", path.display()))?;
    out.written.push(path.clone());
    Ok(path)
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

fn print_summary(s: &Summary) {
    for f in &s.files {
        println!("wrote {}", f.display());
    }
    if !s.report.is_empty() {
        println!("approximated or skipped:");
        print!("{}", s.report);
    }
    if let Some(p) = &s.project {
        println!("open {} in Signex", p.display());
    }
}

fn summary_json(s: &Summary, error: Option<&str>) -> String {
    let warnings: Vec<serde_json::Value> = s
        .report
        .entries()
        .map(|(m, n)| serde_json::json!({ "message": m, "count": n }))
        .collect();
    serde_json::json!({
        "ok": error.is_none(),
        "error": error,
        "project": s.project,
        "files": s.files,
        "warnings": warnings,
    })
    .to_string()
}
