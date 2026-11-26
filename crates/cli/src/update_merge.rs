//! `--update` keeps what a user added to the library: the new `.snxlib` is
//! merged into the one on disk, dropping only the rows this converter wrote
//! before (its record, untrusted) and adding the freshly converted ones.

use std::collections::BTreeSet;
use std::sync::Arc;

use signex_library::{LibraryFile, LibraryTable};
use uuid::Uuid;

fn row_id(t: &LibraryTable, r: &signex_library::LibraryRow) -> Option<Uuid> {
    t.cell(r, "row_id").and_then(|s| Uuid::parse_str(s).ok())
}

/// The text of the merged library. `written_before` is the rows the earlier
/// run wrote, or empty when there is no usable record (nothing is removed).
pub fn merge(old: &str, new: &str, written_before: &BTreeSet<Uuid>) -> Result<String, String> {
    let mut merged = LibraryFile::parse(old).map_err(|e| format!("existing library: {e}"))?;
    let fresh = LibraryFile::parse(new).map_err(|e| format!("converted library: {e}"))?;
    let new_ids: BTreeSet<Uuid> = fresh
        .tables
        .values()
        .flat_map(|t| t.rows.iter().filter_map(|r| row_id(t, r)))
        .collect();
    for t in merged.tables.values_mut() {
        let keep: Vec<_> = t
            .rows
            .iter()
            .filter(|r| match row_id(t, r) {
                Some(id) => !written_before.contains(&id) && !new_ids.contains(&id),
                None => true,
            })
            .cloned()
            .collect();
        t.rows = keep;
    }
    for (name, nt) in &fresh.tables {
        let mt = merged
            .tables
            .entry(name.clone())
            .or_insert_with(|| LibraryTable {
                columns: nt.columns.clone(),
                ..Default::default()
            });
        for c in &nt.columns {
            if !mt.columns.contains(c) {
                mt.columns.push(c.clone());
            }
        }
        mt.rows.extend(nt.rows.iter().map(Arc::clone));
        mt.rows
            .sort_by(|a, b| a.cells.get("row_id").cmp(&b.cells.get("row_id")));
    }
    for c in &fresh.manifest.classes {
        if !merged.manifest.classes.iter().any(|m| m.key == c.key) {
            merged.manifest.classes.push(c.clone());
        }
    }
    merged.manifest.library = fresh.manifest.library.clone();
    merged.write().map_err(|e| format!("merged library: {e}"))
}
