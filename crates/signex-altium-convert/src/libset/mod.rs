//! SchLib and PcbLib sources to one Signex library: symbol envelopes, one
//! footprint file per footprint, and a `.snxlib` with a part row per
//! schematic component (`docs/CONVERSION.md`, "Library conversion").
//!
//! A conversion is a pure function of its inputs: ids are derived, rows and
//! envelopes carry no wall clock, and the same sources give the same bytes.

mod bind;
mod classes;
mod fields;
mod ids;
mod snxlib;

use std::collections::{BTreeMap, HashMap};

use altium_format::pcb::PcbLib;
use altium_format::sch::SchLib;
use chrono::{DateTime, Timelike, Utc};
use serde_json::json;
use signex_library::component::{ComponentRow, DESIGNATOR_PREFIX_PARAM, PlmReserved};
use signex_library::param::ParamValue;
use signex_library::primitive::Footprint;
use signex_library::primitive::symbol::{Symbol, SymbolFile};
use signex_library::templates::TemplateRegistry;
use signex_library::{ComponentClass, FootprintFile, InternalPn, LifecycleState, PrimitiveRef};
use uuid::Uuid;

use crate::library::{footprints_scoped, symbols_scoped};
use crate::report::{Item, ItemKind, Report, Severity};
use bind::{FootprintIndex, bind};
use snxlib::{epoch, library_text, row_cells};

pub use ids::library_id;

/// Version every converted primitive and part starts at.
const VERSION: &str = "1.0.0";

pub enum SourceBody {
    Sch(SchLib),
    Pcb(PcbLib),
}

/// One source file, already read.
pub struct SourceLib {
    /// Sort key and tie-break for collisions (the file's path).
    pub path: String,
    /// The file's stem; names its items in the report (`<name>:<item>`).
    pub name: String,
    pub body: SourceBody,
}

pub struct LibraryOptions {
    /// The library display name.
    pub name: String,
    /// Overrides the id derived from the source file names.
    pub library_id: Option<Uuid>,
    /// The parts of the library an `--update` run refreshes, by row id.
    pub previous: HashMap<Uuid, PreviousRow>,
    /// The symbols and footprints of the library an `--update` refreshes.
    pub previous_symbols: HashMap<Uuid, PreviousPrimitive>,
    pub previous_footprints: HashMap<Uuid, PreviousPrimitive>,
    /// The time stamped on a part or primitive an update changed.
    pub now: DateTime<Utc>,
}

impl LibraryOptions {
    /// A fresh conversion: nothing before it.
    pub fn fresh(name: impl Into<String>) -> Self {
        LibraryOptions {
            name: name.into(),
            library_id: None,
            previous: HashMap::new(),
            previous_symbols: HashMap::new(),
            previous_footprints: HashMap::new(),
            now: epoch(),
        }
    }
}

/// What an earlier run wrote for a part: an update keeps its lifecycle
/// state and creation time, and keeps its version and update time while
/// its content hash is unchanged (contract section 5).
#[derive(Debug, Clone)]
pub struct PreviousRow {
    pub state: LifecycleState,
    pub released: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub version: String,
    pub content_hash: [u8; 32],
    pub symbol_version: String,
    pub footprint_version: String,
}

/// What an earlier run wrote for a symbol or footprint: its version and
/// times, and a fingerprint of its content apart from them.
#[derive(Debug, Clone)]
pub struct PreviousPrimitive {
    pub version: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub fingerprint: String,
}

fn symbol_text(sym: &Symbol) -> String {
    let mut s = sym.clone();
    s.version = String::new();
    s.created = epoch();
    s.updated = epoch();
    let file = SymbolFile {
        format: "snxsym/v1".into(),
        file_uuid: Uuid::nil(),
        display_name: String::new(),
        copied_from: None,
        symbols: vec![s],
        created: epoch(),
        updated: epoch(),
    };
    file.to_toml_string().unwrap_or_default()
}

fn footprint_text(fp: &Footprint) -> String {
    let mut f = fp.clone();
    f.version = String::new();
    f.created = epoch();
    f.updated = epoch();
    FootprintFile::with_identity(f, Uuid::nil(), epoch())
        .to_toml_string()
        .unwrap_or_default()
}

