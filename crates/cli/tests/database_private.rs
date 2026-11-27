//! The private database corpus, end to end (pool only): skipped unless
//! `ALP_PRIVATE_DB_DIR` names the checkout. Counts only are printed.

mod common;
use common::{Tmp, run, schema};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;

#[test]
fn the_corpus_database_converts_and_the_report_accounts_for_every_row() {
    let Some(dir) = std::env::var_os("ALP_PRIVATE_DB_DIR") else {
        eprintln!("ALP_PRIVATE_DB_DIR is unset; skipping");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let dblib = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dblib"))
        })
        .unwrap();
    let out = Tmp::new();
    let o = run(&[
        OsStr::new("library"),
        dblib.as_os_str(),
        OsStr::new("--output"),
        out.0.join("lib").as_os_str(),
        OsStr::new("--json"),
    ]);
    let last = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .last()
        .unwrap()
        .to_string();
    let s: Value = serde_json::from_str(&last).unwrap();
    schema().validate(&s).unwrap();
    let code = o.status.code().unwrap();
    assert!([0, 2].contains(&code), "{s}");
    let r: Value =
        serde_json::from_slice(&std::fs::read(out.0.join("lib/import-report.json")).unwrap())
            .unwrap();
    schema().validate(&r).unwrap();
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for i in r["items"].as_array().unwrap() {
        *by.entry(format!(
            "{} {}",
            i["code"].as_str().unwrap(),
            i["severity"].as_str().unwrap()
        ))
        .or_default() += 1;
    }
    eprintln!(
        "exit {code}; source {}; written {}",
        r["counts"]["source"], r["counts"]["written"]
    );
    for (k, v) in &by {
        if k.starts_with("row.") || k.starts_with("table.") || k.starts_with("part.pn") {
            eprintln!("  {k}: {v}");
        }
    }
    let rows_failed: usize = by
        .iter()
        .filter(|(k, _)| k.starts_with("row."))
        .map(|(_, v)| v)
        .sum();
    let src = r["counts"]["source"]["parts"].as_u64().unwrap() as usize;
    let written = r["counts"]["written"]["parts"].as_u64().unwrap() as usize;
    assert_eq!(
        src,
        written + rows_failed,
        "every row is written or reported"
    );
}
