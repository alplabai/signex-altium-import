//! Whole-library conversion: SchLib and PcbLib sources to one library.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use altium_format::pcb::PcbLib;
use altium_format::sch::{Component, Implementation, SchLib};
use signex_altium_convert::Report;
use signex_altium_convert::libset::{Built, LibraryOptions, SourceBody, SourceLib, build_library};
use signex_altium_convert::report::{ItemKind, Severity};
use signex_library::footprint_binding::decode_footprint_cell;
use signex_library::manufacturer::ManufacturerPart;
use signex_library::primitive::symbol::SymbolFile;
use signex_library::{FootprintFile, LibraryFile};

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into()
}

fn sch(name: &str) -> SourceLib {
    SourceLib {
        released: false,
        path: format!("/in/{name}"),
        name: stem(name),
        body: SourceBody::Sch(SchLib::open(sample(name)).unwrap()),
    }
}

fn pcb(name: &str) -> SourceLib {
    SourceLib {
        released: false,
        path: format!("/in/{name}"),
        name: stem(name),
        body: SourceBody::Pcb(PcbLib::open(sample(name)).unwrap()),
    }
}

fn opts() -> LibraryOptions {
    LibraryOptions::fresh("Test")
}

const SCHLIBS: [&str; 3] = [
    "simple.SchLib",
    "graphic-mixed.SchLib",
    "long-pindata.SchLib",
];

fn convert(sources: &[SourceLib]) -> (Built, Report) {
    let mut report = Report::default();
    let built = build_library(sources, &opts(), &mut report, &mut |_, _, _| {}).expect("builds");
    (built, report)
}

fn snxlib(b: &Built) -> LibraryFile {
    let (_, text) = b
        .files
        .iter()
        .find(|(p, _)| p.ends_with(".snxlib"))
        .unwrap();
    LibraryFile::parse(text).expect("library parses")
}

/// The board conversion must hold every pad of the footprint: the layer
/// tokens must be ones it knows. Slotted pads are the one exception (signex
/// #3128), and they are reported by the converter.
fn placeable(fp: &signex_library::primitive::Footprint) {
    let drawn = signex_library::drawn_board_footprint(fp);
    for i in drawn.unheld_pads {
        assert!(
            fp.pads[i]
                .drill
                .as_ref()
                .is_some_and(|d| d.slot_length.is_some()),
            "pad {} of {} cannot be placed: layers {:?}",
            fp.pads[i].number,
            fp.name,
            fp.pads[i].layers
        );
    }
    assert_eq!(fp.anchor, [0.0, 0.0], "{}", fp.name);
}

/// Every reference in the library resolves, part numbers are unique, every
/// structured cell decodes.
fn self_check(b: &Built) {
    let lib = snxlib(b);
    let mut symbols = HashSet::new();
    let mut footprints = HashSet::new();
    for (path, text) in &b.files {
        if path.starts_with("symbols/") {
            for s in SymbolFile::from_toml_str(text).unwrap().symbols {
                symbols.insert(s.uuid.to_string());
            }
        } else if let Some(id) = path.strip_prefix("footprints/") {
            let f = FootprintFile::from_toml_str(text).unwrap();
            assert_eq!(f.footprints.len(), 1, "one footprint per file");
            assert_eq!(format!("{}.snxfpt", f.footprints[0].uuid), id);
            placeable(&f.footprints[0]);
            footprints.insert(f.footprints[0].uuid.to_string());
        }
    }
    let lid = lib.manifest.library_id.to_string();
    let mut pns = HashSet::new();
    let mut rows = 0;
    for (name, table) in &lib.tables {
        assert!(lib.manifest.classes.iter().any(|c| &c.key == name));
        for row in &table.rows {
            rows += 1;
            let cell = |c: &str| table.cell(row, c).unwrap().to_string();
            assert!(pns.insert(cell("internal_pn")), "duplicate part number");
            assert_eq!(cell("class"), *name);
            let sym: serde_json::Value = serde_json::from_str(&cell("symbol_ref")).unwrap();
            assert_eq!(sym["library_id"], lid.as_str());
            assert!(
                symbols.contains(sym["uuid"].as_str().unwrap()),
                "symbol resolves"
            );
            let (default, alts) = decode_footprint_cell(&cell("footprint_ref")).unwrap();
            for r in default.iter().chain(alts.iter().map(|a| &a.footprint)) {
                assert_eq!(r.library_id.to_string(), lid);
                assert!(
                    footprints.contains(&r.uuid.to_string()),
                    "footprint resolves"
                );
            }
            let _: ManufacturerPart = serde_json::from_str(&cell("primary_mpn")).unwrap();
            let _: Vec<serde_json::Value> = serde_json::from_str(&cell("alternates")).unwrap();
            let _: Vec<serde_json::Value> = serde_json::from_str(&cell("supply")).unwrap();
            let _: serde_json::Value = serde_json::from_str(&cell("parameters")).unwrap();
            assert_eq!(cell("content_hash").len(), 64);
            assert!(["Draft", "Released"].contains(&cell("state").as_str()));
            assert_eq!(cell("version"), "1.0.0");
        }
    }
    assert_eq!(rows, b.written.parts);
}

