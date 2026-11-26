//! Database libraries end to end: the synthetic fixture
//! (`tests/fixtures/db/`, see its NOTICE) covers a missing symbol, a missing
//! alternate footprint, a duplicate key, a missing key, a disabled table and a
//! row without a footprint.

mod common;
use common::{Tmp, repo, run, sample, schema};
use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

/// The fixture and the libraries it names, in a temporary folder.
fn setup(tmp: &Tmp) -> PathBuf {
    let root = tmp.0.join("db");
    copy_dir(&repo().join("tests/fixtures/db"), &root);
    let libs = root.join("Libs");
    std::fs::create_dir_all(&libs).unwrap();
    for (from, to) in [
        ("simple.SchLib", "Passives.SchLib"),
        ("graphic-mixed.SchLib", "Misc.SchLib"),
        ("Simple.PcbLib", "Footprints1.PcbLib"),
        ("Simple.PcbLib", "Footprints2.PcbLib"),
    ] {
        std::fs::copy(sample(from), libs.join(to)).unwrap();
    }
    root
}

fn lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8(stdout.to_vec())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn convert(root: &Path, out: &Path, extra: &[&str]) -> (std::process::Output, Value) {
    let dblib = root.join("Synthetic.DbLib");
    let csv = root.join("csv");
    let mut args: Vec<&OsStr> = vec![
        os("library"),
        dblib.as_os_str(),
        os("--db"),
        csv.as_os_str(),
        os("--output"),
        out.as_os_str(),
        os("--json"),
    ];
    args.extend(extra.iter().map(|a| os(a)));
    let o = run(&args);
    let s = lines(&o.stdout).pop().expect("a summary");
    (o, s)
}

fn report(out: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(out.join("import-report.json")).unwrap()).unwrap()
}

