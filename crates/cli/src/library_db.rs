//! A `.DbLib` input: read the database library, find the database and the
//! libraries its rows name, and hand the rows to the library builder.

use std::path::{Path, PathBuf};

use altium_format::intlib::IntLib;
use altium_format::limits::Budget;
use altium_format::pcb::PcbLib;
use altium_format::sch::SchLib;
use signex_altium_convert::Report;
use signex_altium_convert::dblib::{
    DbLib, DbSource, MapFile, PrepareInput, Resolver, parse_dblib, prepare,
};
use signex_altium_convert::libset::{SourceBody, SourceLib};

use crate::library_cmd::LibraryArgs;
use crate::progress::Progress;
use crate::report_doc::SourceFile;

/// Largest `.DbLib` or mapping file read.
const MAX_TEXT: u64 = 64 * 1024 * 1024;

pub struct Database {
    pub sources: Vec<SourceLib>,
    pub files: Vec<SourceFile>,
    pub mapping: MapFile,
}

fn read_text(p: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if !meta.is_file() || meta.len() > MAX_TEXT {
        return Err(format!(
            "{} is not a plain file of a sane size",
            p.display()
        ));
    }
    let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    // DbLib files are written in the local ANSI code page (Windows-1252 here).
    Ok(match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => encoding_rs::WINDOWS_1252
            .decode(e.as_bytes())
            .0
            .into_owned(),
    })
}

/// `Data Source=<path>` of a connection string.
fn data_source(conn: &str) -> Option<String> {
    conn.split(';').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case("Data Source")
            .then(|| v.trim().to_string())
    })
}

/// The database to read: `--db` (a folder of CSV files, an Access file, or a
/// connection string naming one), else what the `.DbLib` stores.
fn locate(args: &LibraryArgs, db: &DbLib, resolver: &Resolver) -> Result<DbSource, String> {
    if let Some(over) = &args.db {
        let candidate = data_source(over).unwrap_or_else(|| over.clone());
        return Path::new(&candidate)
            .exists()
            .then(|| DbSource::from_path(Path::new(&candidate)))
            .flatten()
            .or_else(|| {
                resolver
                    .resolve(&candidate)
                    .and_then(|p| DbSource::from_path(&p))
            })
            .ok_or_else(|| format!("--db {over}: not a folder of .csv files or an Access file"));
    }
    let l = &db.links;
    let given = if l.database_path.is_empty() {
        data_source(&l.connection_string).unwrap_or_default()
    } else {
        l.database_path.clone()
    };
    if given.is_empty() {
        return Err("the database library stores no database path; pass --db".into());
    }
    let kind = l.database_type.to_lowercase();
    if !kind.is_empty() && !kind.contains("access") {
        return Err(format!(
            "database type {:?} is not supported; export the tables to .csv files and pass --db <folder>",
            l.database_type
        ));
    }
    resolver
        .resolve(&given)
        .and_then(|p| DbSource::from_path(&p))
        .ok_or_else(|| {
            format!("database {given} not found (as given, beside the library or in the search paths); pass --db")
        })
}

fn load(path: &Path, budget: &mut Budget) -> Result<Vec<(SourceBody, bool)>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let err = |e: altium_format::Error| format!("{}: {e}", path.display());
    match ext.as_str() {
        "schlib" => Ok(vec![(
            SourceBody::Sch(SchLib::open(path).map_err(err)?),
            false,
        )]),
        "pcblib" => Ok(vec![(
            SourceBody::Pcb(PcbLib::open_with(path, budget).map_err(err)?),
            false,
        )]),
        "intlib" => {
            let int = IntLib::open_with(path, budget).map_err(err)?;
            let mut out: Vec<(SourceBody, bool)> = Vec::new();
            out.extend(
                int.schlibs
                    .into_iter()
                    .map(|e| (SourceBody::Sch(e.lib), true)),
            );
            out.extend(
                int.pcblibs
                    .into_iter()
                    .map(|e| (SourceBody::Pcb(e.lib), true)),
            );
            Ok(out)
        }
        _ => Err(format!(
            "{}: not a schematic, footprint or integrated library",
            path.display()
        )),
    }
}

pub fn read_database(
    args: &LibraryArgs,
    dblib_path: &Path,
    budget: &mut Budget,
    report: &mut Report,
    progress: &Progress,
) -> Result<Database, String> {
    let text = read_text(dblib_path)?;
    let db = parse_dblib(&text).map_err(|e| format!("{}: {e}", dblib_path.display()))?;
    let dir: PathBuf = dblib_path
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let resolver = Resolver::new(
        &dir,
        &args.search_path,
        &db.links.search_path,
        db.links.search_subdirs,
    );
    let source = locate(args, &db, &resolver)?;
    let user_map = match &args.map {
        Some(m) => MapFile::parse(&read_text(m)?)?,
        None => MapFile::default(),
    };
    let input = PrepareInput {
        dblib: &db,
        source: &source,
        resolver: &resolver,
        user_map: &user_map,
    };
    let prepared = prepare(
        &input,
        &mut |p| load(p, budget),
        report,
        &mut |i, n, name| progress.progress(i, Some(n), Some(name)),
    )?;
    let mut files = vec![SourceFile::hash(dblib_path).map_err(|e| format!("{e:#}"))?];
    if source.path().is_file() {
        files.push(SourceFile::hash(source.path()).map_err(|e| format!("{e:#}"))?);
    }
    for f in &prepared.files {
        files.push(SourceFile::hash(f).map_err(|e| format!("{e:#}"))?);
    }
    Ok(Database {
        sources: prepared.sources,
        files,
        mapping: prepared.mapping,
    })
}
