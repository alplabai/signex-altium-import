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

// --- display modes -----------------------------------------------------------

fn symbols_of(lib: &SchLib, report: &mut Report) -> Vec<signex_library::primitive::symbol::Symbol> {
    let text = symbol_file(lib, "t", report).to_toml_string().unwrap();
    SymbolFile::from_toml_str(&text).unwrap().symbols
}

#[test]
fn the_sample_multimode_symbol_keeps_both_bodies() {
    let lib = SchLib::open(sample("simple.SchLib")).unwrap();
    let src = lib
        .components
        .iter()
        .find(|c| c.lib_reference == "Multimode 1")
        .unwrap();
    let modes: std::collections::BTreeSet<i32> = src.pins.iter().map(|p| p.display_mode).collect();
    assert!(modes.contains(&0) && modes.contains(&1), "{modes:?}");
    let mut report = Report::default();
    let syms = symbols_of(&lib, &mut report);
    let s = syms.iter().find(|s| s.name == "Multimode 1").unwrap();
    let count = |style: u8| s.pins.iter().filter(|p| p.body_style == style).count();
    assert_eq!(
        count(1),
        src.pins.iter().filter(|p| p.display_mode == 0).count()
    );
    assert_eq!(
        count(2),
        src.pins.iter().filter(|p| p.display_mode == 1).count()
    );
    assert!(
        s.graphics.iter().any(|g| g.body_style == 2)
            || s.graphics.iter().all(|g| g.body_style != 2)
    );
    assert!(
        !report.entries().any(|(m, _)| m.contains("display mode")),
        "{report}"
    );
}

#[test]
fn a_third_display_mode_is_reported_and_dropped() {
    let mut lib = SchLib::open(sample("simple.SchLib")).unwrap();
    let c = lib
        .components
        .iter_mut()
        .find(|c| c.lib_reference == "Multimode 1")
        .unwrap();
    let mut extra = c.pins[0].clone();
    extra.display_mode = 2;
    extra.designator = "X3".into();
    c.pins.push(extra);
    let before = c.pins.len() - 1;
    let mut report = Report::default();
    let syms = symbols_of(&lib, &mut report);
    let s = syms.iter().find(|s| s.name == "Multimode 1").unwrap();
    assert_eq!(s.pins.len(), before);
    assert!(s.pins.iter().all(|p| p.number != "X3"));
    let item = report
        .items()
        .iter()
        .find(|i| i.code == "symbol.display_mode_dropped")
        .unwrap();
    assert_eq!(
        item.severity,
        signex_altium_convert::report::Severity::Dropped
    );
}

#[test]
fn an_item_for_no_particular_mode_belongs_to_both_bodies() {
    let mut lib = SchLib::open(sample("simple.SchLib")).unwrap();
    let c = lib
        .components
        .iter_mut()
        .find(|c| c.lib_reference == "Multimode 1")
        .unwrap();
    c.pins[0].display_mode = -1;
    let n = c.pins[0].designator.clone();
    let mut report = Report::default();
    let syms = symbols_of(&lib, &mut report);
    let s = syms.iter().find(|s| s.name == "Multimode 1").unwrap();
    assert_eq!(s.pins.iter().find(|p| p.number == n).unwrap().body_style, 0);
}

fn pin_with_edges(inner: u8, outer: u8) -> (SymbolFile, Report) {
    let mut lib = SchLib::open(sample("simple.SchLib")).unwrap();
    lib.components.truncate(1);
    let pin = &mut lib.components[0].pins[0];
    pin.display_mode = 0;
    pin.symbols.inner_edge = inner;
    pin.symbols.outer_edge = outer;
    let mut report = Report::default();
    (symbol_file(&lib, "t", &mut report), report)
}

#[test]
fn clock_and_inversion_land_in_the_outside_edge_column() {
    use signex_library::primitive::symbol::PinSymbolKind;
    let (f, r) = pin_with_edges(3, 0);
    let p = &f.symbols[0].pins[0];
    assert_eq!(p.outside_edge_symbol, PinSymbolKind::ClockEdge);
    assert_eq!(p.inside_edge_symbol, PinSymbolKind::None);
    assert!(r.is_empty());
    let (f, _) = pin_with_edges(0, 1);
    assert_eq!(f.symbols[0].pins[0].outside_edge_symbol, PinSymbolKind::Dot);
}

#[test]
fn a_pin_with_both_edge_symbols_keeps_the_dot_and_reports_the_loss() {
    use signex_library::primitive::symbol::PinSymbolKind;
    let (f, r) = pin_with_edges(3, 1);
    assert_eq!(f.symbols[0].pins[0].outside_edge_symbol, PinSymbolKind::Dot);
    let i = r
        .items()
        .iter()
        .find(|i| i.code == "symbol.pin_decoration_merged")
        .unwrap();
    assert_eq!(
        i.severity,
        signex_altium_convert::report::Severity::Approximated
    );
}