fn codes(r: &Value, prefix: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = r["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["code"].as_str().unwrap().starts_with(prefix))
        .map(|i| {
            (
                i["code"].as_str().unwrap().to_string(),
                i["source"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn the_fixture_produces_exactly_the_expected_report_entries() {
    let tmp = Tmp::new();
    let root = setup(&tmp);
    let out = tmp.0.join("out");
    let (o, s) = convert(
        &root,
        &out,
        &["--map", root.join("map.toml").to_str().unwrap()],
    );
    schema().validate(&s).unwrap();
    assert_eq!(o.status.code(), Some(2), "{s}");
    assert_eq!(s["exit"], "partial");
    let r = report(&out);
    schema().validate(&r).unwrap();

    let skipped = codes(&r, "row.");
    assert_eq!(
        skipped,
        vec![
            (
                "row.footprint_missing".to_string(),
                "Resistors:RES-NOALT".to_string()
            ),
            ("row.key_missing".to_string(), "Resistors:row 1".to_string()),
            (
                "row.symbol_missing".to_string(),
                "Resistors:RES-NOSYM".to_string()
            ),
        ]
    );
    let tables = codes(&r, "table.");
    assert_eq!(
        tables,
        vec![
            ("table.disabled".to_string(), "Retired".to_string()),
            ("table.missing".to_string(), "Absent".to_string()),
            (
                "table.reserved_unknown".to_string(),
                "Resistors".to_string()
            ),
            ("table.sim_skipped".to_string(), "Resistors".to_string()),
        ]
    );
    let dup: Vec<_> = codes(&r, "part.pn_collision");
    assert_eq!(dup.len(), 1);

    // 8 rows, 3 skipped, 5 written; the report accounts for each.
    assert_eq!(r["counts"]["source"]["parts"], 8);
    assert_eq!(r["counts"]["written"]["parts"], 5);
    assert!(s["counts"]["skipped"].as_u64().unwrap() >= 3);

    // One library table per enabled database table, rows released.
    let lib = std::fs::read_to_string(
        std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "snxlib"))
            .unwrap(),
    )
    .unwrap();
    assert!(lib.contains("[tables.Resistors]") && lib.contains("[tables.Capacitors]"));
    assert!(!lib.contains("Retired"));
    assert_eq!(lib.matches("\tReleased\t").count(), 5);
    for pn in ["RES-OK", "RES-DUP", "RES-DUP~2", "RES-NOFP", "CAP-1"] {
        assert!(lib.contains(&format!("\t{pn}\t")), "{pn}");
    }
    assert!(!lib.contains("RES-NOSYM") && !lib.contains("RES-NOALT"));

    // Alternate footprints 2..n are bindings of the first part.
    let line = lib.lines().find(|l| l.contains("\tRES-OK\t")).unwrap();
    let cells: Vec<&str> = line.split('\t').collect();
    assert!(
        cells[6].starts_with('['),
        "default plus alternates: {}",
        cells[6]
    );
    assert_eq!(cells[6].matches("\"footprint\"").count(), 3);
    // Mapped columns became fields: manufacturer (via --map), supply, datasheet.
    assert!(cells[8].contains("\"manufacturer\":\"Acme\"") && cells[8].contains("A-100"));
    assert!(cells[10].contains("SHOP-1"));
    assert!(cells[3].contains("https://example.com/ds.pdf"));
    assert!(
        cells[11].contains("\"Colour\":{\"kind\":\"Text\",\"value\":\"blue\"}"),
        "{}",
        cells[11]
    );
    // The mapping the run used sits beside the report.
    assert!(out.join("import-mapping.toml").is_file());
}

#[test]
fn the_written_mapping_reproduces_the_run() {
    let tmp = Tmp::new();
    let root = setup(&tmp);
    let (a, b) = (tmp.0.join("a"), tmp.0.join("b"));
    convert(
        &root,
        &a,
        &["--map", root.join("map.toml").to_str().unwrap()],
    );
    let used = a.join("import-mapping.toml");
    let text = std::fs::read_to_string(&used).unwrap();
    assert!(
        text.contains("\"Maker\" = \"Manufacturer\"") && text.contains("[Key]"),
        "{text}"
    );
    convert(&root, &b, &["--map", used.to_str().unwrap()]);
    assert_eq!(
        text,
        std::fs::read_to_string(b.join("import-mapping.toml")).unwrap()
    );
    let lib = |d: &Path| {
        std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "snxlib"))
            .map(|p| std::fs::read(p).unwrap())
            .unwrap()
    };
    assert_eq!(lib(&a), lib(&b));
}

#[test]
fn each_referenced_library_is_read_once_and_only_named_primitives_are_written() {
    let tmp = Tmp::new();
    let root = setup(&tmp);
    let out = tmp.0.join("out");
    convert(&root, &out, &[]);
    let r = report(&out);
    // The DbLib, the CSV-less run has no database file, and 4 libraries at most.
    let sources = r["sources"].as_array().unwrap().len();
    assert!(sources <= 5, "{sources}");
    // Footprints: the three the rows name (Footprints1 x2 and Footprints2 x1),
    // not every footprint of every library.
    let n = std::fs::read_dir(out.join("footprints")).unwrap().count();
    assert_eq!(n, 3);
    // Symbols: the two components the rows use, in two envelopes.
    assert_eq!(std::fs::read_dir(out.join("symbols")).unwrap().count(), 2);
}

#[test]
fn a_stored_database_that_is_not_here_asks_for_db() {
    let tmp = Tmp::new();
    let root = setup(&tmp);
    let out = tmp.0.join("out");
    let dblib = root.join("Synthetic.DbLib");
    let o = run(&[
        os("library"),
        dblib.as_os_str(),
        os("--output"),
        out.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let s = lines(&o.stdout).pop().unwrap();
    assert!(s["ok"] == false);
    let rep = report(&out);
    assert!(rep["items"].to_string().contains("pass --db"));
}

#[test]
fn a_database_library_is_converted_on_its_own() {
    let tmp = Tmp::new();
    let root = setup(&tmp);
    let dblib = root.join("Synthetic.DbLib");
    let other = sample("simple.SchLib");
    let o = run(&[
        os("library"),
        dblib.as_os_str(),
        other.as_os_str(),
        os("--output"),
        tmp.0.join("o").as_os_str(),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn duplicate_keys_get_their_suffix_whatever_the_row_order() {
    let tmp = Tmp::new();
    let root = setup(&tmp);
    let (a, b) = (tmp.0.join("a"), tmp.0.join("b"));
    convert(&root, &a, &[]);
    // The same rows in the opposite physical order (a compacted database
    // reorders them): the same library, byte for byte.
    let csv = root.join("csv/Resistors.csv");
    let text = std::fs::read_to_string(&csv).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    let head = lines.remove(0);
    lines.reverse();
    std::fs::write(&csv, format!("{head}\n{}\n", lines.join("\n"))).unwrap();
    convert(&root, &b, &[]);
    let lib = |d: &Path| {
        std::fs::read_dir(d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "snxlib"))
            .map(|p| std::fs::read(p).unwrap())
            .unwrap()
    };
    assert_eq!(lib(&a), lib(&b));
}