fn completeness(b: &Built, r: &Report) {
    let err = |k: ItemKind| {
        r.items()
            .iter()
            .filter(|i| i.kind == k && i.severity == Severity::Error)
            .count()
    };
    assert_eq!(b.source.parts, b.written.parts + err(ItemKind::Part));
    assert_eq!(b.source.symbols, b.written.symbols + err(ItemKind::Symbol));
    assert_eq!(
        b.source.footprints,
        b.written.footprints + err(ItemKind::Footprint)
    );
}

#[test]
fn every_sample_pair_converts_and_checks_out() {
    for s in SCHLIBS {
        let (b, r) = convert(&[sch(s), pcb("Simple.PcbLib")]);
        self_check(&b);
        completeness(&b, &r);
        assert!(b.written.parts > 0 && b.written.footprints > 0, "{s}");
        assert_eq!(b.written.symbols, b.source.symbols);
    }
    // All of them in one library.
    let all: Vec<SourceLib> = SCHLIBS
        .iter()
        .map(|s| sch(s))
        .chain([pcb("Simple.PcbLib")])
        .collect();
    let (b, r) = convert(&all);
    self_check(&b);
    completeness(&b, &r);
}

#[test]
fn converting_twice_is_byte_identical_and_order_free() {
    let a = convert(&[
        sch("simple.SchLib"),
        sch("graphic-mixed.SchLib"),
        pcb("Simple.PcbLib"),
    ])
    .0;
    let b = convert(&[
        pcb("Simple.PcbLib"),
        sch("graphic-mixed.SchLib"),
        sch("simple.SchLib"),
    ])
    .0;
    assert_eq!(a.library_id, b.library_id);
    assert_eq!(a.files, b.files);
}

#[test]
fn the_library_id_is_derived_and_overridable() {
    let a = convert(&[sch("simple.SchLib")]).0;
    let mut r = Report::default();
    let id = uuid::Uuid::from_u128(7);
    let o = LibraryOptions {
        library_id: Some(id),
        ..LibraryOptions::fresh("X")
    };
    let b = build_library(&[sch("simple.SchLib")], &o, &mut r, &mut |_, _, _| {}).unwrap();
    assert_eq!(b.library_id, id);
    assert_ne!(a.library_id, id);
    assert_eq!(snxlib(&b).manifest.library_id, id);
}

#[test]
fn the_class_follows_the_stored_prefix() {
    // Sample designators are real prefixes (`R?`) or the placeholder `*`.
    let (b, _) = convert(&[sch("simple.SchLib")]);
    let lib = snxlib(&b);
    assert!(lib.tables.contains_key("generic"));
    for (class, t) in &lib.tables {
        for row in &t.rows {
            let params = t.cell(row, "parameters").unwrap();
            let prefix = |p: &str| {
                params.contains(&format!(
                    "\"designator_prefix\":{{\"kind\":\"Text\",\"value\":\"{p}\"}}"
                ))
            };
            assert_eq!(class == "resistor", prefix("R"), "{class}: {params}");
            if class == "generic" {
                assert!(!prefix("R") && !prefix("C") && !prefix("L"));
            }
        }
    }
}

fn with_designator(mut c: Component, name: &str, designator: &str) -> Component {
    c.lib_reference = name.into();
    c.designator.get_or_insert_with(Default::default).text = designator.into();
    c
}

fn schlib_of(comps: Vec<Component>) -> SourceLib {
    SourceLib {
        released: false,
        path: "/in/parts.SchLib".into(),
        name: "parts".into(),
        body: SourceBody::Sch(SchLib {
            components: comps,
            ..Default::default()
        }),
    }
}

fn base_component() -> Component {
    SchLib::open(sample("simple.SchLib"))
        .unwrap()
        .components
        .remove(0)
}

#[test]
fn prefixes_pick_template_classes_and_repeated_names_get_suffixes() {
    let base = base_component();
    let comps = vec![
        with_designator(base.clone(), "RES", "R?"),
        with_designator(base.clone(), "CAP", "C?"),
        with_designator(base.clone(), "IND", "L?"),
        with_designator(base.clone(), "DIODE", "D?"),
        with_designator(base.clone(), "RES", "R?"),
        with_designator(base.clone(), "RES", "R?"),
    ];
    let (b, r) = convert(&[schlib_of(comps)]);
    self_check(&b);
    let lib = snxlib(&b);
    let keys: Vec<&String> = lib.tables.keys().collect();
    assert_eq!(keys, ["capacitor", "diode", "inductor", "resistor"]);
    let labels: Vec<&str> = lib
        .manifest
        .classes
        .iter()
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(labels, ["Capacitor", "Diode", "Inductor", "Resistor"]);
    let pns: Vec<String> = lib.tables["resistor"]
        .rows
        .iter()
        .map(|row| {
            lib.tables["resistor"]
                .cell(row, "internal_pn")
                .unwrap()
                .to_string()
        })
        .collect();
    let set: HashSet<_> = pns.iter().cloned().collect();
    assert_eq!(
        set,
        HashSet::from(["RES".into(), "RES~2".into(), "RES~3".into()])
    );
    assert_eq!(
        r.items()
            .iter()
            .filter(|i| i.code == "part.pn_collision")
            .count(),
        2
    );
    let t = &lib.tables["resistor"];
    assert!(
        t.cell(&t.rows[0], "parameters")
            .unwrap()
            .contains("\"designator_prefix\":{\"kind\":\"Text\",\"value\":\"R\"}")
    );
    // Ids are derived from the name, not random.
    let (again, _) = convert(&[schlib_of(vec![with_designator(base.clone(), "RES", "R?")])]);
    assert!(b.files.iter().any(|f| f.0.starts_with("symbols/")));
    assert!(again.written.parts == 1);
}

