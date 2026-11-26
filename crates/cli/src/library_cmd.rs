//! `signex-altium-import library <input>... --output <dir> [options]`
//! (library converter contract, section 2.2).
//!
//! The run itself is `library_run`.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};

use crate::contract::LibrarySummary;
use crate::library_run::run;
use crate::progress::Progress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Models {
    /// Embed 3D models in footprint files.
    Include,
    /// Write no models and report each one skipped.
    Skip,
}

#[derive(Parser, Debug)]
#[command(
    name = "signex-altium-import library",
    about = "Convert Altium libraries into one Signex library directory"
)]
pub struct LibraryArgs {
    /// Source files or folders; several go into one library.
    #[arg(required = true, num_args = 1..)]
    pub inputs: Vec<PathBuf>,

    /// The library directory to write.
    #[arg(long, short, required = true)]
    pub output: PathBuf,

    /// Library display name (default: derived from the first input).
    #[arg(long)]
    pub name: Option<String>,

    /// Overrides the derived library id (a UUID).
    #[arg(long)]
    pub library_id: Option<String>,

    /// Database sources: a file path, connection string or CSV folder.
    #[arg(long)]
    pub db: Option<String>,

    /// Extra folders searched for libraries a source refers to.
    #[arg(long = "search-path")]
    pub search_path: Vec<PathBuf>,

    /// Field-mapping overrides (TOML).
    #[arg(long)]
    pub map: Option<PathBuf>,

    /// Embed 3D models or skip them.
    #[arg(long, value_enum, default_value_t = Models::Include)]
    pub models: Models,

    /// Folder that model paths in the source resolve against.
    #[arg(long)]
    pub models_dir: Option<PathBuf>,

    /// Read and map everything, write only the report.
    #[arg(long)]
    pub dry_run: bool,

    /// `--output` holds a library written from the same source.
    #[arg(long)]
    pub update: bool,

    /// Print the summary as the last stdout line.
    #[arg(long)]
    pub json: bool,

    /// Print NDJSON progress events on stdout (implies `--json`).
    #[arg(long)]
    pub progress: bool,
}

impl LibraryArgs {
    pub fn json(&self) -> bool {
        self.json || self.progress
    }

    /// Checks that need no source file; the message is the summary's error.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(id) = &self.library_id {
            uuid::Uuid::parse_str(id).map_err(|_| format!("--library-id {id:?} is not a UUID"))?;
        }
        if let Some(m) = self.map.as_ref().filter(|m| !m.is_file()) {
            return Err(format!("--map {} is not a file", m.display()));
        }
        if let Some(d) = self.models_dir.as_ref().filter(|d| !d.is_dir()) {
            return Err(format!("--models-dir {} is not a folder", d.display()));
        }
        for d in &self.search_path {
            if !d.is_dir() {
                return Err(format!("--search-path {} is not a folder", d.display()));
            }
        }
        // Two inputs of one kind with the same stem would name their items
        // alike, so their ids would collide.
        let mut seen = std::collections::HashSet::new();
        for p in &self.inputs {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_lowercase();
            // An integrated library holds one library of each kind.
            let kinds: &[&str] = if ext == "intlib" {
                &["schlib", "pcblib"]
            } else {
                &[ext.as_str()]
            };
            if kinds
                .iter()
                .any(|k| !seen.insert((k.to_string(), stem.clone())))
            {
                return Err(format!(
                    "{}: another input of the same kind has this file name; rename one",
                    p.display()
                ));
            }
        }
        if self.name.as_deref().is_some_and(str::is_empty) {
            return Err("--name is empty".into());
        }
        Ok(())
    }
}

/// A source format `library` knows by extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Source {
    SchLib,
    PcbLib,
    IntLib,
}

pub(crate) fn classify(path: &Path) -> Result<Source, String> {
    if path.is_dir() {
        return Err(format!(
            "{}: folder inputs are not implemented yet",
            path.display()
        ));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "schlib" => Ok(Source::SchLib),
        "pcblib" => Ok(Source::PcbLib),
        "intlib" => Ok(Source::IntLib),
        "dblib" => Err(format!(
            "{}: reading .dblib is not implemented yet",
            path.display()
        )),
        _ => Err(format!(
            "{}: unsupported input; expected .SchLib, .PcbLib, .IntLib or .DbLib",
            path.display()
        )),
    }
}

/// Without `--update` the output must not exist or be empty.
pub(crate) fn check_output(args: &LibraryArgs) -> Result<(), String> {
    let out = &args.output;
    match (out.exists(), args.update) {
        (false, false) => Ok(()),
        (false, true) => Err(format!("--update: {} does not exist", out.display())),
        (true, true) if out.is_dir() => Ok(()),
        (true, true) => Err(format!("--update: {} is not a directory", out.display())),
        (true, false) => {
            let empty = out.is_dir()
                && std::fs::read_dir(out)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false);
            if empty {
                Ok(())
            } else {
                Err(format!(
                    "{} exists and is not empty; use --update to refresh a library",
                    out.display()
                ))
            }
        }
    }
}

