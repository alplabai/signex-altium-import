//! A database library to rows and sources: every enabled table's rows are
//! resolved against the schematic and footprint libraries they name, each
//! library read once, and a row that cannot be resolved is skipped and
//! reported (`error`, so the run exits 2).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::map::{KEY, MapFile, SKIP, map_table};
use super::parse::DbLib;
use super::resolve::Resolver;
use super::source::DbSource;
use crate::libset::{DatabaseSet, DbRow, SourceBody, SourceLib};
use crate::report::{Item, ItemKind, Report, Severity};

pub struct PrepareInput<'a> {
    pub dblib: &'a DbLib,
    pub source: &'a DbSource,
    pub resolver: &'a Resolver,
    pub user_map: &'a MapFile,
}

pub struct Prepared {
    /// The libraries read (once each), then one set of rows per table.
    pub sources: Vec<SourceLib>,
    /// The mapping every table used.
    pub mapping: MapFile,
    /// Library files read, for the report's sources.
    pub files: Vec<PathBuf>,
}

/// A reserved target (`[Library Ref]`, `[Footprint Path 2]`, ...).
#[derive(Debug, PartialEq)]
enum Reserved {
    LibraryRef,
    LibraryPath,
    FootprintRef(usize),
    FootprintPath(usize),
    Description,
    Sim,
    Unknown,
}

fn reserved(target: &str) -> Option<Reserved> {
    let inner = target.trim().strip_prefix('[')?.strip_suffix(']')?;
    let norm = inner
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let numbered = |base: &str| -> Option<usize> {
        let rest = norm.strip_prefix(base)?.trim();
        if rest.is_empty() {
            Some(1)
        } else {
            rest.parse().ok().filter(|n| *n >= 1)
        }
    };
    Some(if norm == "library ref" {
        Reserved::LibraryRef
    } else if norm == "library path" {
        Reserved::LibraryPath
    } else if let Some(n) = numbered("footprint ref") {
        Reserved::FootprintRef(n)
    } else if let Some(n) = numbered("footprint path") {
        Reserved::FootprintPath(n)
    } else if norm == "description" {
        Reserved::Description
    } else if norm.starts_with("sim") {
        Reserved::Sim
    } else {
        Reserved::Unknown
    })
}

type Loaded = Vec<(SourceBody, bool)>;
/// A library's schematic and footprint body: index in `sources`, and name.
type Slots = (Option<(usize, String)>, Option<(usize, String)>);

struct Library {
    /// Index of the schematic / footprint body in `sources`, with its name.
    sch: Option<(usize, String)>,
    pcb: Option<(usize, String)>,
}

fn item(
    kind: ItemKind,
    code: &'static str,
    sev: Severity,
    source: String,
    message: String,
) -> Item {
    Item {
        kind,
        source,
        id: None,
        code,
        severity: sev,
        message,
        detail: Value::Null,
    }
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("library")
        .to_string()
}