impl PreviousPrimitive {
    pub fn of_symbol(sym: &Symbol) -> Self {
        PreviousPrimitive {
            version: sym.version.clone(),
            created: sym.created,
            updated: sym.updated,
            fingerprint: symbol_text(sym),
        }
    }

    pub fn of_footprint(fp: &Footprint) -> Self {
        PreviousPrimitive {
            version: fp.version.clone(),
            created: fp.created,
            updated: fp.updated,
            fingerprint: footprint_text(fp),
        }
    }
}

/// Version and times of a freshly converted primitive: new ones start at
/// `1.0.0` stamped `stamp`; one an update finds on disk keeps its version
/// and times while its content is unchanged, else the patch version rises
/// and `updated` is `now`.
fn stamp_primitive(
    previous: Option<&PreviousPrimitive>,
    fingerprint: &str,
    stamp: DateTime<Utc>,
    now: DateTime<Utc>,
) -> (String, DateTime<Utc>, DateTime<Utc>) {
    match previous {
        None => (VERSION.into(), stamp, stamp),
        Some(p) if p.fingerprint == fingerprint => (p.version.clone(), p.created, p.updated),
        Some(p) => (bump_patch(&p.version), p.created, now),
    }
}

/// The newest container modified time across the sources, in whole
/// seconds: deterministic, from the input files. The Unix epoch when none
/// records one.
fn source_stamp(sources: &[SourceLib]) -> DateTime<Utc> {
    sources
        .iter()
        .filter_map(|s| match &s.body {
            SourceBody::Sch(l) => l.modified,
            SourceBody::Pcb(l) => l.modified,
        })
        .max()
        .map(|t| {
            DateTime::<Utc>::from(t)
                .with_nanosecond(0)
                .unwrap_or_else(epoch)
        })
        .unwrap_or_else(epoch)
}

/// `1.0.3` to `1.0.4`; a version that is not `X.Y.Z` is returned as is.
fn bump_patch(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    match parts.as_slice() {
        [a, b, c] => match c.parse::<u64>() {
            Ok(n) => format!("{a}.{b}.{}", n + 1),
            Err(_) => v.to_string(),
        },
        _ => v.to_string(),
    }
}

/// Item counts of a library.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub symbols: usize,
    pub footprints: usize,
    pub parts: usize,
    pub models: usize,
}

/// A converted library: the files to write, relative to the library root.
pub struct Built {
    pub library_id: Uuid,
    pub files: Vec<(String, String)>,
    pub source: Counts,
    pub written: Counts,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("{0}")]
    Row(String),
    #[error("{0}")]
    Library(String),
}

/// File name of an envelope or library: ASCII letters and digits in lower
/// case, every other run replaced by one `-` (`LIBRARY_FORMAT.md` section 1).
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

fn error_item(code: &'static str, source: String, message: String, kind: ItemKind) -> Item {
    Item {
        kind,
        source,
        id: None,
        code,
        severity: Severity::Error,
        message,
        detail: serde_json::Value::Null,
    }
}