// --- footprint binding ---------------------------------------------------

fn pcblib_with(names: &[&str]) -> SourceLib {
    let mut lib = PcbLib::open(sample("Simple.PcbLib")).unwrap();
    let first = lib.footprints[0].clone();
    lib.footprints = names
        .iter()
        .map(|n| {
            let mut f = first.clone();
            f.name = (*n).to_string();
            f
        })
        .collect();
    SourceLib {
        released: false,
        path: "/in/fp.PcbLib".into(),
        name: "fp".into(),
        body: SourceBody::Pcb(lib),
    }
}

fn imp(name: &str, current: bool, map: &[(&str, &str)]) -> Implementation {
    Implementation {
        model_name: name.into(),
        model_type: "PCBLIB".into(),
        is_current: current,
        pin_map: map
            .iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect(),
        pin_map_unread: false,
    }
}

fn part_with(list: Vec<Implementation>) -> SourceLib {
    let mut c = with_designator(base_component(), "PART", "U?");
    c.implementations = list;
    schlib_of(vec![c])
}

type Bound = (Option<String>, Vec<(String, Vec<(String, String)>)>, String);

fn footprint_cell(b: &Built) -> Bound {
    let lib = snxlib(b);
    let t = lib.tables.values().next().expect("one table");
    let row = &t.rows[0];
    let (d, a) = decode_footprint_cell(t.cell(row, "footprint_ref").unwrap()).unwrap();
    (
        d.map(|r| r.uuid.to_string()),
        a.into_iter()
            .map(|x| {
                (
                    x.footprint.uuid.to_string(),
                    x.pin_map_overrides
                        .into_iter()
                        .map(|o| (o.symbol_pin_number, o.footprint_pad_number))
                        .collect(),
                )
            })
            .collect(),
        t.cell(row, "pin_map_overrides").unwrap().to_string(),
    )
}

fn footprint_id(b: &Built, name: &str) -> String {
    b.files
        .iter()
        .filter(|(p, _)| p.starts_with("footprints/"))
        .map(|(_, t)| {
            FootprintFile::from_toml_str(t)
                .unwrap()
                .footprints
                .remove(0)
        })
        .find(|f| f.name == name)
        .unwrap()
        .uuid
        .to_string()
}

#[test]
fn one_footprint_binds_as_the_default() {
    let (b, r) = convert(&[
        pcblib_with(&["FP_A", "FP_B", "FP_C"]),
        part_with(vec![imp("FP_B", true, &[])]),
    ]);
    let (d, alts, map) = footprint_cell(&b);
    assert_eq!(d.unwrap(), footprint_id(&b, "FP_B"));
    assert!(alts.is_empty());
    assert_eq!(map, "[]");
    self_check(&b);
    completeness(&b, &r);
}

#[test]
fn two_and_three_footprints_bind_default_then_alternates_in_list_order() {
    let list = vec![
        imp("FP_A", false, &[]),
        imp("FP_B", true, &[]),
        imp("FP_C", false, &[]),
    ];
    let (b, _) = convert(&[pcblib_with(&["FP_A", "FP_B", "FP_C"]), part_with(list)]);
    let (d, alts, _) = footprint_cell(&b);
    assert_eq!(d.unwrap(), footprint_id(&b, "FP_B"));
    let ids: Vec<&String> = alts.iter().map(|a| &a.0).collect();
    assert_eq!(ids, [&footprint_id(&b, "FP_A"), &footprint_id(&b, "FP_C")]);
    self_check(&b);

    let two = vec![imp("FP_A", true, &[]), imp("FP_C", false, &[])];
    let (b2, _) = convert(&[pcblib_with(&["FP_A", "FP_B", "FP_C"]), part_with(two)]);
    let (d2, alts2, _) = footprint_cell(&b2);
    assert_eq!(d2.unwrap(), footprint_id(&b2, "FP_A"));
    assert_eq!(alts2.len(), 1);
    assert_eq!(alts2[0].0, footprint_id(&b2, "FP_C"));
}

#[test]
fn a_remapped_pin_becomes_a_pin_map_override() {
    let list = vec![
        imp("FP_A", true, &[("1", "1"), ("2", "3")]),
        imp("FP_B", false, &[("1", "2")]),
    ];
    let (b, _) = convert(&[pcblib_with(&["FP_A", "FP_B"]), part_with(list)]);
    let (_, alts, map) = footprint_cell(&b);
    assert_eq!(
        map,
        r#"[{"symbol_pin_number":"2","footprint_pad_number":"3"}]"#
    );
    assert_eq!(alts[0].1, vec![("1".to_string(), "2".to_string())]);
}

