//! Against `tests/samples/simple.PcbDoc`, whose object list Altium exported
//! to `simple.csv` (coordinates relative to the board origin, in mm).

use altium_format::pcb::{HoleShape, Layer, PadShape, PcbDoc, PcbLib};
use altium_format::prj::Project;
use altium_format::sch::Pt;

fn sample(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/samples")
        .join(name)
}

const MIL_PER_MM: f64 = 1.0 / 0.0254;

/// Board-relative mm, as the CSV reports it.
fn rel(doc: &PcbDoc, p: Pt) -> (f64, f64) {
    (
        ((p.x - doc.board.origin.x) / MIL_PER_MM * 1000.0).round() / 1000.0,
        ((p.y - doc.board.origin.y) / MIL_PER_MM * 1000.0).round() / 1000.0,
    )
}

fn mm(mils: f64) -> f64 {
    (mils / MIL_PER_MM * 1000.0).round() / 1000.0
}

#[test]
fn pcbdoc_matches_the_exported_object_list() {
    let doc = PcbDoc::open(sample("simple.PcbDoc")).unwrap();
    let p = &doc.primitives;
    assert_eq!(doc.nets, ["Net4", "Net3", "Net2", "Net1"]);
    assert_eq!(doc.components.len(), 3);
    assert_eq!(p.vias.len(), 2);
    assert_eq!(p.arcs.len(), 1);
    assert_eq!(p.pads.len(), 4);
    assert_eq!(p.tracks.len(), 19);
    assert_eq!(p.texts.len(), 6);

    // Track,,Mechanical 1,,No Net,Free,0,0,60,0,0.2
    let outline: Vec<_> = p
        .tracks
        .iter()
        .filter(|t| t.common.layer == Layer(57))
        .collect();
    assert_eq!(outline.len(), 4);
    assert_eq!(rel(&doc, outline[0].a), (0.0, 0.0));
    assert_eq!(rel(&doc, outline[0].b), (60.0, 0.0));
    assert_eq!(mm(outline[0].width), 0.2);

    // Track,,Top Layer,,Net1,Free,8,11,14,11,0.2
    let t = p
        .tracks
        .iter()
        .find(|t| t.common.layer == Layer::TOP && rel(&doc, t.a) == (8.0, 11.0))
        .unwrap();
    assert_eq!(doc.net_name(t.common.net), Some("Net1"));
    assert_eq!(rel(&doc, t.b), (14.0, 11.0));

    // Via,,MultiLayer,,Net1,Free,18,15 ... hole 0.2, diameter 0.4
    let v = p
        .vias
        .iter()
        .find(|v| rel(&doc, v.position) == (18.0, 15.0))
        .unwrap();
    assert_eq!(doc.net_name(v.common.net), Some("Net1"));
    assert_eq!((mm(v.diameter), mm(v.hole)), (0.4, 0.2));
    assert_eq!((v.start_layer, v.end_layer), (Layer::TOP, Layer::BOTTOM));

    // Arc,,,Top Layer,No Net,,8,16 radius 0.85, 0..360, width 0.05, keepout
    let a = &p.arcs[0];
    assert_eq!(rel(&doc, a.center), (8.0, 16.0));
    assert_eq!(
        (mm(a.radius), a.start, a.end, mm(a.width)),
        (0.85, 0.0, 360.0, 0.05)
    );
    assert!(a.common.keepout);

    // Pad,,Top Layer,,Net1,,8,11 ... Rounded Rectangle 1.85 x 1.85, 50%
    let pad = p
        .pads
        .iter()
        .find(|x| rel(&doc, x.position) == (8.0, 11.0))
        .unwrap();
    assert_eq!(pad.name, "1");
    assert_eq!(doc.net_name(pad.common.net), Some("Net1"));
    assert_eq!((mm(pad.size_top.x), mm(pad.size_top.y)), (1.85, 1.85));
    assert_eq!(pad.shape_top, PadShape::RoundedRect);
    assert_eq!(pad.corner_radius_pct, 50);
    assert_eq!(pad.hole, 0.0);
    assert_eq!(pad.hole_shape, HoleShape::Round);

    // Pad 2 of the capacitor at (8.725, 21), rotated 90, 0.84 x 0.75, 25%.
    let c2 = p
        .pads
        .iter()
        .find(|x| rel(&doc, x.position) == (8.725, 21.0))
        .unwrap();
    assert_eq!(c2.name, "2");
    assert_eq!(c2.rotation, 90.0);
    assert_eq!(c2.corner_radius_pct, 25);
    assert_eq!((mm(c2.size_top.x), mm(c2.size_top.y)), (0.84, 0.75));

    // Component,,Top Layer,,-,Free,8,21 ... CAPC1608X09L
    let cap = doc
        .components
        .iter()
        .find(|c| c.pattern == "CAPC1608X09L")
        .unwrap();
    assert_eq!(rel(&doc, cap.position), (8.0, 21.0));
    assert_eq!(cap.layer, Layer::TOP);

    // Text,Designator,TopOverlay ... 7.0822,12.4416 height 1.524 stroke 0.254
    let d = p.texts.iter().find(|x| x.is_designator).unwrap();
    assert_eq!(rel(&doc, d.position), (7.082, 12.442));
    assert_eq!((mm(d.height), mm(d.stroke_width)), (1.524, 0.254));

    // Board outline: 60 x 40 mm rectangle from the origin.
    let o: Vec<_> = doc.board.outline.iter().map(|v| rel(&doc, v.at)).collect();
    assert_eq!(
        o,
        [
            (0.0, 0.0),
            (0.0, 40.0),
            (60.0, 40.0),
            (60.0, 0.0),
            (0.0, 0.0)
        ]
    );
    assert_eq!(doc.board.copper_stack.first().unwrap().0, Layer::TOP);
    assert_eq!(doc.board.copper_stack.last().unwrap().0, Layer::BOTTOM);
    assert_eq!(doc.polygons.len(), 1);
    assert!(doc.polygons[0].vertices.len() >= 3);
}