pub fn build_library(
    sources: &[SourceLib],
    opts: &LibraryOptions,
    report: &mut Report,
    on_item: &mut dyn FnMut(usize, usize, &str),
) -> Result<Built, BuildError> {
    let total: usize = sources
        .iter()
        .map(|s| match &s.body {
            SourceBody::Sch(l) => l.components.len(),
            SourceBody::Pcb(l) => l.footprints.len(),
        })
        .sum();
    let mut done = 0;
    let stamp = source_stamp(sources);
    let mut footprint_versions: HashMap<Uuid, String> = HashMap::new();
    let mut sorted: Vec<&SourceLib> = sources.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let file_names: Vec<String> = sources
        .iter()
        .map(|s| {
            s.path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&s.path)
                .to_string()
        })
        .collect();
    let lib_id = opts
        .library_id
        .unwrap_or_else(|| ids::library_id(&file_names));
    let mut files: Vec<(String, String)> = Vec::new();
    let mut source = Counts::default();
    let mut written = Counts::default();

    // Footprints first: parts bind to them by name.
    let mut index = FootprintIndex::default();
    for s in &sorted {
        let SourceBody::Pcb(lib) = &s.body else {
            continue;
        };
        let names: Vec<&str> = lib.footprints.iter().map(|f| f.name.as_str()).collect();
        let ids = ids::ids_for(lib_id, "footprint", &s.name, &names);
        source.footprints += names.len();
        let fps = footprints_scoped(lib, &s.name, &ids, report);
        for mut fp in fps {
            let (version, created, updated) = stamp_primitive(
                opts.previous_footprints.get(&fp.uuid),
                &footprint_text(&fp),
                stamp,
                opts.now,
            );
            fp.version = version;
            fp.created = created;
            fp.updated = updated;
            footprint_versions.insert(fp.uuid, fp.version.clone());
            let here = format!("{}:{}", s.name, fp.name);
            if let Err(first) = index.insert(&fp.name, fp.uuid, &here) {
                report.set_scope_with_id(here.clone(), fp.uuid);
                report.warn_detail(
                    "footprint.name_collision",
                    format!("footprint {here} repeats {first}; parts bind to {first}"),
                    json!({ "first": first, "repeat": here }),
                );
                report.set_scope("");
            }
            let file_id = ids::item_id(lib_id, "footprint-file", &fp.uuid.to_string());
            let rel = format!("footprints/{}.snxfpt", fp.uuid);
            let file = FootprintFile::with_identity(fp, file_id, stamp);
            let text = file
                .to_toml_string()
                .map_err(|e| BuildError::Library(e.to_string()))?;
            files.push((rel, text));
            written.footprints += 1;
            done += 1;
            on_item(
                done,
                total,
                &format!("{}:{}", s.name, file.footprints[0].name),
            );
        }
    }

    // Symbols and parts.
    let templates = TemplateRegistry::new_with_builtins();
    let mut taken_pns: HashMap<String, usize> = HashMap::new();
    let mut rows: BTreeMap<&'static str, Vec<signex_library::LibraryRow>> = BTreeMap::new();
    let mut classes: BTreeMap<&'static str, &'static str> = BTreeMap::new();
    let mut used_slugs: HashMap<String, usize> = HashMap::new();
    for s in &sorted {
        let SourceBody::Sch(lib) = &s.body else {
            continue;
        };
        let names: Vec<&str> = lib
            .components
            .iter()
            .map(|c| c.lib_reference.as_str())
            .collect();
        source.symbols += names.len();
        source.parts += names.len();
        let sym_ids = ids::ids_for(lib_id, "symbol", &s.name, &names);
        let part_ids = ids::ids_for(lib_id, "part", &s.name, &names);
        let mut symbols = symbols_scoped(lib, &s.name, &sym_ids, report);
        for sym in &mut symbols {
            let (version, created, updated) = stamp_primitive(
                opts.previous_symbols.get(&sym.uuid),
                &symbol_text(sym),
                stamp,
                opts.now,
            );
            sym.version = version;
            sym.created = created;
            sym.updated = updated;
        }
        let sym_versions: Vec<String> = symbols.iter().map(|s| s.version.clone()).collect();
        written.symbols += symbols.len();
        if !symbols.is_empty() {
            let file_uuid = ids::item_id(lib_id, "symbol-file", &s.name);
            let mut rel = format!("symbols/{}.snxsym", slug(&s.name));
            let n = used_slugs.entry(rel.clone()).or_default();
            *n += 1;
            if *n > 1 {
                rel = format!("symbols/{file_uuid}.snxsym");
            }
            let env = SymbolFile {
                format: "snxsym/v1".into(),
                file_uuid,
                display_name: s.name.clone(),
                copied_from: None,
                symbols,
                created: stamp,
                updated: stamp,
            };
            let text = env
                .to_toml_string()
                .map_err(|e| BuildError::Library(e.to_string()))?;
            files.push((rel, text));
        }

        for (i, comp) in lib.components.iter().enumerate() {
            let source_name = format!("{}:{}", s.name, comp.lib_reference);
            done += 1;
            on_item(done, total, &source_name);
            let row_id = part_ids[i];
            report.set_scope_with_id(source_name.clone(), row_id);

            let Some(binding) = bind(comp, &index, report) else {
                // The error item says why; the part is not written.
                report.set_scope("");
                continue;
            };

            let prefix = classes::designator_prefix(
                comp.designator.as_ref().map_or("", |d| d.text.as_str()),
            );
            let class = classes::class_for_prefix(&prefix);
            let template = templates
                .resolve(Uuid::nil(), class)
                .or_else(|| templates.resolve(Uuid::nil(), "generic"));
            let f = fields::map_fields(comp, template, report);
            let mut parameters = f.parameters;
            if !prefix.is_empty() {
                parameters.insert(DESIGNATOR_PREFIX_PARAM.into(), ParamValue::Text(prefix));
            }
            if let Some(t) = template {
                let missing: Vec<&str> = t
                    .required_params
                    .iter()
                    .filter(|p| !parameters.contains_key(&p.name))
                    .map(|p| p.name.as_str())
                    .collect();
                if !missing.is_empty() {
                    report.warn_detail(
                        "part.template_incomplete",
                        format!("the {class} template wants {}", missing.join(", ")),
                        json!({ "class": class, "missing": missing }),
                    );
                }
            }

            // Part number: the component name, `~2`, `~3` for repeats in
            // (source path, source order).
            let base = comp.lib_reference.clone();
            let n = taken_pns.entry(base.clone()).or_default();
            *n += 1;
            let pn = if *n == 1 {
                base.clone()
            } else {
                format!("{base}~{n}")
            };
            if *n > 1 {
                report.warn_detail(
                    "part.pn_collision",
                    format!("part number {base} is taken; this part is {pn}"),
                    json!({ "part_number": pn, "name": base }),
                );
            }

            let mut row = ComponentRow {
                row_id,
                internal_pn: InternalPn::new(pn),
                class: ComponentClass::new(class),
                datasheet: Default::default(),
                datasheet_label: String::new(),
                description: f.description,
                more_datasheets: Vec::new(),
                state: LifecycleState::Draft,
                symbol_ref: PrimitiveRef::new(lib_id, sym_ids[i]),
                alternate_symbols: Vec::new(),
                footprint_ref: binding.default.map(|id| PrimitiveRef::new(lib_id, id)),
                alternate_footprints: binding.alternate_bindings(lib_id, &footprint_versions),
                sim_ref: None,
                pin_map_overrides: binding.default_overrides,
                primary_mpn: f.primary,
                alternates: f.alternates,
                supply: f.supply,
                parameters,
                plm: PlmReserved::default(),
                version: VERSION.into(),
                released: false,
                symbol_version: sym_versions[i].clone(),
                footprint_version: binding
                    .default
                    .and_then(|id| footprint_versions.get(&id).cloned())
                    .unwrap_or_default(),
                sim_version: String::new(),
                created: stamp,
                updated: stamp,
                content_hash: [0; 32],
                server_revision: None,
            };
            row.set_documents(f.docs);
            let previous = opts.previous.get(&row_id);
            if let Some(p) = previous {
                row.state = p.state;
                row.released = p.released;
                row.created = p.created;
                row.version = p.version.clone();
                row.updated = p.updated;
            }
            let made = signex_library::validate::validate_row_for_write(&row)
                .map_err(|e| BuildError::Row(e.to_string()))
                .and_then(|()| {
                    row.content_hash = signex_library::hash_row_content(&row)
                        .map_err(|e| BuildError::Row(e.to_string()))?;
                    if previous.is_some_and(|p| {
                        p.content_hash != row.content_hash
                            || p.symbol_version != row.symbol_version
                            || p.footprint_version != row.footprint_version
                    }) {
                        row.version = bump_patch(&row.version);
                        row.updated = opts.now;
                    }
                    row_cells(&row)
                });
            report.set_scope("");
            match made {
                Ok(cells) => {
                    classes.insert(class, classes::class_label(class));
                    rows.entry(class).or_default().push(cells);
                    written.parts += 1;
                }
                Err(e) => report.push(error_item(
                    "part.row_invalid",
                    source_name,
                    format!("part not written: {e}"),
                    ItemKind::Part,
                )),
            }
        }
    }

    let name = opts.name.trim();
    let name = if name.is_empty() { "library" } else { name };
    let text = library_text(lib_id, name, &classes, rows)?;
    files.push((format!("{}.snxlib", slug(name)), text));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Built {
        library_id: lib_id,
        files,
        source,
        written,
    })
}