#[test]
fn a_missing_footprint_skips_the_part_with_an_error() {
    let list = vec![imp("FP_A", true, &[]), imp("Absent", false, &[])];
    let (b, r) = convert(&[pcblib_with(&["FP_A"]), part_with(list)]);
    assert_eq!(b.written.parts, 0);
    assert_eq!(b.source.parts, 1);
    let item = r
        .items()
        .iter()
        .find(|i| i.code == "part.footprint_missing")
        .unwrap();
    assert_eq!(item.severity, Severity::Error);
    assert_eq!(item.kind, ItemKind::Part);
    assert_eq!(item.source, "parts:PART");
    assert!(item.id.is_some(), "the id the part would have had");
    completeness(&b, &r);
}

#[test]
fn a_part_naming_no_footprint_gets_a_row_without_one() {
    let (b, _) = convert(&[part_with(vec![])]);
    let (d, alts, _) = footprint_cell(&b);
    assert!(d.is_none() && alts.is_empty());
}

#[test]
fn sim_models_are_info_only() {
    let mut sim = imp("SPICE", false, &[]);
    sim.model_type = "SIM".into();
    let (b, r) = convert(&[
        pcblib_with(&["FP_A"]),
        part_with(vec![sim, imp("FP_A", true, &[])]),
    ]);
    assert_eq!(b.written.parts, 1);
    let i = r
        .items()
        .iter()
        .find(|i| i.code == "part.model_skipped")
        .unwrap();
    assert_eq!(i.severity, Severity::Info);
}

// --- frames ----------------------------------------------------------------

#[test]
fn footprint_coordinates_are_y_down() {
    // Pads are written with +y down: the source's y is negated, x is kept.
    let src = PcbLib::open(sample("Simple.PcbLib")).unwrap();
    let (b, _) = convert(&[pcb("Simple.PcbLib")]);
    let mut asymmetric = 0;
    for f in &src.footprints {
        let out = b
            .files
            .iter()
            .filter(|(p, _)| p.starts_with("footprints/"))
            .map(|(_, t)| {
                FootprintFile::from_toml_str(t)
                    .unwrap()
                    .footprints
                    .remove(0)
            })
            .find(|o| o.name == f.name)
            .unwrap();
        assert_eq!(out.pads.len(), f.primitives.pads.len());
        for (o, p) in out.pads.iter().zip(&f.primitives.pads) {
            assert!(
                (o.position[0] - p.position.x * 0.0254).abs() < 1e-4,
                "{}",
                f.name
            );
            assert!(
                (o.position[1] + p.position.y * 0.0254).abs() < 1e-4,
                "{}",
                f.name
            );
            if p.position.y.abs() > 1.0 {
                asymmetric += 1;
            }
        }
    }
    assert!(
        asymmetric > 0,
        "the samples must exercise a pad off the x axis"
    );
}

#[test]
fn counts_match_the_sources() {
    let (b, _) = convert(&[sch("simple.SchLib"), pcb("Simple.PcbLib")]);
    let s = SchLib::open(sample("simple.SchLib"))
        .unwrap()
        .components
        .len();
    let p = PcbLib::open(sample("Simple.PcbLib"))
        .unwrap()
        .footprints
        .len();
    assert_eq!(
        (b.source.symbols, b.source.parts, b.source.footprints),
        (s, s, p)
    );
}

// --- part fields -----------------------------------------------------------

fn param(c: &mut Component, name: &str, text: &str) {
    c.parameters.push(altium_format::sch::Parameter {
        name: name.into(),
        text: altium_format::sch::Text {
            text: text.into(),
            ..Default::default()
        },
    });
}