/// Entry point of the `library` subcommand; returns the process exit code.
pub fn main(raw: &[OsString]) -> i32 {
    let argv = std::iter::once(OsString::from("library")).chain(raw.iter().cloned());
    let args = match LibraryArgs::try_parse_from(argv) {
        Ok(a) => a,
        Err(e) if !e.use_stderr() => {
            let _ = e.print();
            return 0;
        }
        Err(e) => {
            // The contract wants a failed summary even for a bad option.
            let msg = e.to_string();
            eprint!("{msg}");
            let first = msg.lines().next().unwrap_or("invalid arguments");
            let error = first.strip_prefix("error: ").unwrap_or(first);
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                LibrarySummary::failed(error).to_json()
            );
            return 1;
        }
    };
    let progress = if args.progress {
        Progress::stdout()
    } else {
        Progress::disabled()
    };
    // A panic is a failed run with a summary, never a bare crash.
    let summary = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&args, &progress)))
        .unwrap_or_else(|p| {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            LibrarySummary::failed(format!("internal error: {what}"))
        });
    let exit = summary.exit();
    if progress.enabled() {
        progress.summary(&summary.to_json());
    } else if args.json() {
        // A closed stdout must not turn into a second panic.
        let _ = writeln!(std::io::stdout(), "{}", summary.to_json());
    } else if let Some(e) = &summary.error {
        eprintln!("error: {e}");
    } else {
        print_human(&summary);
    }
    exit.code()
}

fn print_human(s: &LibrarySummary) {
    if let Some(r) = &s.report {
        println!("wrote {}", r.display());
    }
    if !s.warnings.is_empty() {
        println!("approximated or skipped:");
        for (m, n) in &s.warnings {
            if *n == 1 {
                println!("  - {m}");
            } else {
                println!("  - {m} (x{n})");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> Result<LibraryArgs, clap::Error> {
        LibraryArgs::try_parse_from(std::iter::once("library").chain(a.iter().copied()))
    }

    #[test]
    fn every_contract_option_parses() {
        let a = parse(&[
            "a.SchLib",
            "b.PcbLib",
            "--output",
            "out",
            "--name",
            "N",
            "--library-id",
            "5b0f3b0e-0000-4000-8000-000000000000",
            "--db",
            "conn",
            "--search-path",
            "s1",
            "--search-path",
            "s2",
            "--map",
            "m.toml",
            "--models",
            "skip",
            "--models-dir",
            "md",
            "--dry-run",
            "--update",
            "--json",
            "--progress",
        ])
        .unwrap();
        assert_eq!(a.inputs.len(), 2);
        assert_eq!(a.search_path.len(), 2);
        assert_eq!(a.models, Models::Skip);
        assert!(a.dry_run && a.update && a.json() && a.progress);
        assert_eq!(a.name.as_deref(), Some("N"));
    }

    #[test]
    fn defaults() {
        let a = parse(&["a.SchLib", "--output", "out"]).unwrap();
        assert_eq!(a.models, Models::Include);
        assert!(!a.dry_run && !a.update && !a.json());
    }

    #[test]
    fn unknown_option_and_missing_pieces_are_errors() {
        assert!(parse(&["a.SchLib", "--output", "o", "--bogus"]).is_err());
        assert!(parse(&["--output", "o"]).is_err());
        assert!(parse(&["a.SchLib"]).is_err());
        assert!(parse(&["a.SchLib", "--output", "o", "--models", "maybe"]).is_err());
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut a = parse(&["a.SchLib", "--output", "o", "--library-id", "nope"]).unwrap();
        assert!(a.validate().unwrap_err().contains("UUID"));
        a.library_id = None;
        a.map = Some("/definitely/not/here.toml".into());
        assert!(a.validate().unwrap_err().contains("--map"));
    }

    #[test]
    fn duplicate_stems_of_one_kind_are_refused_case_insensitively() {
        let a = parse(&["a/X.SchLib", "b/x.schlib", "--output", "o"]).unwrap();
        assert!(a.validate().unwrap_err().contains("same kind"));
        let same = parse(&["X.SchLib", "X.SchLib", "--output", "o"]).unwrap();
        assert!(same.validate().is_err());
        let ok = parse(&["X.SchLib", "X.PcbLib", "--output", "o"]).unwrap();
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn input_kinds() {
        assert_eq!(classify(Path::new("x.SCHLIB")), Ok(Source::SchLib));
        assert_eq!(classify(Path::new("x.pcblib")), Ok(Source::PcbLib));
        assert_eq!(classify(Path::new("x.IntLib")), Ok(Source::IntLib));
        assert!(
            classify(Path::new("x.DbLib"))
                .unwrap_err()
                .contains("not implemented")
        );
        assert!(
            classify(Path::new("x.txt"))
                .unwrap_err()
                .contains("unsupported")
        );
    }
}
