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

fn frame(text: &str) -> Vec<u8> {
    let mut payload = text.as_bytes().to_vec();
    payload.push(0);
    let mut out = (payload.len() as u32).to_le_bytes().to_vec();
    out.extend(payload);
    out
}

fn symbol_with_map(extra: &[&str]) -> Vec<altium_format::sch::Implementation> {
    let mut s = Vec::new();
    for rec in [
        "|RECORD=1|LibReference=X|PartCount=2|",
        "|RECORD=44|",
        "|RECORD=45|ModelName=FP|ModelType=PCBLIB|IsCurrent=T|",
    ]
    .iter()
    .chain(extra)
    {
        s.extend(frame(rec));
    }
    altium_format::sch::parse_symbol_stream(&s)
        .unwrap()
        .implementations
}

#[test]
fn an_unread_pin_map_is_flagged_never_left_silently_empty() {
    // A bare record 46 (as in the samples) says nothing.
    assert!(!symbol_with_map(&["|RECORD=46|"])[0].pin_map_unread);
    // A record 46 with keys, or any record 47, is a map this reader skips.
    assert!(symbol_with_map(&["|RECORD=46|DesignatorCount=2|"])[0].pin_map_unread);
    assert!(symbol_with_map(&["|RECORD=46|", "|RECORD=47|"])[0].pin_map_unread);
    assert!(!symbol_with_map(&[])[0].pin_map_unread);
}

#[test]
fn no_sample_implementation_has_an_unread_pin_map() {
    let doc = SchDoc::open(sample("simple.SchDoc")).unwrap();
    for c in &doc.components {
        assert!(c.implementations.iter().all(|i| !i.pin_map_unread));
    }
}

#[test]
fn a_library_records_its_modified_time_when_the_file_has_one() {
    // Not every writer fills the root entry's time; when present it is after
    // the epoch, and never a 1601 zero.
    let lib = SchLib::open(sample("simple.SchLib")).unwrap();
    if let Some(t) = lib.modified {
        assert!(t > std::time::UNIX_EPOCH);
    }
}

fn binary_pin(mode: u8) -> Vec<u8> {
    let mut p = vec![2, 0, 0, 0, 0]; // record 2, reserved
    p.extend(1i16.to_le_bytes()); // owner part
    p.push(mode);
    p.extend([0, 0, 0, 0]); // symbols
    p.push(0); // description
    p.push(0); // formal type
    p.push(4); // electrical
    p.push(0x38); // conglomerate
    p.extend(10i16.to_le_bytes());
    p.extend(0i16.to_le_bytes());
    p.extend(0i16.to_le_bytes());
    p.extend([0, 0, 0, 0]);
    p.push(1);
    p.push(b'A');
    p.push(1);
    p.push(b'1');
    let mut out = (p.len() as u32 | 1 << 24).to_le_bytes().to_vec();
    out.extend(p);
    out
}

#[test]
fn a_binary_pin_display_mode_is_signed() {
    for (stored, want) in [(0u8, 0), (1, 1), (0xFF, -1)] {
        let mut s = frame("|RECORD=1|LibReference=X|PartCount=2|");
        s.extend(binary_pin(stored));
        let c = altium_format::sch::parse_symbol_stream(&s).unwrap();
        assert_eq!(c.pins.len(), 1);
        assert_eq!(c.pins[0].display_mode, want, "stored {stored}");
    }
}
