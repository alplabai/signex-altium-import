//! `describe` and `library` against the Signex library converter schema.

mod common;
use common::{Tmp, run, sample, schema};
use serde_json::Value;
use std::ffi::OsStr;

fn lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8(stdout.to_vec())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {l}: {e}")))
        .collect()
}

fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

#[test]
fn describe_validates_against_the_schema() {
    let o = run(&[os("describe"), os("--json")]);
    assert_eq!(o.status.code(), Some(0));
    let v = lines(&o.stdout);
    assert_eq!(v.len(), 1);
    let d = &v[0];
    schema().validate(d).expect("describe matches schema");
    assert_eq!(d["contract"], "1.0");
    assert_eq!(d["converter"], "signex-altium-import");
    let kind = |ext: &str| {
        d["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["extensions"][0] == ext)
            .unwrap_or_else(|| panic!("no {ext}"))["kind"]
            .clone()
    };
    for ext in ["schlib", "pcblib", "intlib"] {
        assert_eq!(kind(ext), "library");
    }
    assert_eq!(kind("dblib"), "database");
}

#[test]
fn dry_run_streams_valid_events_and_writes_only_the_report() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    let o = run(&[
        os("library"),
        sample("simple.SchLib").as_os_str(),
        sample("Simple.PcbLib").as_os_str(),
        os("--output"),
        dir.as_os_str(),
        os("--dry-run"),
        os("--progress"),
    ]);
    let ev = lines(&o.stdout);
    let v = schema();
    for e in &ev {
        v.validate(e).unwrap_or_else(|er| panic!("{e}: {er}"));
    }
    assert_eq!(ev[0]["event"], "phase");
    assert_eq!(ev[0]["phase"], "read");
    let phases: Vec<_> = ev
        .iter()
        .filter(|e| e["event"] == "phase")
        .map(|e| e["phase"].as_str().unwrap())
        .collect();
    assert_eq!(phases, ["read", "convert"]);
    let s = ev.last().unwrap();
    assert_eq!(s["event"], "summary");
    assert_eq!(s["ok"], true);
    assert!(["ok", "partial"].contains(&s["exit"].as_str().unwrap()));
    let want = if s["exit"] == "ok" { 0 } else { 2 };
    assert_eq!(o.status.code(), Some(want));
    assert_eq!(s["files"], serde_json::json!([]));

    // Only the report is written, and the summary points inside --output.
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["import-report.json", "import-report.md"]);
    let canon = dir.canonicalize().unwrap();
    assert!(std::path::Path::new(s["report"].as_str().unwrap()).starts_with(&canon));

    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("import-report.json")).unwrap()).unwrap();
    v.validate(&report).expect("report matches schema");
    assert_eq!(report["sources"].as_array().unwrap().len(), 2);
    assert!(report["counts"]["source"]["symbols"].as_u64().unwrap() > 0);
    assert!(report["counts"]["source"]["footprints"].as_u64().unwrap() > 0);
    // Every item carries a catalogued, well-formed code.
    for i in report["items"].as_array().unwrap() {
        assert!(i["code"].as_str().unwrap().contains('.'));
        assert!(i["source"].as_str().is_some());
    }
}

#[test]
fn real_run_fails_cleanly_until_output_lands() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    let o = run(&[
        os("library"),
        sample("simple.SchLib").as_os_str(),
        os("--output"),
        dir.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let ev = lines(&o.stdout);
    assert_eq!(ev.len(), 1, "no progress without --progress");
    let s = &ev[0];
    schema().validate(s).expect("failed summary matches schema");
    assert_eq!(s["ok"], false);
    assert_eq!(s["exit"], "failed");
    assert_eq!(s["error"], "library output not implemented yet");
    assert!(dir.join("import-report.json").is_file());
    assert!(!dir.join("simple.snxsym").exists());
}

#[test]
fn bad_input_fails_with_a_summary() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    for input in ["x.txt", "x.IntLib"] {
        let o = run(&[
            os("library"),
            os(input),
            os("--output"),
            dir.as_os_str(),
            os("--json"),
        ]);
        assert_eq!(o.status.code(), Some(1), "{input}");
        let s = lines(&o.stdout).pop().unwrap();
        schema().validate(&s).unwrap();
        assert_eq!(s["ok"], false, "{input}");
    }
    assert!(!dir.exists(), "nothing is written for rejected inputs");
}

#[test]
fn unknown_option_exits_1_with_a_failed_summary() {
    let o = run(&[
        os("library"),
        sample("simple.SchLib").as_os_str(),
        os("--output"),
        os("/nonexistent/out"),
        os("--frobnicate"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let s = lines(&o.stdout).pop().unwrap();
    schema().validate(&s).unwrap();
    assert_eq!(s["ok"], false);
    assert_eq!(s["exit"], "failed");
    assert!(s["error"].as_str().unwrap().contains("--frobnicate"));
}

#[test]
fn a_non_empty_output_is_refused_without_update() {
    let out = Tmp::new();
    std::fs::write(out.0.join("keep.txt"), "x").unwrap();
    let o = run(&[
        os("library"),
        sample("simple.SchLib").as_os_str(),
        os("--output"),
        out.0.as_os_str(),
        os("--dry-run"),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    assert!(out.0.join("keep.txt").is_file());
    assert!(!out.0.join("import-report.json").exists());
}

#[test]
fn bad_library_id_is_refused() {
    let out = Tmp::new();
    let o = run(&[
        os("library"),
        sample("simple.SchLib").as_os_str(),
        os("--output"),
        out.0.join("o").as_os_str(),
        os("--library-id"),
        os("nope"),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let s = lines(&o.stdout).pop().unwrap();
    assert!(s["error"].as_str().unwrap().contains("UUID"));
}

#[test]
fn an_unreadable_input_fails_and_is_reported() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    let o = run(&[
        os("library"),
        os("nonexistent.SchLib"),
        os("--output"),
        dir.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let s = lines(&o.stdout).pop().unwrap();
    schema().validate(&s).unwrap();
    assert_eq!(s["ok"], false);
    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("import-report.json")).unwrap()).unwrap();
    schema().validate(&report).unwrap();
    assert_eq!(report["items"][0]["code"], "library.input_unreadable");
    assert_eq!(report["items"][0]["severity"], "error");
}
