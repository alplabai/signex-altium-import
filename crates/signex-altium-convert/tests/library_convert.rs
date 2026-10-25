use altium_format::pcb::PcbLib;
use altium_format::sch::SchLib;
use signex_altium_convert::Report;
use signex_altium_convert::library::{footprint_file, symbol_file};
use signex_library::primitive::footprint::{FootprintFile, PadKind, PadShape};
use signex_library::primitive::symbol::{PinOrientation, SymbolFile};

fn sample(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

#[test]
fn schlib_round_trips_through_snxsym() {
    let lib = SchLib::open(sample("simple.SchLib")).unwrap();
    let mut report = Report::default();
    let file = symbol_file(&lib, "simple", &mut report);
    let text = file.to_toml_string().unwrap();
    let back = SymbolFile::from_toml_str(&text).unwrap();

    assert_eq!(back.symbols.len(), lib.components.len());
    let r = back
        .symbols
        .iter()
        .find(|s| s.name == "Resistor - Standard")
        .unwrap();
    assert_eq!(r.pins.len(), 2);
    assert_eq!(r.designator, "R?");
    // Pin 2 points down from the body at (0, -250 mil): its electrical end
    // is 50 mil further, and it faces up toward the body.
    let p2 = r.pins.iter().find(|p| p.number == "2").unwrap();
    assert_eq!(p2.position, [0.0, -7.62]);
    assert_eq!(p2.orientation, PinOrientation::Up);
    assert_eq!(p2.length, 1.27);
    assert!(!r.graphics.is_empty());

    let pos = back.symbols.iter().find(|s| s.name == "Position").unwrap();
    assert_eq!(pos.pins.len(), 27);
}

#[test]
fn every_sample_schlib_converts() {
    for name in ["graphic-mixed.SchLib", "long-pindata.SchLib"] {
        let lib = SchLib::open(sample(name)).unwrap();
        let file = symbol_file(&lib, name, &mut Report::default());
        let text = file.to_toml_string().unwrap();
        SymbolFile::from_toml_str(&text).unwrap();
    }
}

#[test]
fn pcblib_round_trips_through_snxfpt() {
    let lib = PcbLib::open(sample("Simple.PcbLib")).unwrap();
    let mut report = Report::default();
    let file = footprint_file(&lib, "Simple", &mut report);
    let text = file.to_toml_string().unwrap();
    let back = FootprintFile::from_toml_str(&text).unwrap();
    assert_eq!(back.footprints.len(), lib.footprints.len());

    let four = back
        .footprints
        .iter()
        .find(|f| f.name == "Four pads")
        .unwrap();
    assert_eq!(four.pads.len(), 4);

    let cap = back
        .footprints
        .iter()
        .find(|f| f.name == "CAPC1608X09L")
        .unwrap();
    assert_eq!(cap.pads.len(), 2);
    assert!(cap.pads.iter().all(|p| p.kind == PadKind::Smd));
    assert!(
        cap.pads
            .iter()
            .all(|p| matches!(p.shape, PadShape::RoundRect { .. }))
    );
    // Mechanical 15 is the courtyard by the library's layer-kind table: a
    // closed rectangle (the centre cross drawn with it is dropped).
    assert_eq!(cap.courtyard.points.len(), 4);
    assert!(
        !report
            .entries()
            .any(|(m, _)| m.starts_with("open courtyard")),
        "{report}"
    );
}
