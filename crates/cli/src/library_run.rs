//! The phases of a `library` run: read, convert, write (contract 2.2).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use altium_format::pcb::PcbLib;
use altium_format::sch::SchLib;
use chrono::{DateTime, Utc};
use signex_altium_convert::libset::{
    LibraryOptions, PreviousRow, SourceBody, SourceLib, build_library,
};
use signex_altium_convert::report::{Item, ItemKind, Severity};
use signex_library::{LibraryFile, LifecycleState};
use uuid::Uuid;

use crate::contract::{Counts, Exit, LibrarySummary};
use crate::library_cmd::{LibraryArgs, Source, check_output, classify};
use crate::output_guard::Guard;
use crate::progress::Progress;
use crate::report_doc::{LibraryReport, SourceFile};

fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("library")
        .to_string()
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn counts(c: signex_altium_convert::libset::Counts) -> Counts {
    Counts {
        symbols: c.symbols,
        footprints: c.footprints,
        parts: c.parts,
        models: c.models,
    }
}

fn library_item(code: &'static str, source: String, message: String) -> Item {
    Item {
        kind: ItemKind::Library,
        source,
        id: None,
        code,
        severity: Severity::Error,
        message,
        detail: serde_json::Value::Null,
    }
}

/// The `.snxlib` files directly in `dir`.
fn snxlibs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "snxlib"))
        .collect();
    v.sort();
    v
}

/// The library an `--update` run refreshes: its path and id.
fn existing_library(dir: &Path) -> Result<(PathBuf, LibraryFile), String> {
    let libs = snxlibs(dir);
    let [path] = libs.as_slice() else {
        return Err(format!(
            "--update: {} holds {} .snxlib files; expected exactly one",
            dir.display(),
            libs.len()
        ));
    };
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("--update: reading {}: {e}", path.display()))?;
    let file = LibraryFile::parse(&text)
        .map_err(|e| format!("--update: {} is not a library: {e}", path.display()))?;
    Ok((path.clone(), file))
}

fn state_of(cell: &str) -> Option<LifecycleState> {
    Some(match cell {
        "Draft" => LifecycleState::Draft,
        "InReview" => LifecycleState::InReview,
        "Released" => LifecycleState::Released,
        "Deprecated" => LifecycleState::Deprecated,
        "Obsolete" => LifecycleState::Obsolete,
        _ => return None,
    })
}