#[test]
fn pcblib_lists_footprints_with_pads() {
    let lib = PcbLib::open(sample("Simple.PcbLib")).unwrap();
    let four = lib
        .footprints
        .iter()
        .find(|f| f.name == "Four pads")
        .unwrap();
    assert_eq!(four.primitives.pads.len(), 4);
    let cap = lib
        .footprints
        .iter()
        .find(|f| f.name == "CAPC1608X09L")
        .unwrap();
    assert_eq!(cap.primitives.pads.len(), 2);
    assert!(!cap.primitives.tracks.is_empty());
}

/// The library lists 13 footprints (the directory in `Library/Data`); the
/// root storage `FileVersionInfo` also has a `Data` stream but is not one.
#[test]
fn pcblib_has_exactly_the_listed_footprints() {
    let lib = PcbLib::open(sample("Simple.PcbLib")).unwrap();
    let mut names: Vec<&str> = lib.footprints.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "CAPC1608X09L",
            "Footprint 22",
            "Four pads",
            "Lines shifted",
            "Pads Combined",
            "SQFP50P800X800X300_HS-33T300X300",
            "Single Pad (0,0)",
            "Single Pad (0,0) Rotate 180",
            "Single Pad (0,0) Rotate 270",
            "Single Pad (0,0) Rotate 90",
            "Single Pad (0,1)",
            "Single Pad (1,0)",
            "Strings on Layers",
        ]
    );
    assert_eq!(lib.skipped.len(), 1);
    assert!(lib.skipped[0].version_info);
}

/// A version record of 125 bytes opens with `|` (124), so its block looks
/// like one length-prefixed string; the directory still rules it out.
#[test]
fn a_version_record_shaped_like_a_name_is_skipped_and_listed() {
    use std::io::{Cursor, Write};
    let bytes = std::fs::read(sample("Simple.PcbLib")).unwrap();
    let mut cf = cfb::CompoundFile::open(Cursor::new(bytes)).unwrap();
    let mut text = b"|COUNT=1|VER0=1".to_vec();
    text.resize(125, b'0');
    let mut data = 125u32.to_le_bytes().to_vec();
    data.extend_from_slice(&text);
    // Replaces the existing stream.
    cf.create_stream("/FileVersionInfo/Data")
        .unwrap()
        .write_all(&data)
        .unwrap();
    cf.flush().unwrap();
    let mut c =
        altium_format::Container::from_bytes("x".into(), cf.into_inner().into_inner()).unwrap();
    let lib = PcbLib::from_container(&mut c).unwrap();
    assert_eq!(lib.footprints.len(), 13);
    assert!(lib.footprints.iter().all(|f| !f.name.starts_with("COUNT=")));
    assert_eq!(lib.skipped.len(), 1);
    assert!(lib.skipped[0].version_info);
    assert_eq!(lib.skipped[0].data_len, 129);
}

#[test]
fn prjpcb_lists_documents() {
    let prj = Project::open(sample("simple-proj.prjpcb")).unwrap();
    assert!(prj.documents.iter().any(|d| d.ends_with(".SchDoc")));
}
