//! The single-file CLI is unchanged: its `--json` output is pinned against
//! goldens captured before the library contract work.

mod common;
use common::{Tmp, placeholders, repo, run, sample};
use std::ffi::OsStr;

fn legacy(file: &str, golden: &str, exit: i32) {
    let out = Tmp::new();
    let o = run(&[
        sample(file).as_os_str(),
        OsStr::new("-o"),
        out.0.as_os_str(),
        OsStr::new("--json"),
    ]);
    let got = placeholders(
        &String::from_utf8(o.stdout).unwrap(),
        &[
            (&out.0.canonicalize().unwrap(), "<OUT>"),
            (&repo().join("tests").join("samples"), "<SAMPLES>"),
        ],
    );
    let want = std::fs::read_to_string(repo().join("tests/fixtures/legacy").join(golden)).unwrap();
    assert_eq!(got, want, "{file}");
    assert_eq!(o.status.code(), Some(exit), "{file}");
}

#[test]
fn schdoc() {
    legacy("simple.SchDoc", "simple.SchDoc.json", 0);
}

#[test]
fn pcbdoc() {
    legacy("simple.PcbDoc", "simple.PcbDoc.json", 0);
}

#[test]
fn schlibs() {
    legacy("simple.SchLib", "simple.SchLib.json", 0);
    legacy("graphic-mixed.SchLib", "graphic-mixed.SchLib.json", 0);
    legacy("long-pindata.SchLib", "long-pindata.SchLib.json", 0);
}

#[test]
fn pcblib() {
    legacy("Simple.PcbLib", "Simple.PcbLib.json", 0);
}

#[test]
fn project() {
    // The sample project lists a sheet that is not in the samples, so the
    // pinned behaviour is its error summary.
    legacy("simple-proj.prjpcb", "simple-proj.prjpcb.json", 1);
}

#[test]
fn missing_input_fails_with_exit_1() {
    let o = run(&[sample("nonexist.SchDoc").as_os_str(), OsStr::new("--json")]);
    let samples = repo().join("tests").join("samples");
    let got = placeholders(
        &String::from_utf8(o.stdout).unwrap(),
        &[(&samples, "<SAMPLES>")],
    );
    let want = std::fs::read_to_string(repo().join("tests/fixtures/legacy/missing.json")).unwrap();
    assert_eq!(got, want);
    assert_eq!(o.status.code(), Some(1));
}
