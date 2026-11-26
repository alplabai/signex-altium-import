//! The phases of a `library` run: read, convert, write (contract 2.2).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use altium_format::intlib::IntLib;
use altium_format::pcb::PcbLib;
use altium_format::sch::SchLib;
use chrono::{DateTime, Utc};
use signex_altium_convert::libset::{
    LibraryOptions, PreviousPrimitive, PreviousRow, SourceBody, SourceLib, build_library,
};
use signex_altium_convert::report::{Item, ItemKind, Severity};
use signex_library::primitive::symbol::SymbolFile;
use signex_library::{FootprintFile, LibraryFile, LifecycleState};
use uuid::Uuid;

use crate::contract::{Counts, Exit, LibrarySummary};
use crate::ledger::Ledger;
use crate::library_cmd::{LibraryArgs, Models, Source, check_output, classify};
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

/// The regular files of the expected form directly in `dir/sub` (`sub` empty
/// for `dir` itself): no symlinks, plain `<name>.<ext>` names, and for
/// footprints a UUID stem. The folder itself must be a real folder. Nothing
/// from a report or an undo log, and nothing outside `dir`.
fn plain_files(dir: &Path, sub: &str, ext: &str, uuid_stem: bool) -> Vec<PathBuf> {
    let folder = if sub.is_empty() {
        dir.to_path_buf()
    } else {
        dir.join(sub)
    };
    if !std::fs::symlink_metadata(&folder).is_ok_and(|m| m.is_dir()) {
        return Vec::new();
    }
    let mut v: Vec<PathBuf> = std::fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file() && !t.is_symlink()))
        .map(|e| e.path())
        .filter(|p| {
            let (Some(name), Some(stem)) = (
                p.file_name().and_then(|n| n.to_str()),
                p.file_stem().and_then(|n| n.to_str()),
            ) else {
                return false;
            };
            p.extension().is_some_and(|e| e == ext)
                && !stem.is_empty()
                && !name.contains(['/', '\\'])
                && (!uuid_stem || Uuid::parse_str(stem).is_ok())
        })
        .collect();
    v.sort();
    v
}

