//! A part row as the cells of a `[tables.<name>]` block, and the library
//! file around them (`LIBRARY_FORMAT.md` sections 2.2 to 2.4).

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use signex_library::component::ComponentRow;
use signex_library::footprint_binding::encode_footprint_cell;
use signex_library::tables::TABLE_HEADER;
use signex_library::{
    ClassEntry, FORMAT_TOKEN, LibraryFile, LibraryMode, LibraryRow, LibrarySection, LibraryTable,
    LifecycleState, SnxlibManifest, UsersConfig, WorkflowConfig,
};
use uuid::Uuid;

use super::BuildError;

/// The time a conversion stamps when no source records a modified time. A
/// conversion is a pure function of its inputs (contract section 5), so it
/// never reads the wall clock.
pub fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

fn time_cell(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn json<T: serde::Serialize>(v: &T) -> Result<String, BuildError> {
    serde_json::to_string(v).map_err(|e| BuildError::Row(e.to_string()))
}

/// The cell for a lifecycle state: the variant's name, as the row codec
/// writes it.
fn state_cell(s: LifecycleState) -> Result<String, BuildError> {
    match serde_json::to_value(s) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        _ => Err(BuildError::Row("lifecycle state has no name".into())),
    }
}

/// The 22 cells of `row`, by column name. The names are checked against the
/// vendored header, so a column the writer adds or moves fails loudly.
pub fn row_cells(row: &ComponentRow) -> Result<LibraryRow, BuildError> {
    let datasheet = if row.datasheet_label.is_empty() && row.more_datasheets.is_empty() {
        json(&row.datasheet)?
    } else {
        json(&row.documents())?
    };
    let footprint = encode_footprint_cell(row.footprint_ref.as_ref(), &row.alternate_footprints)
        .map_err(|e| BuildError::Row(e.to_string()))?;
    let sim = match &row.sim_ref {
        Some(r) => json(r)?,
        None => String::new(),
    };
    let hash: String = row
        .content_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let cells: [(&str, String); 22] = [
        ("row_id", row.row_id.to_string()),
        ("internal_pn", row.internal_pn.as_str().to_string()),
        ("class", row.class.as_str().to_string()),
        ("datasheet", datasheet),
        ("state", state_cell(row.state)?),
        ("symbol_ref", json(&row.symbol_ref)?),
        ("footprint_ref", footprint),
        ("sim_ref", sim),
        ("primary_mpn", json(&row.primary_mpn)?),
        ("alternates", json(&row.alternates)?),
        ("supply", json(&row.supply)?),
        ("parameters", json(&row.parameters)?),
        ("pin_map_overrides", json(&row.pin_map_overrides)?),
        ("created", time_cell(row.created)),
        ("updated", time_cell(row.updated)),
        ("content_hash", hash),
        ("version", row.version.clone()),
        ("released", row.released.to_string()),
        ("symbol_version", row.symbol_version.clone()),
        ("footprint_version", row.footprint_version.clone()),
        ("sim_version", row.sim_version.clone()),
        ("description", row.description.clone()),
    ];
    let names: Vec<&str> = cells.iter().map(|(n, _)| *n).collect();
    if names != TABLE_HEADER {
        return Err(BuildError::Row(
            "the vendored table header changed; update row_cells".into(),
        ));
    }
    let mut out = LibraryRow::default();
    for (col, cell) in cells {
        out.cells.insert(col.to_string(), cell);
    }
    if !row.alternate_symbols.is_empty() {
        out.cells.insert(
            signex_library::tables::ALTERNATE_SYMBOLS_COLUMN.to_string(),
            json(&row.alternate_symbols)?,
        );
    }
    Ok(out)
}

/// The library file text: manifest, then one table per class.
pub fn library_text(
    library_id: Uuid,
    name: &str,
    classes: &BTreeMap<String, &'static str>,
    rows: BTreeMap<String, Vec<LibraryRow>>,
) -> Result<String, BuildError> {
    let manifest = SnxlibManifest {
        format: FORMAT_TOKEN.to_string(),
        library_id,
        library: LibrarySection {
            name: name.to_string(),
            description: None,
            ..LibrarySection::default()
        },
        mode: LibraryMode::default(),
        workflow: WorkflowConfig::default(),
        users: UsersConfig {
            default_role: "Designer".to_string(),
            ..UsersConfig::default()
        },
        classes: classes
            .iter()
            .map(|(key, label)| ClassEntry {
                key: key.clone(),
                label: (*label).to_string(),
            })
            .collect(),
    };
    let mut tables = BTreeMap::new();
    for (class, mut rows) in rows {
        rows.sort_by(|a, b| a.cells["row_id"].cmp(&b.cells["row_id"]));
        let mut columns: Vec<String> = TABLE_HEADER.iter().map(|c| (*c).to_string()).collect();
        let alt = signex_library::tables::ALTERNATE_SYMBOLS_COLUMN;
        if rows.iter().any(|r| r.cells.contains_key(alt)) {
            columns.push(alt.to_string());
        }
        tables.insert(
            class,
            LibraryTable {
                columns,
                rows: rows.into_iter().map(Arc::new).collect(),
                ..Default::default()
            },
        );
    }
    let file = LibraryFile {
        manifest,
        tables,
        layout: Default::default(),
    };
    let text = file
        .write()
        .map_err(|e| BuildError::Library(e.to_string()))?;
    // The text must be what the writer would write for what it reads back.
    let again = LibraryFile::parse(&text)
        .and_then(|f| f.write())
        .map_err(|e| BuildError::Library(e.to_string()))?;
    if again != text {
        return Err(BuildError::Library("output is not canonical".into()));
    }
    Ok(text)
}
