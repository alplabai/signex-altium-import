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

fn library_run(dir: &std::path::Path, extra: &[&str]) -> (std::process::Output, Value) {
    let (sch, pcb) = (sample("simple.SchLib"), sample("Simple.PcbLib"));
    let mut args = vec![
        os("library"),
        sch.as_os_str(),
        pcb.as_os_str(),
        os("--output"),
        dir.as_os_str(),
        os("--json"),
    ];
    args.extend(extra.iter().map(|a| os(a)));
    let o = run(&args);
    let s = lines(&o.stdout).pop().expect("a summary");
    schema().validate(&s).expect("summary matches schema");
    (o, s)
}

fn tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(
        root: &std::path::Path,
        d: &std::path::Path,
        out: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = Default::default();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn a_real_run_writes_a_library_report_and_summary() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    let (o, s) = library_run(&dir, &["--name", "Samples"]);
    assert_eq!(s["ok"], true);
    assert!([0, 2].contains(&o.status.code().unwrap()));
    assert_eq!(
        s["exit"],
        if o.status.code() == Some(0) {
            "ok"
        } else {
            "partial"
        }
    );
    let canon = dir.canonicalize().unwrap();
    let files: Vec<String> = s["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert!(files.iter().any(|f| f.ends_with("samples.snxlib")));
    assert!(
        files
            .iter()
            .any(|f| f.contains("/symbols/") && f.ends_with(".snxsym"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.contains("/footprints/") && f.ends_with(".snxfpt"))
    );
    for f in &files {
        assert!(
            std::path::Path::new(f).starts_with(&canon) && std::path::Path::new(f).is_file(),
            "{f}"
        );
    }
    assert!(s["counts"]["parts"].as_u64().unwrap() > 0);
    assert_eq!(
        s["counts"]["parts"].as_u64().unwrap() + s["counts"]["skipped"].as_u64().unwrap(),
        serde_json::from_slice::<Value>(&std::fs::read(dir.join("import-report.json")).unwrap())
            .unwrap()["counts"]["source"]["parts"]
            .as_u64()
            .unwrap()
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(dir.join("import-report.json")).unwrap()).unwrap();
    schema().validate(&report).unwrap();
    assert!(report["library_id"].as_str().is_some());
    assert!(dir.join("import-report.md").is_file());
}

#[test]
fn a_second_run_into_a_fresh_directory_is_byte_identical() {
    let out = Tmp::new();
    let (a, b) = (out.0.join("a"), out.0.join("b"));
    library_run(&a, &[]);
    library_run(&b, &[]);
    let (mut ta, mut tb) = (tree(&a), tree(&b));
    // The report names its sources and start time; the library is the contract.
    for t in [&mut ta, &mut tb] {
        t.remove("import-report.json");
        t.remove("import-report.md");
    }
    assert_eq!(ta, tb);
}

#[test]
fn update_over_the_same_source_changes_nothing() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    library_run(&dir, &[]);
    let before = tree(&dir);
    let (_, s) = library_run(&dir, &["--update"]);
    assert_eq!(s["ok"], true);
    let mut after = tree(&dir);
    let mut before = before;
    for t in [&mut before, &mut after] {
        t.remove("import-report.json");
        t.remove("import-report.md");
    }
    assert_eq!(before, after);
}

#[test]
fn update_refuses_a_different_library_and_leaves_it_alone() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    library_run(
        &dir,
        &["--library-id", "5b0f3b0e-0000-4000-8000-000000000001"],
    );
    let before = tree(&dir);
    let (o, s) = library_run(
        &dir,
        &[
            "--update",
            "--library-id",
            "5b0f3b0e-0000-4000-8000-000000000002",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(s["ok"], false);
    assert!(s["error"].as_str().unwrap().contains("id"));
    let mut after = tree(&dir);
    let mut before = before;
    for t in [&mut before, &mut after] {
        t.remove("import-report.json");
        t.remove("import-report.md");
    }
    assert_eq!(before, after, "the library on disk is untouched");
}

#[test]
fn update_removes_what_the_source_no_longer_has() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    library_run(&dir, &[]);
    let stale = dir.join("footprints/00000000-0000-0000-0000-000000000000.snxfpt");
    let real = std::fs::read_dir(dir.join("footprints"))
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    std::fs::copy(real, &stale).unwrap();
    library_run(&dir, &["--update"]);
    assert!(!stale.exists());
}

#[test]
fn update_needs_a_library_in_the_directory() {
    let out = Tmp::new();
    let (o, s) = library_run(&out.0, &["--update"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(s["error"].as_str().unwrap().contains(".snxlib"));
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

fn broken_input(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn damaged_inputs_fail_with_a_summary_not_a_crash() {
    let out = Tmp::new();
    let whole = std::fs::read(sample("simple.SchLib")).unwrap();
    let cases = [
        ("truncated.SchLib", whole[..whole.len() / 3].to_vec()),
        (
            "garbage.SchLib",
            (0..4096u32).map(|i| (i * 31 % 251) as u8).collect(),
        ),
        ("empty.PcbLib", Vec::new()),
    ];
    for (name, bytes) in cases {
        let input = broken_input(&out.0, name, &bytes);
        let dir = out.0.join(format!("out-{name}"));
        let o = run(&[
            os("library"),
            input.as_os_str(),
            os("--output"),
            dir.as_os_str(),
            os("--json"),
        ]);
        assert_eq!(o.status.code(), Some(1), "{name}");
        let s = lines(&o.stdout).pop().unwrap();
        schema().validate(&s).unwrap();
        assert_eq!(s["ok"], false, "{name}");
        assert_eq!(s["exit"], "failed", "{name}");
        assert!(dir.join("import-report.json").is_file(), "{name}");
    }
}

#[test]
fn update_needs_an_existing_directory() {
    let out = Tmp::new();
    let file = out.0.join("a-file");
    std::fs::write(&file, "x").unwrap();
    for target in [out.0.join("missing"), file] {
        let o = run(&[
            os("library"),
            sample("simple.SchLib").as_os_str(),
            os("--output"),
            target.as_os_str(),
            os("--update"),
            os("--json"),
        ]);
        assert_eq!(o.status.code(), Some(1));
        let s = lines(&o.stdout).pop().unwrap();
        assert!(s["error"].as_str().unwrap().contains("--update"));
    }
}

#[test]
fn a_path_named_like_a_command_is_an_input() {
    let out = Tmp::new();
    std::fs::write(out.0.join("describe"), "x").unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_signex-altium-import"))
        .current_dir(&out.0)
        .args(["describe", "--json"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let s = lines(&o.stdout).pop().unwrap();
    // The single-file summary, not a describe object.
    assert!(s.get("project").is_some() && s.get("contract").is_none());
}

#[test]
fn update_fails_on_a_part_row_it_cannot_read_back() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    library_run(&dir, &[]);
    let lib = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "snxlib"))
        .unwrap();
    let text = std::fs::read_to_string(&lib).unwrap();
    assert!(text.contains("\tDraft\t"));
    std::fs::write(&lib, text.replacen("\tDraft\t", "\tBogus\t", 1)).unwrap();
    let before = tree(&dir);
    let (o, s) = library_run(&dir, &["--update"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(s["error"].as_str().unwrap().contains("unreadable state"));
    let (mut a, mut b) = (before, tree(&dir));
    for t in [&mut a, &mut b] {
        t.remove("import-report.json");
        t.remove("import-report.md");
    }
    assert_eq!(a, b);
}

#[test]
fn two_inputs_with_one_file_name_are_refused() {
    let out = Tmp::new();
    let one = sample("simple.SchLib");
    let o = run(&[
        os("library"),
        one.as_os_str(),
        one.as_os_str(),
        os("--output"),
        out.0.join("o").as_os_str(),
        os("--json"),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let s = lines(&o.stdout).pop().unwrap();
    assert!(s["error"].as_str().unwrap().contains("same kind"));
}

#[cfg(unix)]
#[test]
fn update_never_deletes_or_reads_through_symlinks() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    library_run(&dir, &[]);
    let victim = out.0.join("victim.txt");
    std::fs::write(&victim, "keep").unwrap();
    let outside_dir = out.0.join("outside");
    std::fs::create_dir(&outside_dir).unwrap();
    let outside_fp = outside_dir.join("00000000-0000-0000-0000-0000000000aa.snxfpt");
    std::fs::write(&outside_fp, "keep").unwrap();
    // A symlinked envelope, a symlinked folder entry, and odd names that are
    // not of the expected forms.
    let fps = dir.join("footprints");
    std::os::unix::fs::symlink(
        &victim,
        fps.join("00000000-0000-0000-0000-0000000000bb.snxfpt"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&victim, dir.join("symbols/evil.snxsym")).unwrap();
    std::fs::write(fps.join("not-a-uuid.snxfpt"), "keep").unwrap();
    std::fs::write(dir.join("symbols/notes.txt"), "keep").unwrap();
    let (_, s) = library_run(&dir, &["--update"]);
    assert_eq!(s["ok"], true, "{s}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    assert!(outside_fp.exists());
    // Not of the expected form: not ours to delete.
    assert!(fps.join("not-a-uuid.snxfpt").exists());
    assert!(dir.join("symbols/notes.txt").exists());
    // The symlinks themselves are left alone too.
    assert!(
        std::fs::symlink_metadata(dir.join("symbols/evil.snxsym"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn update_with_a_symlinked_folder_does_not_walk_it() {
    let out = Tmp::new();
    let dir = out.0.join("lib");
    library_run(&dir, &[]);
    let elsewhere = out.0.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let stale = elsewhere.join("00000000-0000-0000-0000-0000000000cc.snxfpt");
    std::fs::write(&stale, "keep").unwrap();
    std::fs::remove_dir_all(dir.join("footprints")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.join("footprints")).unwrap();
    let (o, _) = library_run(&dir, &["--update"]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "writing through a symlinked folder is refused"
    );
    assert!(stale.exists());
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 1);
}
