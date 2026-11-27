//! `--update` keeps what a user added to the library: the new `.snxlib` is
//! merged into the one on disk, dropping only the rows this converter wrote
//! before (its record, untrusted) and adding the freshly converted ones.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use signex_library::footprint_binding::decode_footprint_cell;
use signex_library::{LibraryFile, LibraryRow, LibraryTable};
use uuid::Uuid;

fn cell<'a>(t: &'a LibraryTable, r: &'a LibraryRow, c: &str) -> &'a str {
    t.cell(r, c).unwrap_or_default()
}

fn row_id(t: &LibraryTable, r: &LibraryRow) -> Option<Uuid> {
    Uuid::parse_str(cell(t, r, "row_id")).ok()
}

/// What a merge produced.
pub struct Merged {
    pub text: String,
    /// Primitives of this library that rows a user added still use, by id,
    /// with the part number of one such row.
    pub kept_refs: BTreeMap<Uuid, String>,
}

fn primitive_uuids(library: Uuid, t: &LibraryTable, r: &LibraryRow) -> Vec<Uuid> {
    let mut out = Vec::new();
    let mut take = |v: &serde_json::Value| {
        if v["library_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            == Some(library)
            && let Some(u) = v["uuid"].as_str().and_then(|s| Uuid::parse_str(s).ok())
        {
            out.push(u);
        }
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(cell(t, r, "symbol_ref")) {
        take(&v);
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(cell(t, r, "alternate_symbols"))
        && let Some(a) = v.as_array()
    {
        a.iter().for_each(|x| take(&x["symbol_ref"]));
    }
    if let Ok((d, alts)) = decode_footprint_cell(cell(t, r, "footprint_ref")) {
        for p in d.into_iter().chain(alts.into_iter().map(|a| a.footprint)) {
            if p.library_id == library {
                out.push(p.uuid);
            }
        }
    }
    out
}

/// The merged library. `written_before` is the rows the earlier run wrote,
/// or empty when there is no usable record (nothing is removed). `name` is
/// the library's display name when the user asked for one; else the existing
/// name stays. A part number a kept row already holds is refused.
pub fn merge(
    old: &str,
    new: &str,
    written_before: &BTreeSet<Uuid>,
    name: Option<&str>,
) -> Result<Merged, String> {
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
    // Part numbers are library-wide: a kept row's number may not be reused.
    let mut kept_pns: BTreeMap<String, (String, Uuid)> = BTreeMap::new();
    let library = merged.manifest.library_id;
    let mut kept_refs = BTreeMap::new();
    for (tname, t) in &merged.tables {
        for r in &t.rows {
            let pn = cell(t, r, "internal_pn").trim().to_string();
            if !pn.is_empty() {
                kept_pns.insert(
                    pn.clone(),
                    (tname.clone(), row_id(t, r).unwrap_or_default()),
                );
            }
            for u in primitive_uuids(library, t, r) {
                kept_refs.entry(u).or_insert_with(|| pn.clone());
            }
        }
    }
    for t in fresh.tables.values() {
        for r in &t.rows {
            let pn = cell(t, r, "internal_pn").trim();
            if let Some((_, other)) = kept_pns.get(pn) {
                return Err(format!(
                    "part number {pn:?} of the imported part {} is already held by the row {other} that was not imported; rename one",
                    cell(t, r, "row_id")
                ));
            }
        }
    }
    for (tname, nt) in &fresh.tables {
        let mt = merged
            .tables
            .entry(tname.clone())
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
    if let Some(n) = name {
        merged.manifest.library.name = n.to_string();
    }
    let text = merged.write().map_err(|e| format!("merged library: {e}"))?;
    Ok(Merged { text, kept_refs })
}