#[test]
fn parameters_land_in_the_part_row() {
    let mut c = with_designator(base_component(), "R_10K", "R?");
    c.parameters.clear();
    for (n, v) in [
        ("Manufacturer", "Acme"),
        ("Manufacturer Part Number", "A-1"),
        ("Manufacturer 2", "Beta"),
        ("Manufacturer Part Number 2", "B-2"),
        ("Supplier 1", "DigiKey"),
        ("Supplier Part Number 1", "SKU-1"),
        ("Supplier Order Qty 1", "50"),
        ("Datasheet", "https://example.com/ds.pdf"),
        ("HelpURL", "https://example.com/help"),
        ("Description", "10k thick film"),
        ("Value", "10 kOhm"),
        ("Tolerance", "1%"),
        ("Power", "0.1W"),
        ("Colour", "blue"),
    ] {
        param(&mut c, n, v);
    }
    let (b, r) = convert(&[schlib_of(vec![c])]);
    self_check(&b);
    let lib = snxlib(&b);
    let t = &lib.tables["resistor"];
    let row = &t.rows[0];
    let cell = |n: &str| t.cell(row, n).unwrap().to_string();
    assert_eq!(
        cell("primary_mpn"),
        r#"{"manufacturer":"Acme","mpn":"A-1","status":"Primary","notes":null}"#
    );
    assert_eq!(
        cell("alternates"),
        r#"[{"manufacturer":"Beta","mpn":"B-2","status":"Approved","notes":null}]"#
    );
    assert_eq!(
        cell("supply"),
        r#"[{"distributor":"DigiKey","sku":"SKU-1","url":null,"moq":50}]"#
    );
    assert_eq!(
        cell("datasheet"),
        r#"[{"kind":"url","url":"https://example.com/ds.pdf"},{"label":"HelpURL","kind":"url","url":"https://example.com/help"}]"#
    );
    assert_eq!(cell("description"), "10k thick film");
    let params = cell("parameters");
    assert!(
        params.contains(r#""value":{"kind":"Measurement","value":{"value":10000.0,"unit":"ohm"}}"#),
        "{params}"
    );
    assert!(
        params.contains(r#""tolerance":{"kind":"Measurement","value":{"value":1.0,"unit":"%"}}"#)
    );
    assert!(params.contains(r#""power":{"kind":"Measurement","value":{"value":0.1,"unit":"W"}}"#));
    assert!(params.contains(r#""Colour":{"kind":"Text","value":"blue"}"#));
    // The resistor template still wants a voltage? No: it asks for value,
    // tolerance and power, all present.
    assert!(
        r.items()
            .iter()
            .all(|i| i.code != "part.template_incomplete")
    );
}

// --- versions, stamps, updates ---------------------------------------------

fn previous_of(
    b: &Built,
) -> (
    std::collections::HashMap<uuid::Uuid, signex_altium_convert::libset::PreviousRow>,
    std::collections::HashMap<uuid::Uuid, signex_altium_convert::libset::PreviousPrimitive>,
    std::collections::HashMap<uuid::Uuid, signex_altium_convert::libset::PreviousPrimitive>,
) {
    use signex_altium_convert::libset::{PreviousPrimitive, PreviousRow};
    let lib = snxlib(b);
    let mut rows = std::collections::HashMap::new();
    for t in lib.tables.values() {
        for r in &t.rows {
            let c = |n: &str| t.cell(r, n).unwrap().to_string();
            let time = |n: &str| {
                chrono::DateTime::parse_from_rfc3339(&c(n))
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            };
            let mut hash = [0u8; 32];
            for (i, h) in hash.iter_mut().enumerate() {
                *h = u8::from_str_radix(&c("content_hash")[2 * i..2 * i + 2], 16).unwrap();
            }
            rows.insert(
                uuid::Uuid::parse_str(&c("row_id")).unwrap(),
                PreviousRow {
                    state: signex_library::LifecycleState::Draft,
                    released: false,
                    created: time("created"),
                    updated: time("updated"),
                    version: c("version"),
                    content_hash: hash,
                    symbol_version: c("symbol_version"),
                    footprint_version: c("footprint_version"),
                },
            );
        }
    }
    let mut syms = std::collections::HashMap::new();
    let mut fps = std::collections::HashMap::new();
    for (path, text) in &b.files {
        if path.starts_with("symbols/") {
            for s in &SymbolFile::from_toml_str(text).unwrap().symbols {
                syms.insert(s.uuid, PreviousPrimitive::of_symbol(s).unwrap());
            }
        } else if path.starts_with("footprints/") {
            for f in &FootprintFile::from_toml_str(text).unwrap().footprints {
                fps.insert(f.uuid, PreviousPrimitive::of_footprint(f).unwrap());
            }
        }
    }
    (rows, syms, fps)
}

fn versions(b: &Built) -> (Vec<String>, Vec<String>, Vec<String>) {
    let lib = snxlib(b);
    let mut rows = Vec::new();
    for t in lib.tables.values() {
        for r in &t.rows {
            rows.push(format!(
                "{}|{}|{}",
                t.cell(r, "version").unwrap(),
                t.cell(r, "symbol_version").unwrap(),
                t.cell(r, "footprint_version").unwrap()
            ));
        }
    }
    let mut syms = Vec::new();
    let mut fps = Vec::new();
    for (p, t) in &b.files {
        if p.starts_with("symbols/") {
            syms.extend(
                SymbolFile::from_toml_str(t)
                    .unwrap()
                    .symbols
                    .iter()
                    .map(|s| s.version.clone()),
            );
        } else if p.starts_with("footprints/") {
            fps.extend(
                FootprintFile::from_toml_str(t)
                    .unwrap()
                    .footprints
                    .iter()
                    .map(|f| f.version.clone()),
            );
        }
    }
    (rows, syms, fps)
}

#[test]
fn an_update_bumps_only_what_changed_and_carries_versions_into_the_row() {
    let list = vec![imp("FP_A", true, &[]), imp("FP_B", false, &[])];
    let sources = |pin: &str| {
        let mut part = part_with(list.clone());
        if let SourceBody::Sch(l) = &mut part.body {
            l.components[0].pins[0].name = pin.into();
        }
        [pcblib_with(&["FP_A", "FP_B"]), part]
    };
    let (first, _) = convert(&sources("IN"));
    let (rows, syms, fps) = previous_of(&first);

    // Unchanged source: nothing moves, byte for byte.
    let mut o = LibraryOptions::fresh("Test");
    (o.previous, o.previous_symbols, o.previous_footprints) =
        (rows.clone(), syms.clone(), fps.clone());
    o.now = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let mut r = Report::default();
    let same = build_library(&sources("IN"), &o, &mut r, &mut |_, _, _| {}).unwrap();
    assert_eq!(same.files, first.files);

    // The symbol changes: its version, the row's symbol_version and the row
    // version rise; footprints stay.
    let mut r = Report::default();
    let changed = build_library(&sources("OUT"), &o, &mut r, &mut |_, _, _| {}).unwrap();
    let (rows_v, syms_v, fps_v) = versions(&changed);
    assert_eq!(syms_v, ["1.0.1"]);
    assert_eq!(rows_v, ["1.0.1|1.0.1|1.0.0"]);
    assert!(fps_v.iter().all(|v| v == "1.0.0"));
    // The alternate footprint is bound at its footprint's version.
    let lib = snxlib(&changed);
    let t = lib.tables.values().next().unwrap();
    let (_, alts) = decode_footprint_cell(t.cell(&t.rows[0], "footprint_ref").unwrap()).unwrap();
    assert_eq!(alts[0].version, "1.0.0");
}

#[test]
fn stamps_come_from_the_sources_never_the_clock() {
    let mut lib = sch("simple.SchLib");
    let t = std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123);
    if let SourceBody::Sch(l) = &mut lib.body {
        l.modified = Some(t);
    }
    let (b, _) = convert(&[lib]);
    let text = b
        .files
        .iter()
        .find(|(p, _)| p.ends_with(".snxlib"))
        .unwrap()
        .1
        .clone();
    assert!(text.contains("2023-11-14T22:13:20+00:00"), "{text}");
    // No recorded time falls back to the epoch.
    let mut none = sch("simple.SchLib");
    if let SourceBody::Sch(l) = &mut none.body {
        l.modified = None;
    }
    let (b, _) = convert(&[none]);
    let text = b
        .files
        .iter()
        .find(|(p, _)| p.ends_with(".snxlib"))
        .unwrap()
        .1
        .clone();
    assert!(text.contains("1970-01-01T00:00:00+00:00"));
}

#[test]
fn a_footprint_name_defined_twice_is_approximated_and_names_both_sources() {
    let mut a = pcblib_with(&["Same"]);
    a.path = "/in/a.PcbLib".into();
    a.name = "a".into();
    let mut b = pcblib_with(&["Same"]);
    b.path = "/in/b.PcbLib".into();
    b.name = "b".into();
    let (_, r) = convert(&[a, b]);
    let i = r
        .items()
        .iter()
        .find(|i| i.code == "footprint.name_collision")
        .unwrap();
    assert_eq!(i.severity, Severity::Approximated);
    assert!(
        i.message.contains("b:Same") && i.message.contains("a:Same"),
        "{}",
        i.message
    );
}

#[test]
fn a_part_name_the_writer_would_refuse_is_cleaned() {
    let c = with_designator(base_component(), "BAD\tNAME'''\n", "U?");
    let (b, r) = convert(&[schlib_of(vec![c])]);
    assert_eq!(b.written.parts, 1);
    let lib = snxlib(&b);
    let t = lib.tables.values().next().unwrap();
    assert_eq!(
        t.cell(&t.rows[0], "internal_pn").unwrap(),
        "BAD NAME\u{2019}\u{2019}\u{2019} "
    );
    assert!(r.items().iter().any(|i| i.code == "part.text_sanitized"));
    self_check(&b);
}

#[test]
fn the_version_info_storage_is_reported_not_converted() {
    let (b, r) = convert(&[pcb("Simple.PcbLib")]);
    assert_eq!(b.source.footprints, 13);
    let skipped: Vec<_> = r
        .items()
        .iter()
        .filter(|i| i.code == "library.storage_skipped")
        .collect();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].detail["version_info"], true);
}

#[test]
fn each_record_read_with_an_assumed_pipe_is_reported() {
    let mut c = base_component();
    c.ansi_pipe_assumed = 2;
    let (b, r) = convert(&[schlib_of(vec![c])]);
    assert_eq!(b.written.parts, 1);
    let n = r
        .items()
        .iter()
        .filter(|i| i.code == "part.ansi_pipe_assumed")
        .count();
    assert_eq!(n, 2);
}

#[test]
fn a_source_saved_again_with_a_newer_time_changes_nothing() {
    let mk = |secs: u64| {
        let mut l = sch("simple.SchLib");
        if let SourceBody::Sch(s) = &mut l.body {
            s.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
        }
        let mut p = pcb("Simple.PcbLib");
        if let SourceBody::Pcb(s) = &mut p.body {
            s.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
        }
        [l, p]
    };
    let (first, _) = convert(&mk(1_700_000_000));
    let (rows, syms, fps) = previous_of(&first);
    let mut o = LibraryOptions::fresh("Test");
    (o.previous, o.previous_symbols, o.previous_footprints) = (rows, syms, fps);
    o.previous_files = first
        .files
        .iter()
        .filter(|(p, _)| p.starts_with("symbols/") || p.starts_with("footprints/"))
        .cloned()
        .collect();
    o.now = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let mut r = Report::default();
    let again = build_library(&mk(1_750_000_000), &o, &mut r, &mut |_, _, _| {}).unwrap();
    assert_eq!(again.files, first.files);
}

// --- integrated libraries and 3D models -------------------------------------

fn int_sources(path: &Path) -> Vec<SourceLib> {
    let int = altium_format::intlib::IntLib::open(path).unwrap();
    let name = stem(&path.file_name().unwrap().to_string_lossy());
    let p = path.to_string_lossy().into_owned();
    int.schlibs
        .into_iter()
        .map(|e| (SourceBody::Sch(e.lib), true))
        .chain(
            int.pcblibs
                .into_iter()
                .map(|e| (SourceBody::Pcb(e.lib), true)),
        )
        .map(|(body, released)| SourceLib {
            released,
            path: p.clone(),
            name: name.clone(),
            body,
        })
        .collect()
}

fn sha(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn footprints_of(b: &Built) -> Vec<FootprintFile> {
    b.files
        .iter()
        .filter(|(p, _)| p.starts_with("footprints/"))
        .map(|(_, t)| FootprintFile::from_toml_str(t).unwrap())
        .collect()
}

#[test]
fn a_packed_intlib_converts_with_released_parts() {
    let sch_bytes = std::fs::read(sample("simple.SchLib")).unwrap();
    let pcb_bytes = std::fs::read(sample("Simple.PcbLib")).unwrap();
    let packed = altium_format::intlib::pack(Some(&sch_bytes), Some(&pcb_bytes), None).unwrap();
    let dir = std::env::temp_dir().join(format!("snx-intlib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Packed.IntLib");
    std::fs::write(&path, packed).unwrap();
    let (b, r) = convert(&int_sources(&path));
    let (pair, _) = convert(&[sch("simple.SchLib"), pcb("Simple.PcbLib")]);
    assert_eq!(b.written, pair.written);
    self_check(&b);
    completeness(&b, &r);
    let lib = snxlib(&b);
    for t in lib.tables.values() {
        for row in &t.rows {
            assert_eq!(t.cell(row, "state").unwrap(), "Released");
            assert_eq!(t.cell(row, "released").unwrap(), "true");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn embedded_step_models_attach_with_the_hash_of_their_inflated_bytes() {
    let src = PcbLib::open(sample("Simple.PcbLib")).unwrap();
    let (b, r) = convert(&[pcb("Simple.PcbLib")]);
    assert_eq!(b.source.models, 2);
    assert_eq!(b.written.models, 2);
    completeness_models(&b, &r);
    let files = footprints_of(&b);
    let mut attached = 0;
    for f in &files {
        assert_eq!(f.footprints.len(), 1);
        let fp = &f.footprints[0];
        if let Some(a) = &fp.step_attachment {
            attached += 1;
            let model = src
                .models
                .iter()
                .find(|m| m.name == a.filename)
                .unwrap_or_else(|| panic!("{}", a.filename));
            assert_eq!(a.content_hash, sha(model.data.as_ref().unwrap()));
            assert_eq!(a.offset_xyz, [0.0, 0.0, 0.0]);
            assert_eq!(a.rotation_xyz, [0.0, 0.0, 0.0]);
            assert_eq!(f.models.len(), 1);
            f.verify_models().unwrap();
            assert_eq!(
                f.step_model(fp).unwrap().unwrap(),
                *model.data.as_ref().unwrap()
            );
        }
    }
    assert_eq!(attached, 2);
    // Files with a model use the embedded-model token.
    let token = |f: &FootprintFile| {
        f.to_toml_string()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    assert!(files.iter().any(|f| token(f).contains("1.4")));
}

fn completeness_models(b: &Built, r: &Report) {
    let errors = r
        .items()
        .iter()
        .filter(|i| i.kind == ItemKind::Model && i.severity == Severity::Error)
        .count();
    assert_eq!(b.source.models, b.written.models + errors);
}

#[test]
fn models_skip_embeds_nothing_and_reports_each() {
    let mut r = Report::default();
    let o = LibraryOptions {
        include_models: false,
        ..LibraryOptions::fresh("Test")
    };
    let b = build_library(&[pcb("Simple.PcbLib")], &o, &mut r, &mut |_, _, _| {}).unwrap();
    assert_eq!((b.source.models, b.written.models), (0, 0));
    assert!(
        footprints_of(&b)
            .iter()
            .all(|f| f.models.is_empty() && f.footprints[0].step_attachment.is_none())
    );
    assert_eq!(
        r.items()
            .iter()
            .filter(|i| i.code == "model.skipped")
            .count(),
        2
    );
}

#[test]
fn models_that_cannot_be_embedded_are_reported_as_errors() {
    for (what, code) in [
        ("not step", "model.not_step"),
        ("external", "model.external"),
        ("missing", "model.missing"),
    ] {
        let mut l = pcb("Simple.PcbLib");
        if let SourceBody::Pcb(lib) = &mut l.body {
            match what {
                "not step" => lib.models[0].data = Some(b"PK\x03\x04 not a step".to_vec()),
                "external" => {
                    lib.models[0].data = None;
                    lib.models[0].embedded = false;
                }
                _ => lib.models[0].id = "{00000000-0000-0000-0000-000000000000}".into(),
            }
        }
        let (b, r) = convert(&[l]);
        assert_eq!((b.source.models, b.written.models), (2, 1), "{what}");
        let i = r
            .items()
            .iter()
            .find(|i| i.code == code)
            .unwrap_or_else(|| panic!("{what}"));
        assert_eq!((i.kind, i.severity), (ItemKind::Model, Severity::Error));
        completeness_models(&b, &r);
    }
}

/// The private corpus: skipped unless `ALP_PRIVATE_INTLIB_DIR` names a folder
/// holding integrated libraries. Nothing derived from them is committed.
#[test]
fn private_intlibs_convert_with_matching_counts() {
    let Some(dir) = std::env::var_os("ALP_PRIVATE_INTLIB_DIR") else {
        eprintln!("ALP_PRIVATE_INTLIB_DIR is unset; skipping");
        return;
    };
    let mut seen = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if !p
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("intlib"))
        {
            continue;
        }
        seen += 1;
        let int = altium_format::intlib::IntLib::open(&p).unwrap();
        let comps: usize = int.schlibs.iter().map(|s| s.lib.components.len()).sum();
        let fps: usize = int.pcblibs.iter().map(|s| s.lib.footprints.len()).sum();
        assert!(
            int.cross_ref_mismatches().is_empty(),
            "{:?}",
            int.cross_ref_mismatches()
        );
        let (b, r) = convert(&int_sources(&p));
        self_check(&b);
        completeness(&b, &r);
        completeness_models(&b, &r);
        assert_eq!(b.source.parts, comps);
        assert_eq!(b.source.footprints, fps);
        assert_eq!(b.written.footprints, fps);
        // Model hashes are those of the inflated streams.
        let mut hashes: Vec<String> = int
            .pcblibs
            .iter()
            .flat_map(|s| s.lib.models.iter())
            .filter_map(|m| m.data.as_ref().map(|d| sha(d)))
            .collect();
        hashes.sort();
        hashes.dedup();
        for f in footprints_of(&b) {
            f.verify_models().unwrap();
            if let Some(a) = &f.footprints[0].step_attachment {
                assert!(hashes.binary_search(&a.content_hash).is_ok());
            }
        }
        // Converting twice is byte-identical.
        let (again, _) = convert(&int_sources(&p));
        assert_eq!(again.files, b.files);
    }
    assert!(seen > 0, "no .IntLib in the folder");
}

#[test]
fn a_model_named_outside_the_file_is_read_from_models_dir() {
    let dir = std::env::temp_dir().join(format!("snx-mdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let step = b"ISO-10303-21;\nHEADER;\nENDSEC;\nEND-ISO-10303-21;\n";
    std::fs::write(dir.join("my-part.STEP"), step).unwrap();
    let mut l = pcb("Simple.PcbLib");
    // Make the first model external: drop it from the table and the embed flag.
    if let SourceBody::Pcb(lib) = &mut l.body {
        let id = lib.models[0].id.clone();
        for f in &mut lib.footprints {
            for b in &mut f.primitives.bodies {
                if b.params.string("MODELID") == id {
                    b.params = altium_format::Params::from_pairs([
                        ("MODELID", id.as_str()),
                        ("MODEL.NAME", "C:\\somewhere\\My-Part.step"),
                    ]);
                }
            }
        }
        lib.models.remove(0);
    }
    let run = |with_dir: bool| {
        let mut r = Report::default();
        let o = LibraryOptions {
            models_dir: with_dir.then(|| dir.clone()),
            ..LibraryOptions::fresh("T")
        };
        let src = SourceLib {
            released: false,
            path: l.path.clone(),
            name: l.name.clone(),
            body: match &l.body {
                SourceBody::Pcb(p) => SourceBody::Pcb(p.clone()),
                SourceBody::Sch(s) => SourceBody::Sch(s.clone()),
                SourceBody::Database(d) => SourceBody::Database(d.clone()),
            },
        };
        let b = build_library(&[src], &o, &mut r, &mut |_, _, _| {}).unwrap();
        (b, r)
    };
    let (b, _) = run(true);
    assert_eq!((b.source.models, b.written.models), (2, 2));
    let hashes: Vec<String> = footprints_of(&b)
        .iter()
        .filter_map(|f| {
            f.footprints[0]
                .step_attachment
                .as_ref()
                .map(|a| a.content_hash.clone())
        })
        .collect();
    assert!(hashes.contains(&sha(step)));
    let (b, r) = run(false);
    assert_eq!(b.written.models, 1);
    assert!(r.items().iter().any(|i| i.code == "model.external"));
    std::fs::remove_dir_all(dir).unwrap();
}
