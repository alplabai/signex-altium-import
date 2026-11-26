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

/// The time every fresh conversion stamps. A conversion is a pure function
/// of its inputs (contract section 5), so it carries no wall clock.
pub fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

fn time_cell(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn json<T: serde::Serialize>(v: &T) -> Result<String, BuildError> {
    serde_json::to_string(v).map_err(|e| BuildError::Row(e.to_string()))
}

fn state_cell(s: LifecycleState) -> &'static str {
    match s {
        LifecycleState::InReview => "InReview",
        LifecycleState::Released => "Released",
        LifecycleState::Deprecated => "Deprecated",
        LifecycleState::Obsolete => "Obsolete",
        _ => "Draft",
    }
}

/// The 22 cells of `row`, by column name.
pub fn row_cells(row: &ComponentRow) -> Result<LibraryRow, BuildError> {
    let datasheet = if row.datasheet_label.is_empty() && row.more_datasheets.is_empty() {
        json(&row.datasheet)?
    } else {
        json(&row.documents())?
    };
    let footprint = encode_footprint_cell(row.footprint_ref.as_ref(), &row.alternate_footprints)
        .map_err(|e| BuildError::Row(e.to_string()))?;
    let hash: String = row
        .content_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let cells = [
        row.row_id.to_string(),
        row.internal_pn.as_str().to_string(),
        row.class.as_str().to_string(),
        datasheet,
        state_cell(row.state).to_string(),
        json(&row.symbol_ref)?,
        footprint,
        String::new(),
        json(&row.primary_mpn)?,
        json(&row.alternates)?,
        json(&row.supply)?,
        json(&row.parameters)?,
        json(&row.pin_map_overrides)?,
        time_cell(row.created),
        time_cell(row.updated),
        hash,
        row.version.clone(),
        row.released.to_string(),
        row.symbol_version.clone(),
        row.footprint_version.clone(),
        row.sim_version.clone(),
        row.description.clone(),
    ];
    let mut out = LibraryRow::default();
    for (col, cell) in TABLE_HEADER.iter().zip(cells) {
        out.cells.insert((*col).to_string(), cell);
    }
    Ok(out)
}

/// The library file text: manifest, then one table per class.
pub fn library_text(
    library_id: Uuid,
    name: &str,
    classes: &BTreeMap<&'static str, &'static str>,
    rows: BTreeMap<&'static str, Vec<LibraryRow>>,
) -> Result<String, BuildError> {
    let manifest = SnxlibManifest {
        format: FORMAT_TOKEN.to_string(),
        library_id,
        library: LibrarySection {
            name: name.to_string(),
            description: None,
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
                key: (*key).to_string(),
                label: (*label).to_string(),
            })
            .collect(),
    };
    let mut tables = BTreeMap::new();
    for (class, mut rows) in rows {
        rows.sort_by(|a, b| a.cells["row_id"].cmp(&b.cells["row_id"]));
        tables.insert(
            class.to_string(),
            LibraryTable {
                columns: TABLE_HEADER.iter().map(|c| (*c).to_string()).collect(),
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