/// The `.snxlib` files directly in `dir`.
fn snxlibs(dir: &Path) -> Vec<PathBuf> {
    plain_files(dir, "", "snxlib", false)
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

/// What the library on disk says about each part. A row this converter
/// cannot read back fails the run: an update must not guess a part's state.
pub fn previous_rows(lib: &LibraryFile) -> Result<HashMap<Uuid, PreviousRow>, String> {
    let mut out = HashMap::new();
    for t in lib.tables.values() {
        for row in &t.rows {
            let cell = |c: &str| t.cell(row, c).unwrap_or_default();
            let bad = |what: &str| {
                format!(
                    "--update: part {:?} has an unreadable {what}; fix or remove it first",
                    cell("internal_pn")
                )
            };
            let time = |c: &str| {
                DateTime::parse_from_rfc3339(cell(c))
                    .map(|d| d.with_timezone(&Utc))
                    .map_err(|_| bad(c))
            };
            let id = Uuid::parse_str(cell("row_id")).map_err(|_| bad("row_id"))?;
            out.insert(
                id,
                PreviousRow {
                    state: state_of(cell("state")).ok_or_else(|| bad("state"))?,
                    released: cell("released") == "true",
                    created: time("created")?,
                    updated: time("updated")?,
                    version: cell("version").to_string(),
                    content_hash: hash_of(cell("content_hash"))
                        .ok_or_else(|| bad("content_hash"))?,
                    symbol_version: cell("symbol_version").to_string(),
                    footprint_version: cell("footprint_version").to_string(),
                },
            );
        }
    }
    Ok(out)
}

type Primitives = HashMap<Uuid, PreviousPrimitive>;

/// The symbols and footprints on disk, by id. An envelope that does not
/// read fails the run.
fn previous_primitives(dir: &Path) -> Result<(Primitives, Primitives), String> {
    let mut syms = HashMap::new();
    let mut fps = HashMap::new();
    for p in plain_files(dir, "symbols", "snxsym", false) {
        let text =
            std::fs::read_to_string(&p).map_err(|e| format!("--update: {}: {e}", p.display()))?;
        let f = SymbolFile::from_toml_str(&text)
            .map_err(|e| format!("--update: {} does not read: {e}", p.display()))?;
        for s in &f.symbols {
            syms.insert(
                s.uuid,
                PreviousPrimitive::of_symbol(s)
                    .map_err(|e| format!("--update: {}: {e}", p.display()))?,
            );
        }
    }
    for p in plain_files(dir, "footprints", "snxfpt", true) {
        let text =
            std::fs::read_to_string(&p).map_err(|e| format!("--update: {}: {e}", p.display()))?;
        let f = FootprintFile::from_toml_str(&text)
            .map_err(|e| format!("--update: {} does not read: {e}", p.display()))?;
        for s in &f.footprints {
            fps.insert(
                s.uuid,
                PreviousPrimitive::of_footprint(s)
                    .map_err(|e| format!("--update: {}: {e}", p.display()))?,
            );
        }
    }
    Ok((syms, fps))
}

/// The envelope files on disk by relative path, so an unchanged one keeps
/// its text (and times).
fn previous_files(dir: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (sub, ext, uuid) in [("symbols", "snxsym", false), ("footprints", "snxfpt", true)] {
        for p in plain_files(dir, sub, ext, uuid) {
            if let (Some(name), Ok(text)) = (p.file_name(), std::fs::read_to_string(&p)) {
                out.insert(format!("{sub}/{}", name.to_string_lossy()), text);
            }
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
    // The previous run's record of what it wrote. It stays in the report of a
    // run that fails or writes nothing, so a retry still knows what is ours.
    if args.update
        && let Ok(canon) = args.output.canonicalize()
    {
        doc.ledger = Ledger::read(&canon);
    }

    // Read.
    progress.phase("read", Some(args.inputs.len()));
    let mut sources: Vec<SourceLib> = Vec::new();
    let mut unreadable = 0;
    for (i, (path, kind)) in args.inputs.iter().zip(&kinds).enumerate() {
        let name = stem(path);
        progress.progress(i, Some(args.inputs.len()), Some(&name));
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let read = SourceFile::hash(&abs).and_then(|src| {
            let bodies = match kind {
                Source::SchLib => vec![(SourceBody::Sch(SchLib::open(&abs)?), false)],
                Source::PcbLib => vec![(SourceBody::Pcb(PcbLib::open(&abs)?), false)],
                Source::IntLib => {
                    let int = IntLib::open(&abs)?;
                    for m in int.cross_ref_mismatches() {
                        doc.report.push(Item {
                            kind: ItemKind::Library,
                            source: name.clone(),
                            id: None,
                            code: "library.crossref_mismatch",
                            severity: Severity::Info,
                            message: m,
                            detail: serde_json::Value::Null,
                        });
                    }
                    // Integrated libraries are published: their parts are released.
                    int.schlibs
                        .into_iter()
                        .map(|e| (SourceBody::Sch(e.lib), true))
                        .chain(
                            int.pcblibs
                                .into_iter()
                                .map(|e| (SourceBody::Pcb(e.lib), true)),
                        )
                        .collect()
                }
            };
            Ok((src, bodies))
        });
        match read {
            Ok((src, bodies)) => {
                for (body, released) in bodies {
                    sources.push(SourceLib {
                        released,
                        path: abs.to_string_lossy().into_owned(),
                        name: name.clone(),
                        body,
                    });
                }
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
    options.include_models = args.models == Models::Include;
    let mut existing = None;
    let mut usable: Option<Ledger> = None;
    if args.update {
        let canon = args
            .output
            .canonicalize()
            .unwrap_or_else(|_| args.output.clone());
        match existing_library(&canon) {
            Ok((path, lib)) => {
                let prev = previous_rows(&lib)
                    .and_then(|rows| previous_primitives(&canon).map(|(s, f)| (rows, s, f)));
                match prev {
                    Ok((rows, syms, fps)) => {
                        options.previous = rows;
                        options.previous_symbols = syms;
                        options.previous_footprints = fps;
                        options.previous_files = previous_files(&canon);
                    }
                    Err(e) => return write_report(args, &doc, Some(e), Vec::new()),
                }
                usable = doc
                    .ledger
                    .clone()
                    .filter(|l| l.library_id == Some(lib.manifest.library_id));
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

    // An update merges into the library on disk, keeping its file name and
    // whatever a user added; only rows this converter wrote earlier go.
    let mut files = built.files.clone();
    if let Some((old_path, _)) = &existing {
        let merged = std::fs::read_to_string(old_path)
            .map_err(|e| format!("--update: {}: {e}", old_path.display()))
            .and_then(|old| {
                let at = files
                    .iter()
                    .position(|(p, _)| p.ends_with(".snxlib"))
                    .ok_or("no library file")?;
                let before = usable.as_ref().map(|l| l.rows.clone()).unwrap_or_default();
                let text = crate::update_merge::merge(&old, &files[at].1, &before)?;
                Ok((at, text))
            });
        match merged {
            Ok((at, text)) => {
                files[at].0 = old_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| files[at].0.clone());
                files[at].1 = text;
            }
            Err(e) => return write_report(args, &doc, Some(format!("--update: {e}")), Vec::new()),
        }
    }

    // Write.
    if args.dry_run {
        // A preview writes nothing but the report.
        return write_report(args, &doc, None, Vec::new());
    }
    progress.phase("write", Some(files.len()));
    let dir = match std::fs::create_dir_all(&args.output).and_then(|()| args.output.canonicalize())
    {
        Ok(d) => d,
        Err(e) => {
            let msg = format!("creating {}: {e}", args.output.display());
            return write_report(args, &doc, Some(msg), Vec::new());
        }
    };
    let mut guard = Guard::new(&dir);
    let mut written = Vec::new();
    let rels: std::collections::BTreeSet<String> = files.iter().map(|(p, _)| p.clone()).collect();
    // Stale: only what the (valid) record says this converter wrote and the
    // source no longer has, and only as plain regular files.
    let stale: Vec<PathBuf> = usable
        .iter()
        .flat_map(|l| l.files.iter())
        .filter(|f| !rels.contains(*f) && crate::ledger::expected_path(f))
        .map(|f| dir.join(f))
        .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
        .collect();
    let res = write_files(&mut guard, &dir, &files, &stale, progress, &mut written);
    match res {
        Ok(()) => {
            doc.ledger = Some(Ledger {
                library_id: Some(built.library_id),
                rows: built.row_ids.iter().copied().collect(),
                symbols: built.symbol_ids.iter().copied().collect(),
                footprints: built.footprint_ids.iter().copied().collect(),
                files: rels,
            });
            let summary = write_report(args, &doc, None, written);
            if summary.error.is_none() {
                guard.commit();
                summary
            } else {
                let mut msg = summary.error.unwrap_or_default();
                for left in guard.rollback(&dir) {
                    msg.push_str("; ");
                    msg.push_str(&left);
                }
                LibrarySummary {
                    error: Some(msg),
                    ..summary
                }
            }
        }
        Err(e) => {
            let mut msg = format!("{e:#}");
            for left in guard.rollback(&dir) {
                msg.push_str("; ");
                msg.push_str(&left);
            }
            write_report(args, &doc, Some(msg), Vec::new())
        }
    }
}

/// Write the new files, then delete the stale ones.
fn write_files(
    guard: &mut Guard,
    dir: &Path,
    files: &[(String, String)],
    stale: &[PathBuf],
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
    for p in stale {
        guard.remove(p)?;
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
