use altium_format::sch::SchDoc;
use signex_altium_convert::Report;
use signex_altium_convert::sch::{SchOptions, convert};
use signex_types::format::SnxSchematic;
use signex_types::schematic::{LabelType, Point, SymbolTransform};

fn sample(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4
}

#[test]
fn converted_sheet_round_trips_and_keeps_connectivity() {
    let doc = SchDoc::open(sample("simple.SchDoc")).unwrap();
    let mut report = Report::default();
    let sheet = convert(&doc, &SchOptions::default(), &mut report);

    // Survives the native writer and parser.
    let text = SnxSchematic::new(sheet).write_string().unwrap();
    let sheet = SnxSchematic::parse(&text).unwrap().sheet;

    assert_eq!(sheet.paper_size, "A4");
    assert_eq!(sheet.symbols.iter().filter(|s| !s.is_power).count(), 3);
    // VCC and GND; IN2 is an off-sheet connector, so a global label.
    assert_eq!(sheet.symbols.iter().filter(|s| s.is_power).count(), 2);
    assert!(
        sheet
            .labels
            .iter()
            .any(|l| l.label_type == LabelType::Global && l.text == "IN2")
    );
    // The power ports sit straight on U?.3 / U?.5; Signex needs a wire there.
    let bridges: Vec<_> = sheet
        .wires
        .iter()
        .filter(|w| close(w.start, w.end))
        .collect();
    assert_eq!(bridges.len(), 2);
    for b in &bridges {
        assert!(
            sheet
                .symbols
                .iter()
                .any(|s| s.is_power && close(s.position, b.start))
        );
    }
    assert_eq!(sheet.child_sheets.len(), 1);
    assert_eq!(sheet.child_sheets[0].filename, "Foobar.snxsch");
    assert_eq!(sheet.child_sheets[0].pins.len(), 3);
    assert_eq!(
        sheet
            .labels
            .iter()
            .filter(|l| l.label_type == LabelType::Net)
            .count(),
        5
    );
    assert_eq!(
        sheet
            .labels
            .iter()
            .filter(|l| l.label_type == LabelType::Hierarchical)
            .count(),
        3
    );

    // Every Altium wire end that touched a pin still touches that pin.
    let ends: Vec<Point> = sheet
        .wires
        .iter()
        .filter(|w| !close(w.start, w.end))
        .flat_map(|w| [w.start, w.end])
        .collect();
    let mut touching = 0;
    for sym in sheet.symbols.iter().filter(|s| !s.is_power) {
        let lib = &sheet.lib_symbols[&sym.lib_id];
        let t = SymbolTransform::from_symbol(sym);
        for lp in lib
            .pins
            .iter()
            .filter(|p| p.unit == 0 || p.unit == sym.unit)
        {
            if ends.iter().any(|e| close(*e, t.apply(lp.pin.position))) {
                touching += 1;
            }
        }
    }
    let altium_touching = doc
        .components
        .iter()
        .flat_map(|c| {
            c.pins
                .iter()
                .filter(move |p| p.owner_part_id <= 0 || p.owner_part_id == c.current_part_id)
        })
        .filter(|p| {
            let h = p.hot_end();
            doc.wires.iter().any(|w| {
                [w.first(), w.last()]
                    .into_iter()
                    .flatten()
                    .any(|e| (e.x - h.x).abs() < 0.01 && (e.y - h.y).abs() < 0.01)
            })
        })
        .count();
    assert!(altium_touching >= 4);
    assert_eq!(touching, altium_touching);

    let r = sheet.symbols.iter().find(|s| s.reference == "R?").unwrap();
    assert_eq!(r.footprint, "R0603L");
    assert_eq!(r.value, "100k");
    // Designator text was placed by the user in Altium; keep it.
    assert!(r.ref_text.is_some());
}