pub fn prepare(
    input: &PrepareInput,
    load: &mut dyn FnMut(&Path) -> Result<Loaded, String>,
    report: &mut Report,
    on_table: &mut dyn FnMut(usize, usize, &str),
) -> Result<Prepared, String> {
    let mut sources: Vec<SourceLib> = Vec::new();
    let mut libs: HashMap<PathBuf, Result<Library, String>> = HashMap::new();
    let mut used_names: HashSet<(bool, String)> = HashSet::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut mapping = MapFile::default();
    let mut sets: Vec<SourceLib> = Vec::new();

    let mut open = |path: &Path,
                    sources: &mut Vec<SourceLib>,
                    files: &mut Vec<PathBuf>|
     -> Result<Slots, String> {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !libs.contains_key(&key) {
            let res = load(&key).map(|bodies| {
                let mut lib = Library {
                    sch: None,
                    pcb: None,
                };
                for (body, released) in bodies {
                    let is_sch = matches!(body, SourceBody::Sch(_));
                    let base = stem(&key);
                    let mut name = base.clone();
                    let mut n = 1;
                    while !used_names.insert((is_sch, name.to_lowercase())) {
                        n += 1;
                        name = format!("{base}~{n}");
                    }
                    let slot = (sources.len(), name.clone());
                    sources.push(SourceLib {
                        released,
                        path: key.to_string_lossy().into_owned(),
                        name,
                        body,
                    });
                    if is_sch {
                        lib.sch.get_or_insert(slot);
                    } else {
                        lib.pcb.get_or_insert(slot);
                    }
                }
                files.push(key.clone());
                lib
            });
            libs.insert(key.clone(), res);
        }
        match &libs[&key] {
            Ok(l) => Ok((l.sch.clone(), l.pcb.clone())),
            Err(e) => Err(e.clone()),
        }
    };

    if input.resolver.truncated() {
        report.push(item(
            ItemKind::Library,
            "library.search_truncated",
            Severity::Info,
            String::new(),
            "a search folder holds more files than the listing limit; later files were not seen"
                .into(),
        ));
    }
    let total = input.dblib.tables.len();
    for (ti, table) in input.dblib.tables.iter().enumerate() {
        on_table(ti, total, &table.name);
        if !table.enabled {
            report.push(item(
                ItemKind::Library,
                "table.disabled",
                Severity::Info,
                table.name.clone(),
                format!(
                    "table {} is disabled in the database library; skipped",
                    table.name
                ),
            ));
            continue;
        }
        let data = match input.source.read(&table.name) {
            Ok(Some(d)) => d,
            Ok(None) => {
                report.push(item(
                    ItemKind::Library,
                    "table.missing",
                    Severity::Error,
                    table.name.clone(),
                    format!("table {} is not in the database", table.name),
                ));
                continue;
            }
            Err(e) => return Err(format!("reading table {}: {e}", table.name)),
        };
        for col in &data.lossy_columns {
            report.push(item(
                ItemKind::Library,
                "table.lossy_text",
                Severity::Info,
                table.name.clone(),
                format!("column {col} held bytes that are not valid UTF-8; they were replaced"),
            ));
        }
        let targets = map_table(table, &data, input.user_map);
        mapping.tables.insert(
            table.name.clone(),
            data.columns
                .iter()
                .cloned()
                .zip(targets.iter().cloned())
                .collect(),
        );
        let key_col = targets.iter().position(|t| t == KEY);
        // A stable order that does not depend on the database's physical row
        // order (a compacted .mdb reorders rows): by key, then every cell.
        // Each row keeps its position in the database for its report label.
        let mut ordered: Vec<(usize, &Vec<String>)> = data.rows.iter().enumerate().collect();
        ordered.sort_by(|(_, a), (_, b)| {
            let k = |r: &Vec<String>| key_col.and_then(|i| r.get(i)).cloned().unwrap_or_default();
            k(a).cmp(&k(b)).then_with(|| a.cmp(b))
        });
        let mut set = DatabaseSet {
            table: crate::text::cell_safe(&table.name).0,
            released: true,
            ..Default::default()
        };
        if key_col.is_none() {
            report.push(item(
                ItemKind::Library,
                "table.no_key",
                Severity::Error,
                table.name.clone(),
                format!(
                    "table {} has no key field (FieldType 0); its rows are skipped",
                    table.name
                ),
            ));
        }
        let mut sim_noted = false;
        let mut unknown_noted: HashSet<String> = HashSet::new();
        for (ri, row) in ordered {
            let cell = |i: usize| row.get(i).map_or("", |s| s.trim());
            let key = key_col.map_or("", cell).to_string();
            let label = if key.is_empty() {
                format!("{}:row {}", table.name, ri + 1)
            } else {
                format!("{}:{key}", table.name)
            };
            let skip =
                |set: &mut DatabaseSet, report: &mut Report, code: &'static str, msg: String| {
                    set.skipped += 1;
                    report.push(item(
                        ItemKind::Row,
                        code,
                        Severity::Error,
                        label.clone(),
                        msg,
                    ));
                };
            if key.is_empty() {
                skip(
                    &mut set,
                    report,
                    "row.key_missing",
                    "the row has no key value; skipped".into(),
                );
                continue;
            }
            let (mut lib_ref, mut lib_path, mut description) =
                (String::new(), String::new(), String::new());
            let mut fp_ref: HashMap<usize, String> = HashMap::new();
            let mut fp_path: HashMap<usize, String> = HashMap::new();
            let mut params: Vec<(String, String)> = Vec::new();
            for (i, target) in targets.iter().enumerate() {
                let v = cell(i);
                if target == KEY || target == SKIP || v.is_empty() {
                    continue;
                }
                match reserved(target) {
                    Some(Reserved::LibraryRef) => lib_ref = v.to_string(),
                    Some(Reserved::LibraryPath) => lib_path = v.to_string(),
                    Some(Reserved::FootprintRef(n)) => {
                        fp_ref.insert(n, v.to_string());
                    }
                    Some(Reserved::FootprintPath(n)) => {
                        fp_path.insert(n, v.to_string());
                    }
                    Some(Reserved::Description) => description = v.to_string(),
                    Some(Reserved::Sim) => {
                        if !sim_noted {
                            sim_noted = true;
                            report.push(item(
                                ItemKind::Library,
                                "table.sim_skipped",
                                Severity::Info,
                                table.name.clone(),
                                format!(
                                    "simulation fields of table {} are not imported",
                                    table.name
                                ),
                            ));
                        }
                    }
                    Some(Reserved::Unknown) => {
                        // Kept as a parameter named like the target, noted once.
                        if unknown_noted.insert(target.clone()) {
                            report.push(item(
                                ItemKind::Library,
                                "table.reserved_unknown",
                                Severity::Info,
                                table.name.clone(),
                                format!("{target} is not a reserved name this converter knows; kept as a parameter"),
                            ));
                        }
                        params.push((target.trim_matches(['[', ']']).to_string(), v.to_string()));
                    }
                    None => params.push((target.clone(), v.to_string())),
                }
            }

            // The symbol.
            if lib_ref.is_empty() || lib_path.is_empty() {
                skip(
                    &mut set,
                    report,
                    "row.symbol_missing",
                    "the row names no symbol (library reference and path); skipped".into(),
                );
                continue;
            }
            let Some(sym_file) = input.resolver.resolve(&lib_path) else {
                skip(
                    &mut set,
                    report,
                    "row.symbol_library_missing",
                    format!("symbol library {lib_path} not found; skipped"),
                );
                continue;
            };
            let mut unreadable_why: Option<String> = None;
            let symbol = match open(&sym_file, &mut sources, &mut files) {
                Ok((Some((idx, name)), _)) => {
                    let SourceBody::Sch(l) = &sources[idx].body else {
                        unreachable!()
                    };
                    unreadable_why = l
                        .unreadable
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case(&lib_ref))
                        .map(|(_, why)| why.clone());
                    l.components
                        .iter()
                        .find(|c| c.lib_reference.eq_ignore_ascii_case(&lib_ref))
                        .map(|c| (name, c.lib_reference.clone()))
                }
                Ok((None, _)) => None,
                Err(e) => {
                    skip(
                        &mut set,
                        report,
                        "row.symbol_library_unreadable",
                        format!("symbol library {lib_path} unreadable: {e}; skipped"),
                    );
                    continue;
                }
            };
            let Some(symbol) = symbol else {
                if let Some(why) = unreadable_why {
                    skip(
                        &mut set,
                        report,
                        "part.symbol_unreadable",
                        format!("symbol {lib_ref} in {lib_path} could not be read: {why}; skipped"),
                    );
                    continue;
                }
                skip(
                    &mut set,
                    report,
                    "row.symbol_missing",
                    format!("symbol {lib_ref} not found in {lib_path}; skipped"),
                );
                continue;
            };

            // The footprints, default first.
            let mut numbers: Vec<usize> = fp_ref.keys().chain(fp_path.keys()).copied().collect();
            numbers.sort_unstable();
            numbers.dedup();
            let mut footprints = Vec::new();
            let mut failed: Option<(&'static str, String)> = None;
            for n in numbers {
                let (Some(name), Some(path)) = (fp_ref.get(&n), fp_path.get(&n)) else {
                    failed = Some((
                        "row.footprint_library_missing",
                        format!("footprint {n} names only one of reference and path; skipped"),
                    ));
                    break;
                };
                let Some(file) = input.resolver.resolve(path) else {
                    failed = Some((
                        "row.footprint_library_missing",
                        format!("footprint library {path} not found; skipped"),
                    ));
                    break;
                };
                match open(&file, &mut sources, &mut files) {
                    Ok((_, Some((idx, lib_name)))) => {
                        let SourceBody::Pcb(l) = &sources[idx].body else {
                            unreachable!()
                        };
                        match l
                            .footprints
                            .iter()
                            .find(|f| f.name.eq_ignore_ascii_case(name))
                        {
                            Some(f) => footprints.push((lib_name, f.name.clone())),
                            None => {
                                failed = Some((
                                    "row.footprint_missing",
                                    format!("footprint {name} not found in {path}; skipped"),
                                ));
                                break;
                            }
                        }
                    }
                    Ok((_, None)) => {
                        failed = Some((
                            "row.footprint_library_missing",
                            format!("{path} holds no footprint library; skipped"),
                        ));
                        break;
                    }
                    Err(e) => {
                        failed = Some((
                            "row.footprint_library_unreadable",
                            format!("footprint library {path} unreadable: {e}; skipped"),
                        ));
                        break;
                    }
                }
            }
            if let Some((code, msg)) = failed {
                skip(&mut set, report, code, msg);
                continue;
            }
            set.rows.push(DbRow {
                key,
                description,
                params,
                symbol,
                footprints,
            });
        }
        sets.push(SourceLib {
            released: true,
            path: String::new(),
            name: set.table.clone(),
            body: SourceBody::Database(set),
        });
    }
    let dbpath = input.source.path().to_string_lossy().into_owned();
    for s in &mut sets {
        s.path = dbpath.clone();
    }
    // Across tables by name, so a repeated key gets its suffix in a fixed order.
    sets.sort_by(|a, b| a.name.cmp(&b.name));
    sources.extend(sets);
    Ok(Prepared {
        sources,
        mapping,
        files,
    })
}
