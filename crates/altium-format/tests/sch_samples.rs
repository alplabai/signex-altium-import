//! Against the sample files in `tests/samples/` (see `tests/samples/NOTICE.md`).

use altium_format::sch::{Electrical, Pt, SchDoc, SchLib};

fn sample(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

#[test]
fn schdoc_lifts_connectivity() {
    let doc = SchDoc::open(sample("simple.SchDoc")).unwrap();
    assert_eq!(doc.components.len(), 3);
    assert_eq!(doc.wires.len(), 9);
    assert_eq!(doc.net_labels.len(), 5);
    assert_eq!(doc.power_ports.len(), 3);
    assert_eq!(doc.ports.len(), 3);
    assert_eq!(doc.sheet_symbols.len(), 1);
    assert_eq!(doc.no_ercs.len(), 2);
    assert_eq!(doc.buses.len(), 1);

    let r = &doc.components[0];
    assert_eq!(r.lib_reference, "Resistor - Standard");
    assert_eq!(r.location, Pt::new(4400.0, 4600.0));
    assert_eq!(r.designator.as_ref().unwrap().text, "R?");
    assert_eq!(r.footprint(), Some("R0603L"));
    assert_eq!(r.pins.len(), 2);
    let pin1 = r.pins.iter().find(|p| p.designator == "1").unwrap();
    assert_eq!(pin1.electrical, Electrical::Passive);
    // The wire runs from the resistor's origin: pin 1's hot end is there.
    assert_eq!(pin1.hot_end(), Pt::new(4400.0, 4600.0));
    assert_eq!(r.parameter("Value"), Some("100k"));

    let sheet = &doc.sheet_symbols[0];
    assert_eq!(sheet.file_name, "Foobar.SchDoc");
    assert_eq!(sheet.entries.len(), 3);
    assert_eq!(sheet.entries[1].side, 1);
    assert_eq!(sheet.entries[2].distance, 300.0);
}

#[test]
fn schlib_lists_every_symbol() {
    let lib = SchLib::open(sample("simple.SchLib")).unwrap();
    assert_eq!(lib.components.len(), 10);
    let r = lib
        .components
        .iter()
        .find(|c| c.lib_reference == "Resistor - Standard")
        .unwrap();
    assert_eq!(r.pins.len(), 2);
    assert!(!r.shapes.is_empty());
    let pos = lib
        .components
        .iter()
        .find(|c| c.lib_reference == "Position")
        .unwrap();
    assert_eq!(pos.pins.len(), 27);
}

#[test]
fn schlib_handles_long_names_and_pin_text() {
    for name in ["graphic-mixed.SchLib", "long-pindata.SchLib"] {
        let lib = SchLib::open(sample(name)).unwrap();
        assert!(!lib.components.is_empty(), "{name}");
    }
}
