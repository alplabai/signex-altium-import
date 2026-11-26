//! The private database corpus: skipped unless `ALP_PRIVATE_DB_DIR` names a
//! folder holding a `.DbLib` and its database. Counts only; nothing derived
//! from the files is committed.

use std::path::PathBuf;
use std::process::Command;

use signex_altium_convert::dblib::{DbSource, parse_dblib};

fn corpus() -> Option<PathBuf> {
    std::env::var_os("ALP_PRIVATE_DB_DIR").map(PathBuf::from)
}

fn find(dir: &std::path::Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
}

#[test]
fn the_corpus_dblib_parses_with_one_key_field_per_table() {
    let Some(dir) = corpus() else {
        eprintln!("ALP_PRIVATE_DB_DIR is unset; skipping");
        return;
    };
    let text = std::fs::read(find(&dir, "dblib").unwrap()).unwrap();
    let text: String = text.iter().map(|&b| char::from(b)).collect();
    let db = parse_dblib(&text).unwrap();
    assert!(db.tables.len() > 100, "{}", db.tables.len());
    for t in db.tables.iter().filter(|t| t.enabled) {
        let keys = t.fields.iter().filter(|f| f.is_key()).count();
        assert_eq!(keys, 1, "one key field per enabled table");
    }
    eprintln!(
        "tables {}, enabled {}",
        db.tables.len(),
        db.tables.iter().filter(|t| t.enabled).count()
    );
}

#[test]
fn every_enabled_table_exports_with_the_row_count_mdb_count_gives() {
    let Some(dir) = corpus() else {
        return;
    };
    let text = std::fs::read(find(&dir, "dblib").unwrap()).unwrap();
    let text: String = text.iter().map(|&b| char::from(b)).collect();
    let db = parse_dblib(&text).unwrap();
    let mdb = find(&dir, "mdb").unwrap();
    let src = DbSource::access(&mdb);
    let (mut tables, mut rows, mut missing) = (0, 0, 0);
    for t in db.tables.iter().filter(|t| t.enabled) {
        // A table the database does not hold is reported by the converter
        // (`table.missing`); count it here.
        let Some(data) = src.read(&t.name).unwrap() else {
            missing += 1;
            continue;
        };
        // `mdb-count` reads a stale count from the table header, so count
        // the records of a fresh export with a minimal quote-aware scan.
        let out = Command::new("mdb-export")
            .arg("-D")
            .arg("%Y-%m-%dT%H:%M:%S")
            .arg(&mdb)
            .arg(&t.name)
            .output()
            .unwrap();
        let (mut in_quotes, mut lines, mut any) = (false, 0usize, false);
        for b in out.stdout {
            match b {
                b'"' => in_quotes = !in_quotes,
                b'\n' if !in_quotes => {
                    lines += 1;
                    any = false;
                }
                _ => any = true,
            }
        }
        lines += usize::from(any);
        let want = lines.saturating_sub(1);
        assert_eq!(
            data.rows.len(),
            want,
            "table #{tables} (name length {})",
            t.name.len()
        );
        tables += 1;
        rows += want;
    }
    eprintln!(
        "exported {tables} tables, {rows} rows, {missing} enabled tables not in the database"
    );
}
