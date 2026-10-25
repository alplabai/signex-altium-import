use altium_format::pcb::PcbDoc;
use signex_altium_convert::Report;
use signex_types::format::SnxPcb;
use signex_types::pcb::{PadShape, PadType, Point};

fn sample(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

/// The renderer's pad placement: origin + R(rotation) * local.
fn pad_world(fp: &signex_types::pcb::Footprint, local: Point) -> Point {
    let (s, c) = fp.rotation.to_radians().sin_cos();
    Point::new(
        fp.position.x + local.x * c - local.y * s,
        fp.position.y + local.x * s + local.y * c,
    )
}

/// CSV coordinates are board-relative with Y up; the 60 x 40 mm board's
/// top edge is Y = 40.
fn csv_to_signex(x: f64, y: f64) -> Point {
    Point::new(x, 40.0 - y)
}

fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
}

#[test]
fn converted_board_round_trips_and_matches_the_object_list() {
    let doc = PcbDoc::open(sample("simple.PcbDoc")).unwrap();
    let mut report = Report::default();
    let board = signex_altium_convert::pcb::convert(&doc, &mut report);

    let text = SnxPcb::new(board).write_string().unwrap();
    let board = SnxPcb::parse(&text).unwrap().board;

    assert_eq!(board.footprints.len(), 3);
    assert_eq!(board.vias.len(), 2);
    assert_eq!(board.nets.len(), 4);
    // Top Layer: 2 tracks, Bottom Layer: 1 track (Net1 everywhere).
    assert_eq!(board.segments.len(), 3);
    assert!(board.segments.iter().all(|s| s.net == 4));
    assert_eq!(board.zones.len(), 1);

    // Outline: 60 x 40 mm with its top-left at the origin.
    let xs: Vec<f64> = board.outline.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = board.outline.iter().map(|p| p.y).collect();
    assert_eq!(xs.iter().cloned().fold(f64::MAX, f64::min), 0.0);
    assert_eq!(xs.iter().cloned().fold(f64::MIN, f64::max), 60.0);
    assert_eq!(ys.iter().cloned().fold(f64::MIN, f64::max), 40.0);

    // Pad,,Top Layer,,Net1,,8,11 -- 1.85 mm rounded rectangle at 50 %.
    let tp = board
        .footprints
        .iter()
        .find(|f| f.footprint_id == "TPS180")
        .unwrap();
    let pad = &tp.pads[0];
    assert!(close(pad_world(tp, pad.position), csv_to_signex(8.0, 11.0)));
    assert_eq!(pad.shape, PadShape::RoundRect);
    assert_eq!(pad.roundrect_ratio, 0.25);
    assert_eq!(pad.pad_type, PadType::Smd);
    assert_eq!(pad.net.as_ref().unwrap().name, "Net1");

    // The capacitor's pads are turned 90 degrees in a 0 degree footprint:
    // 0.84 x 0.75 becomes 0.75 x 0.84.
    let cap = board
        .footprints
        .iter()
        .find(|f| f.footprint_id == "CAPC1608X09L")
        .unwrap();
    assert_eq!(cap.pads.len(), 2);
    for (num, x) in [("1", 7.275), ("2", 8.725)] {
        let p = cap.pads.iter().find(|p| p.number == num).unwrap();
        assert!(
            close(pad_world(cap, p.position), csv_to_signex(x, 21.0)),
            "pad {num}"
        );
        assert_eq!((p.size.x, p.size.y), (0.75, 0.84));
    }
    // Silkscreen and courtyard lines belong to the footprint.
    assert!(cap.graphics.iter().any(|g| g.layer == "TopSilkscreen"));
    assert!(cap.graphics.iter().any(|g| g.layer == "TopCourtyard"));
    assert!(cap.graphics.iter().any(|g| g.layer == "TopFab"));

    // Via,,MultiLayer,,Net1,Free,18,15
    assert!(
        board
            .vias
            .iter()
            .any(|v| close(v.position, csv_to_signex(18.0, 15.0)))
    );
}
