//! The Signex library-format conformance fixtures, and this converter's own
//! output, against the vendored serializers: everything reads and re-writes
//! byte for byte, and the content hashes agree.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use signex_altium_convert::Report;
use signex_altium_convert::libset::{LibraryOptions, SourceBody, SourceLib, build_library};
use signex_library::component::{ComponentRow, DatasheetDoc, DatasheetRef, PlmReserved};
use signex_library::footprint_binding::decode_footprint_cell;
use signex_library::manufacturer::{DistributorListing, ManufacturerPart};
use signex_library::param::ParamMap;
use signex_library::primitive::symbol::SymbolFile;
use signex_library::{
    ComponentClass, FootprintFile, InternalPn, LibraryFile, LibraryTable, LifecycleState,
    PrimitiveRef, hash_row_content,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/library-format")
}

fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    v.sort();
    assert!(!v.is_empty(), "no .{ext} in {}", dir.display());
    v
}

fn rewrites_identically(path: &Path, text: &str) {
    let again = match path.extension().and_then(|e| e.to_str()).unwrap() {
        "snxlib" => LibraryFile::parse(text).unwrap().write().unwrap(),
        "snxsym" => SymbolFile::from_toml_str(text)
            .unwrap()
            .to_toml_string()
            .unwrap(),
        "snxfpt" => FootprintFile::from_toml_str(text)
            .unwrap()
            .to_toml_string()
            .unwrap(),
        e => panic!("unexpected .{e}"),
    };
    assert_eq!(
        again,
        text,
        "{} does not re-write byte-identical",
        path.display()
    );
}

#[test]
fn every_fixture_reads_and_rewrites_byte_identical() {
    let lib = fixtures().join("library");
    for ext in ["snxlib"] {
        for p in files(&lib, ext) {
            rewrites_identically(&p, &std::fs::read_to_string(&p).unwrap());
        }
    }
    for p in files(&lib.join("symbols"), "snxsym") {
        rewrites_identically(&p, &std::fs::read_to_string(&p).unwrap());
    }
    for p in files(&lib.join("footprints"), "snxfpt") {
        rewrites_identically(&p, &std::fs::read_to_string(&p).unwrap());
    }
}