fn hash_of(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// What the library on disk says about each part.
pub fn previous_rows(lib: &LibraryFile) -> HashMap<Uuid, PreviousRow> {
    let mut out = HashMap::new();
    for t in lib.tables.values() {
        for row in &t.rows {
            let cell = |c: &str| t.cell(row, c).unwrap_or_default();
            let time =
                |c: &str| DateTime::parse_from_rfc3339(cell(c)).map(|d| d.with_timezone(&Utc));
            let Some(content_hash) = hash_of(cell("content_hash")) else {
                continue;
            };
            let (Ok(id), Some(state), Ok(created), Ok(updated)) = (
                Uuid::parse_str(cell("row_id")),
                state_of(cell("state")),
                time("created"),
                time("updated"),
            ) else {
                continue;
            };
            out.insert(
                id,
                PreviousRow {
                    state,
                    released: cell("released") == "true",
                    created,
                    updated,
                    version: cell("version").to_string(),
                    content_hash,
                },
            );
        }
    }
    out
}

/// What a run produced: the summary, and the report when one was written.
pub fn run(args: &LibraryArgs, progress: &Progress) -> LibrarySummary {
    if let Err(e) = args.validate() {
        return LibrarySummary::failed(e);
    }
    let mut kinds = Vec::new();
    for p in &args.inputs {
        match classify(p) {
            Ok(k) => kinds.push(k),
            Err(e) => return LibrarySummary::failed(e),
        }
    }
    if let Err(e) = check_output(args) {
        return LibrarySummary::failed(e);
    }

    let mut doc = LibraryReport::new(now());
    doc.library_id = args.library_id.clone();

    // Read.
    progress.phase("read", Some(args.inputs.len()));
    let mut sources: Vec<SourceLib> = Vec::new();
    let mut unreadable = 0;
    for (i, (path, kind)) in args.inputs.iter().zip(&kinds).enumerate() {
        let name = stem(path);
        progress.progress(i, Some(args.inputs.len()), Some(&name));
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let read = SourceFile::hash(&abs).and_then(|src| {
            let body = match kind {
                Source::SchLib => SourceBody::Sch(SchLib::open(&abs)?),
                Source::PcbLib => SourceBody::Pcb(PcbLib::open(&abs)?),
            };
            Ok((src, body))
        });
        match read {
            Ok((src, body)) => {
                sources.push(SourceLib {
                    path: abs.to_string_lossy().into_owned(),
                    name,
                    body,
                });
                doc.sources.push(src);
            }
            Err(e) => {
                unreadable += 1;
                doc.report.push(library_item(
                    "library.input_unreadable",
                    name,
                    format!("{}: {e:#}", path.display()),
                ));
            }
        }
    }
    progress.progress(args.inputs.len(), Some(args.inputs.len()), None);
    if unreadable > 0 {
        let msg = format!("{unreadable} input(s) could not be read");
        return write_report(args, &doc, Some(msg), Vec::new());
    }

    // `--update`: the library on disk must be the one this source makes.
    let mut options = LibraryOptions::fresh(match &args.name {
        Some(n) => n.clone(),
        None => args.inputs.first().map(|p| stem(p)).unwrap_or_default(),
    });
    options.library_id = args
        .library_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok());
    options.now = Utc::now();
    let mut existing = None;
    if args.update {
        match existing_library(&args.output) {
            Ok((path, lib)) => {
                options.previous = previous_rows(&lib);
                existing = Some((path, lib.manifest.library_id));
            }
            Err(e) => return write_report(args, &doc, Some(e), Vec::new()),
        }
    }

    // Convert.
    let total = sources
        .iter()
        .map(|s| match &s.body {
            SourceBody::Sch(l) => l.components.len(),
            SourceBody::Pcb(l) => l.footprints.len(),
        })
        .sum();
    progress.phase("convert", Some(total));
    let built = build_library(
        &sources,
        &options,
        &mut doc.report,
        &mut |done, total, item| {
            progress.progress(done, Some(total), Some(item));
        },
    );
    let built = match built {
        Ok(b) => b,
        Err(e) => return write_report(args, &doc, Some(format!("{e}")), Vec::new()),
    };
    doc.library_id = Some(built.library_id.to_string());
    doc.source = counts(built.source);
    doc.written = counts(built.written);
    if !doc.is_complete() {
        let msg = format!(
            "internal error: the report does not account for every source item ({:?} unaccounted)",
            doc.unaccounted()
        );
        return write_report(args, &doc, Some(msg), Vec::new());
    }
    if let Some((_, id)) = existing.as_ref().filter(|(_, id)| *id != built.library_id) {
        let msg = format!(
            "--update: the library in {} has id {id}, this source gives {}; pass --library-id to keep the old one",
            args.output.display(),
            built.library_id
        );
        return write_report(args, &doc, Some(msg), Vec::new());
    }

    // Write.
    if args.dry_run {
        // A preview writes nothing but the report.
        return write_report(args, &doc, None, Vec::new());
    }
    progress.phase("write", Some(built.files.len()));
    let dir = match std::fs::create_dir_all(&args.output).and_then(|()| args.output.canonicalize())
    {
        Ok(d) => d,
        Err(e) => {
            let msg = format!("creating {}: {e}", args.output.display());
            return write_report(args, &doc, Some(msg), Vec::new());
        }
    };
    let mut guard = Guard::new();
    let mut written = Vec::new();
    let res = write_files(
        &mut guard,
        &dir,
        &built.files,
        existing.as_ref().map(|e| &e.0),
        progress,
        &mut written,
    );
    match res {
        Ok(()) => {
            let summary = write_report(args, &doc, None, written);
            if summary.error.is_none() {
                guard.commit();
            }
            summary
        }
        Err(e) => {
            drop(guard);
            write_report(args, &doc, Some(format!("{e:#}")), Vec::new())
        }
    }
}

/// Write the new files, then delete what an update left behind: old
/// envelopes, footprints and library files the new set does not hold.
fn write_files(
    guard: &mut Guard,
    dir: &Path,
    files: &[(String, String)],
    old_snxlib: Option<&PathBuf>,
    progress: &Progress,
    written: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for (i, (rel, text)) in files.iter().enumerate() {
        progress.progress(i, Some(files.len()), Some(rel));
        let path = dir.join(rel);
        guard.write(&path, text.as_bytes())?;
        written.push(path);
    }
    progress.progress(files.len(), Some(files.len()), None);
    if let Some(old) = old_snxlib {
        let keep: std::collections::HashSet<PathBuf> = written.iter().cloned().collect();
        let mut stale: Vec<PathBuf> = vec![old.clone()];
        for sub in ["symbols", "footprints"] {
            if let Ok(rd) = std::fs::read_dir(dir.join(sub)) {
                stale.extend(rd.flatten().map(|e| e.path()));
            }
        }
        for p in stale
            .into_iter()
            .filter(|p| !keep.contains(p) && p.is_file())
        {
            guard.remove(&p)?;
        }
    }
    Ok(())
}

/// Write the report into `--output`. With `error` the summary is a failed
/// one that still names the report; without, it carries the written files.
fn write_report(
    args: &LibraryArgs,
    doc: &LibraryReport,
    error: Option<String>,
    files: Vec<PathBuf>,
) -> LibrarySummary {
    let dir = &args.output;
    if let Err(e) = std::fs::create_dir_all(dir) {
        return LibrarySummary::failed(format!("creating {}: {e}", dir.display()));
    }
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    let (json, _md) = match doc.write(&dir) {
        Ok(p) => p,
        Err(e) => return LibrarySummary::failed(format!("{e:#}")),
    };
    let exit = if error.is_some() {
        Exit::Failed
    } else {
        Exit::of_report(&doc.report)
    };
    LibrarySummary {
        exit: Some(exit),
        error,
        library: Some(dir),
        report: Some(json),
        files,
        counts: Some(doc.written),
        skipped: doc.skipped(),
        warnings: doc
            .report
            .warnings()
            .into_iter()
            .map(|(m, n)| (m.to_string(), n))
            .collect(),
    }
}