fn row_from_cells(t: &LibraryTable, r: &signex_library::LibraryRow) -> ComponentRow {
    let c = |n: &str| t.cell(r, n).unwrap_or_default().to_string();
    let json = |n: &str| -> serde_json::Value {
        serde_json::from_str(&c(n)).unwrap_or_else(|e| panic!("{n}: {e}"))
    };
    let docs: Vec<DatasheetDoc> = if c("datasheet").trim_start().starts_with('[') {
        serde_json::from_str(&c("datasheet")).unwrap()
    } else {
        vec![DatasheetDoc::new(
            "",
            serde_json::from_str::<DatasheetRef>(&c("datasheet")).unwrap(),
        )]
    };
    let (default, alternates) = decode_footprint_cell(&c("footprint_ref")).unwrap();
    let sim: Option<PrimitiveRef> =
        (!c("sim_ref").is_empty()).then(|| serde_json::from_str(&c("sim_ref")).unwrap());
    let time = |n: &str| {
        chrono::DateTime::parse_from_rfc3339(&c(n))
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    let mut hash = [0u8; 32];
    for (i, b) in hash.iter_mut().enumerate() {
        *b = u8::from_str_radix(&c("content_hash")[2 * i..2 * i + 2], 16).unwrap();
    }
    let state = match c("state").as_str() {
        "Draft" => LifecycleState::Draft,
        "InReview" => LifecycleState::InReview,
        "Released" => LifecycleState::Released,
        "Deprecated" => LifecycleState::Deprecated,
        "Obsolete" => LifecycleState::Obsolete,
        s => panic!("state {s}"),
    };
    let mut row = ComponentRow {
        row_id: c("row_id").parse().unwrap(),
        internal_pn: InternalPn::new(c("internal_pn")),
        class: ComponentClass::new(c("class")),
        datasheet: Default::default(),
        datasheet_label: String::new(),
        description: c("description"),
        more_datasheets: Vec::new(),
        state,
        symbol_ref: serde_json::from_value(json("symbol_ref")).unwrap(),
        alternate_symbols: Vec::new(),
        footprint_ref: default,
        alternate_footprints: alternates,
        sim_ref: sim,
        pin_map_overrides: serde_json::from_value(json("pin_map_overrides")).unwrap(),
        primary_mpn: serde_json::from_value::<ManufacturerPart>(json("primary_mpn")).unwrap(),
        alternates: serde_json::from_value(json("alternates")).unwrap(),
        supply: serde_json::from_value::<Vec<DistributorListing>>(json("supply")).unwrap(),
        parameters: serde_json::from_value::<ParamMap>(json("parameters")).unwrap(),
        plm: PlmReserved::default(),
        version: c("version"),
        released: c("released") == "true",
        symbol_version: c("symbol_version"),
        footprint_version: c("footprint_version"),
        sim_version: c("sim_version"),
        created: time("created"),
        updated: time("updated"),
        content_hash: hash,
        server_revision: None,
    };
    row.set_documents(docs);
    if !c("alternate_symbols").is_empty() {
        row.alternate_symbols = serde_json::from_str(&c("alternate_symbols")).unwrap();
    }
    row
}

#[test]
fn the_content_hash_fixtures_match_the_writers_hash() {
    let text = std::fs::read_to_string(fixtures().join("library/Conformance.snxlib")).unwrap();
    let lib = LibraryFile::parse(&text).unwrap();
    let mut checked = 0;
    for t in lib.tables.values() {
        for r in &t.rows {
            let row = row_from_cells(t, r);
            let want = std::fs::read(fixtures().join(format!("content-hash/{}.json", row.row_id)))
                .unwrap();
            let digest = Sha256::digest(&want);
            assert_eq!(
                row.content_hash.as_slice(),
                digest.as_slice(),
                "{}",
                row.internal_pn
            );
            assert_eq!(
                hash_row_content(&row).unwrap(),
                row.content_hash,
                "{}",
                row.internal_pn
            );
            // Encoding the decoded row gives the fixture's own cells back
            // (covers the array forms and the optional column).
            let again = signex_altium_convert::libset::row_cells(&row).unwrap();
            for (col, cell) in &again.cells {
                assert_eq!(
                    t.cell(r, col).unwrap_or(""),
                    cell,
                    "{}: {col}",
                    row.internal_pn
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 3);
}

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

#[test]
fn this_converters_own_output_reads_and_rewrites_byte_identical() {
    let sources = vec![
        SourceLib {
            released: false,
            path: "/in/simple.SchLib".into(),
            name: "simple".into(),
            body: SourceBody::Sch(
                altium_format::sch::SchLib::open(sample("simple.SchLib")).unwrap(),
            ),
        },
        SourceLib {
            released: false,
            path: "/in/Simple.PcbLib".into(),
            name: "Simple".into(),
            body: SourceBody::Pcb(
                altium_format::pcb::PcbLib::open(sample("Simple.PcbLib")).unwrap(),
            ),
        },
    ];
    let mut report = Report::default();
    let built = build_library(
        &sources,
        &LibraryOptions::fresh("Own"),
        &mut report,
        &mut |_, _, _| {},
    )
    .unwrap();
    assert!(built.files.len() > 2);
    for (rel, text) in &built.files {
        rewrites_identically(Path::new(rel), text);
    }
    // And its stored hashes are the writer's own.
    let (_, snxlib) = built
        .files
        .iter()
        .find(|(p, _)| p.ends_with(".snxlib"))
        .unwrap();
    let lib = LibraryFile::parse(snxlib).unwrap();
    for t in lib.tables.values() {
        for r in &t.rows {
            let row = row_from_cells(t, r);
            assert_eq!(
                hash_row_content(&row).unwrap(),
                row.content_hash,
                "{}",
                row.internal_pn
            );
        }
    }
}
