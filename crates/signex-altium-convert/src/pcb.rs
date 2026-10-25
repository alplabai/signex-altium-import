//! `PcbDoc` -> `PcbBoard`.
//!
//! Frames: Altium boards are mils, Y up, absolute. Signex boards are mm,
//! Y down; the outline's top-left lands on (0, 0). Footprint pads and
//! graphics are footprint-local, rotated by the footprint's rotation.

use std::collections::HashMap;

use altium_format::pcb::{self as apcb, Layer, PadShape as APadShape, PcbDoc, Vertex};
use altium_format::sch::Pt;
use signex_types::pcb::{
    BoardGraphic, BoardText, DrillDef, Footprint, FpGraphic, LayerDef, NetDef, Pad, PadNet,
    PadShape, PadType, PcbBoard, PcbSetup, Point, Segment, Via, ViaType, Zone,
};
use signex_types::property::PcbProperty;
use uuid::Uuid;

use crate::report::Report;
use crate::units::{deg, mm};

/// Arcs flattened to chords of at most this many degrees.
const ARC_STEP_DEG: f64 = 10.0;

pub fn convert(doc: &PcbDoc, report: &mut Report) -> PcbBoard {
    let frame = Frame::for_doc(doc);
    let names = LayerNames::new(doc);
    let mut board = PcbBoard {
        uuid: Uuid::new_v4(),
        version: 1,
        generator: "signex-altium-import".into(),
        thickness: signex_types::pcb::PCB_DEFAULT_THICKNESS_MM,
        outline: outline(&doc.board.outline)
            .into_iter()
            .map(|p| frame.world(p))
            .collect(),
        layers: doc
            .board
            .copper_stack
            .iter()
            .enumerate()
            .map(|(i, (l, _))| LayerDef {
                id: i as u8,
                name: names.of(*l),
                layer_type: "signal".into(),
            })
            .collect(),
        setup: Some(PcbSetup {
            grid_size: signex_types::pcb::PCB_GRID_MM,
            trace_width: signex_types::pcb::PCB_DEFAULT_TRACE_WIDTH_MM,
            via_diameter: signex_types::pcb::PCB_DEFAULT_VIA_DIAMETER_MM,
            via_drill: signex_types::pcb::PCB_DEFAULT_VIA_DRILL_MM,
            clearance: signex_types::pcb::PCB_DEFAULT_CLEARANCE_MM,
            track_min_width: signex_types::pcb::PCB_TRACK_MIN_MM,
            via_min_diameter: signex_types::pcb::PCB_VIA_MIN_DIAMETER_MM,
            via_min_drill: signex_types::pcb::PCB_VIA_MIN_DRILL_MM,
        }),
        nets: doc
            .nets
            .iter()
            .enumerate()
            .map(|(i, n)| NetDef {
                number: i as u32 + 1,
                name: n.clone(),
            })
            .collect(),
        footprints: Vec::new(),
        segments: Vec::new(),
        vias: Vec::new(),
        zones: Vec::new(),
        graphics: Vec::new(),
        texts: Vec::new(),
        unknown: Default::default(),
    };

    // Footprints first, so owned primitives can find their parent. Slot i
    // of `board.footprints` is Altium component i.
    for (i, c) in doc.components.iter().enumerate() {
        board.footprints.push(footprint(i as u16, c, doc, &frame));
    }
    let owner = |c: Option<u16>| c.map(usize::from).filter(|&i| i < doc.components.len());
    let comp_rotation = |slot: usize| doc.components[slot].rotation;

    let p = &doc.primitives;
    for pad in &p.pads {
        let net = pad_net(doc, pad.common.net);
        match owner(pad.common.component) {
            Some(slot) => {
                let fp = &board.footprints[slot];
                let pad = convert_pad(pad, fp, comp_rotation(slot), &frame, &names, net, report);
                board.footprints[slot].pads.push(pad);
            }
            None => {
                report.warn("free pad (not in a component) imported into its own footprint");
                let mut fp = free_pad_holder(pad, &frame);
                let pad = convert_pad(pad, &fp, 0.0, &frame, &names, net, report);
                fp.pads.push(pad);
                board.footprints.push(fp);
            }
        }
    }

    for t in &p.tracks {
        let layer = names.of(t.common.layer);
        if t.common.layer.is_copper() && !t.common.keepout {
            board
                .segments
                .push(segment(&frame, t.a, t.b, t.width, layer, t.common.net));
        } else if let Some(slot) = owner(t.common.component) {
            let fp = &mut board.footprints[slot];
            let (a, b) = (
                fp_local(fp, frame.world(t.a)),
                fp_local(fp, frame.world(t.b)),
            );
            fp.graphics.push(FpGraphic {
                graphic_type: "line".into(),
                layer,
                width: mm(t.width),
                start: Some(a),
                end: Some(b),
                ..fp_graphic_defaults()
            });
        } else {
            board.graphics.push(BoardGraphic {
                graphic_type: "line".into(),
                layer: keepout_layer(&t.common, layer),
                width: mm(t.width),
                start: Some(frame.world(t.a)),
                end: Some(frame.world(t.b)),
                center: None,
                radius: 0.0,
                points: Vec::new(),
            });
        }
    }

    for a in &p.arcs {
        let layer = names.of(a.common.layer);
        if a.common.layer.is_copper() && !a.common.keepout {
            report.warn("copper arc imported as straight track segments");
            let pts = arc_polyline(a.center, a.radius, a.start, a.end);
            for w in pts.windows(2) {
                board.segments.push(segment(
                    &frame,
                    w[0],
                    w[1],
                    a.width,
                    layer.clone(),
                    a.common.net,
                ));
            }
            continue;
        }
        let full = is_full(a.start, a.end);
        let (s, m, e) = arc_points(a.center, a.radius, a.start, a.end);
        let kind = if full { "circle" } else { "arc" };
        if let Some(slot) = owner(a.common.component) {
            let fp = &mut board.footprints[slot];
            let l = |p: Pt| fp_local(fp, frame.world(p));
            let g = FpGraphic {
                graphic_type: kind.into(),
                layer,
                width: mm(a.width),
                start: Some(l(s)),
                end: (!full).then(|| l(e)),
                center: Some(l(a.center)),
                mid: (!full).then(|| l(m)),
                radius: mm(a.radius),
                ..fp_graphic_defaults()
            };
            fp.graphics.push(g);
        } else {
            board.graphics.push(BoardGraphic {
                graphic_type: kind.into(),
                layer: keepout_layer(&a.common, layer),
                width: mm(a.width),
                start: Some(frame.world(s)),
                end: (!full).then(|| frame.world(e)),
                center: Some(frame.world(a.center)),
                radius: mm(a.radius),
                points: Vec::new(),
            });
        }
    }

    for v in &p.vias {
        let through = v.start_layer == Layer::TOP && v.end_layer == Layer::BOTTOM;
        board.vias.push(Via {
            uuid: Uuid::new_v4(),
            position: frame.world(v.position),
            diameter: mm(v.diameter),
            drill: mm(v.hole),
            layers: vec![names.of(v.start_layer), names.of(v.end_layer)],
            net: net_number(v.common.net),
            via_type: if through {
                ViaType::Through
            } else {
                ViaType::Blind
            },
            unknown_columns: Default::default(),
        });
    }

    for t in &p.texts {
        match owner(t.common.component) {
            // The designator and comment strings are the footprint's own
            // reference and value.
            Some(_) if t.is_designator || t.is_comment => {}
            Some(slot) => {
                let fp = &mut board.footprints[slot];
                let pos = fp_local(fp, frame.world(t.position));
                fp.graphics.push(FpGraphic {
                    graphic_type: "text".into(),
                    layer: names.of(t.common.layer),
                    width: mm(t.stroke_width),
                    text: t.text.clone(),
                    font_size: mm(t.height),
                    position: Some(pos),
                    rotation: deg(t.rotation - comp_rotation(slot)),
                    ..fp_graphic_defaults()
                });
            }
            None if !t.text.is_empty() => board.texts.push(BoardText {
                uuid: Uuid::new_v4(),
                text: t.text.clone(),
                position: frame.world(t.position),
                layer: names.of(t.common.layer),
                font_size: mm(t.height),
                rotation: deg(t.rotation),
            }),
            None => {}
        }
    }

    for f in &p.fills {
        let pts: Vec<Point> = rect_corners(f.a, f.b, f.rotation)
            .into_iter()
            .map(|p| frame.world(p))
            .collect();
        if f.common.layer.is_copper() && !f.common.keepout {
            board
                .zones
                .push(zone(doc, f.common.net, names.of(f.common.layer), pts));
        } else {
            board.graphics.push(BoardGraphic {
                graphic_type: "polygon".into(),
                layer: keepout_layer(&f.common, names.of(f.common.layer)),
                width: 0.0,
                start: None,
                end: None,
                center: None,
                radius: 0.0,
                points: pts,
            });
        }
    }

    for poly in &doc.polygons {
        let pts: Vec<Point> = outline(&poly.vertices)
            .into_iter()
            .map(|p| frame.world(p))
            .collect();
        if pts.len() < 3 {
            continue;
        }
        let hatch = poly.params.str("HATCHSTYLE");
        if !hatch.is_empty() && !hatch.eq_ignore_ascii_case("Solid") {
            report.warn("hatched polygon pour imported as a solid zone");
        }
        let mut z = zone(doc, poly.net, names.of(poly.layer), pts);
        z.priority = poly.params.i64("POURINDEX").unwrap_or(0).max(0) as u32;
        board.zones.push(z);
    }

    for r in &p.regions {
        // Poured copper belongs to its polygon (imported above), and
        // footprint regions are custom pad shapes: only free regions are
        // design data of their own.
        if r.common.polygon.is_some() || r.common.component.is_some() {
            continue;
        }
        let pts: Vec<Point> = r.outline.iter().map(|p| frame.world(*p)).collect();
        if pts.len() < 3 {
            continue;
        }
        if r.common.layer.is_copper() && r.kind == 0 && !r.params.bool("ISBOARDCUTOUT") {
            board
                .zones
                .push(zone(doc, r.common.net, names.of(r.common.layer), pts));
        } else {
            report.warn("cutout or non-copper region not imported");
        }
    }

    for (ty, n) in &p.skipped {
        let what = match ty {
            12 => "3D component body".to_string(),
            t => format!("PCB primitive type {t}"),
        };
        for _ in 0..*n {
            report.warn(format!("{what} not imported"));
        }
    }
    board
}

/// Signex numbers nets from 1; 0 is "no net".
fn net_number(n: Option<u16>) -> u32 {
    n.map_or(0, |n| u32::from(n) + 1)
}

fn pad_net(doc: &PcbDoc, n: Option<u16>) -> Option<PadNet> {
    let i = n?;
    let name = doc.nets.get(usize::from(i))?;
    Some(PadNet {
        number: u32::from(i) + 1,
        name: name.clone(),
    })
}

fn segment(frame: &Frame, a: Pt, b: Pt, width: f64, layer: String, net: Option<u16>) -> Segment {
    Segment {
        uuid: Uuid::new_v4(),
        start: frame.world(a),
        end: frame.world(b),
        width: mm(width),
        layer,
        net: net_number(net),
        unknown_columns: Default::default(),
    }
}

/// Board space -> Signex: shift so the outline's top-left is (0, 0), flip Y.
struct Frame {
    min_x: f64,
    max_y: f64,
}

impl Frame {
    fn for_doc(doc: &PcbDoc) -> Self {
        let mut pts: Vec<Pt> = outline(&doc.board.outline);
        if pts.is_empty() {
            let p = &doc.primitives;
            pts.extend(p.tracks.iter().flat_map(|t| [t.a, t.b]));
            pts.extend(p.pads.iter().map(|x| x.position));
            pts.extend(p.vias.iter().map(|x| x.position));
            pts.extend(doc.components.iter().map(|c| c.position));
        }
        if pts.is_empty() {
            pts.push(doc.board.origin);
        }
        Self {
            min_x: pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            max_y: pts.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
        }
    }

    fn world(&self, p: Pt) -> Point {
        Point::new(mm(p.x - self.min_x), mm(self.max_y - p.y))
    }
}

/// Altium layer -> the layer name `.snxpcb` files use.
struct LayerNames {
    mech_names: HashMap<u8, String>,
}

impl LayerNames {
    fn new(doc: &PcbDoc) -> Self {
        let mech_names = (1..=16u8)
            .filter_map(|n| {
                let name = doc.board.params.string(&format!("LAYER{}NAME", n + 56));
                (!name.is_empty()).then_some((n, name))
            })
            .collect();
        Self { mech_names }
    }

    fn of(&self, l: Layer) -> String {
        match l {
            Layer::TOP => "TopCopper".into(),
            Layer::BOTTOM => "BottomCopper".into(),
            Layer::TOP_OVERLAY => "TopSilkscreen".into(),
            Layer::BOTTOM_OVERLAY => "BottomSilkscreen".into(),
            Layer::TOP_SOLDER => "TopSolderMask".into(),
            Layer::BOTTOM_SOLDER => "BottomSolderMask".into(),
            Layer::TOP_PASTE => "TopPaste".into(),
            Layer::BOTTOM_PASTE => "BottomPaste".into(),
            Layer::KEEP_OUT => "KeepOut".into(),
            Layer::MULTI_LAYER => "TopCopper".into(),
            _ => {
                if let Some(n) = l.mid() {
                    format!("Inner{n}Copper")
                } else if let Some(n) = l.plane() {
                    // Planes follow the 30 signal mid layers in Signex's
                    // single inner numbering.
                    format!("Inner{}Copper", 30 + u32::from(n))
                } else if let Some(n) = l.mechanical() {
                    self.mechanical(n)
                } else {
                    format!("Altium{}", l.0)
                }
            }
        }
    }

    /// A mechanical layer's role lives only in its user-facing name.
    fn mechanical(&self, n: u8) -> String {
        let name = self
            .mech_names
            .get(&n)
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let bottom = name.contains("bottom");
        if name.contains("courtyard") {
            if bottom {
                "BottomCourtyard"
            } else {
                "TopCourtyard"
            }
            .into()
        } else if name.contains("assembly") {
            if bottom { "BottomFab" } else { "TopFab" }.into()
        } else if name.contains("outline") || name.contains("board shape") {
            "EdgeCuts".into()
        } else {
            format!("Mechanical{n}")
        }
    }
}

fn keepout_layer(c: &apcb::Common, layer: String) -> String {
    if c.keepout { "KeepOut".into() } else { layer }
}

/// Text-record outline -> points, with arc edges flattened.
fn outline(vertices: &[Vertex]) -> Vec<Pt> {
    let mut out = Vec::new();
    for v in vertices {
        match v.arc {
            Some(a) if a.radius > 0.0 => {
                // Walk the arc from whichever end the vertex sits on.
                let mut pts = arc_polyline(a.center, a.radius, a.start, a.end);
                let d = |p: &Pt| (p.x - v.at.x).hypot(p.y - v.at.y);
                if let (Some(first), Some(last)) = (pts.first(), pts.last())
                    && d(last) < d(first)
                {
                    pts.reverse();
                }
                out.extend(pts);
            }
            _ => out.push(v.at),
        }
    }
    out.dedup_by(|a, b| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6);
    out
}

fn is_full(start: f64, end: f64) -> bool {
    let s = (end - start).rem_euclid(360.0);
    s < 1e-6 || (s - 360.0).abs() < 1e-6
}

fn sweep(start: f64, end: f64) -> f64 {
    let s = (end - start).rem_euclid(360.0);
    if s < 1e-9 { 360.0 } else { s }
}

fn on_arc(c: Pt, r: f64, a: f64) -> Pt {
    let t = a.to_radians();
    Pt::new(c.x + r * t.cos(), c.y + r * t.sin())
}

fn arc_points(c: Pt, r: f64, start: f64, end: f64) -> (Pt, Pt, Pt) {
    let s = sweep(start, end);
    (
        on_arc(c, r, start),
        on_arc(c, r, start + s / 2.0),
        on_arc(c, r, start + s),
    )
}

fn arc_polyline(c: Pt, r: f64, start: f64, end: f64) -> Vec<Pt> {
    let s = sweep(start, end);
    let n = (s / ARC_STEP_DEG).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| on_arc(c, r, start + s * i as f64 / n as f64))
        .collect()
}

fn rect_corners(a: Pt, b: Pt, rotation: f64) -> Vec<Pt> {
    let c = Pt::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let (hw, hh) = ((b.x - a.x).abs() / 2.0, (b.y - a.y).abs() / 2.0);
    let (s, co) = rotation.to_radians().sin_cos();
    [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
        .iter()
        .map(|(x, y)| Pt::new(c.x + x * co - y * s, c.y + x * s + y * co))
        .collect()
}

fn zone(doc: &PcbDoc, net: Option<u16>, layer: String, outline: Vec<Point>) -> Zone {
    let (net, net_name) = pad_net(doc, net)
        .map(|n| (n.number, n.name))
        .unwrap_or_default();
    Zone {
        uuid: Uuid::new_v4(),
        net,
        net_name,
        layer,
        outline,
        holes: Vec::new(),
        priority: 0,
        fill_type: "solid".into(),
        thermal_relief: true,
        thermal_gap: signex_types::pcb::PCB_THERMAL_GAP_MM,
        thermal_width: signex_types::pcb::PCB_THERMAL_BRIDGE_MM,
        clearance: signex_types::pcb::PCB_DEFAULT_CLEARANCE_MM,
        min_thickness: signex_types::pcb::PCB_ZONE_MIN_THICKNESS_MM,
        unknown: Default::default(),
    }
}

/// Rotate `v` by `deg` degrees with the plain matrix, as the renderer does.
fn rotate(v: Point, deg: f64) -> Point {
    let (s, c) = deg.to_radians().sin_cos();
    Point::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// World -> footprint-local, inverting `world = origin + R(rotation) * local`.
fn fp_local(fp: &Footprint, world: Point) -> Point {
    let d = Point::new(world.x - fp.position.x, world.y - fp.position.y);
    let l = rotate(d, -fp.rotation);
    Point::new((l.x * 1e6).round() / 1e6, (l.y * 1e6).round() / 1e6)
}

fn fp_graphic_defaults() -> FpGraphic {
    FpGraphic {
        graphic_type: String::new(),
        layer: String::new(),
        width: 0.0,
        start: None,
        end: None,
        center: None,
        mid: None,
        radius: 0.0,
        points: Vec::new(),
        text: String::new(),
        font_size: 0.0,
        position: None,
        rotation: 0.0,
        fill: String::new(),
    }
}

fn property(key: &str, value: &str) -> PcbProperty {
    PcbProperty {
        key: key.into(),
        value: value.into(),
        hidden: true,
        ..Default::default()
    }
}

fn footprint(index: u16, c: &apcb::Component, doc: &PcbDoc, frame: &Frame) -> Footprint {
    let texts: Vec<&apcb::Text> = doc
        .primitives
        .texts
        .iter()
        .filter(|t| t.common.component == Some(index))
        .collect();
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    let designator = non_empty(c.params.string("SOURCEDESIGNATOR"))
        .or_else(|| {
            texts
                .iter()
                .find(|t| t.is_designator)
                .and_then(|t| non_empty(t.text.clone()))
        })
        .unwrap_or_default();
    let comment = texts
        .iter()
        .find(|t| t.is_comment)
        .and_then(|t| non_empty(t.text.clone()))
        .or_else(|| non_empty(c.params.string("COMMENT")))
        .unwrap_or_default();
    let properties = [
        ("Description", c.description.as_str()),
        ("Source Library", c.source_library.as_str()),
        ("Altium Unique ID", c.unique_id.as_str()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| property(k, v))
    .collect();
    Footprint {
        uuid: Uuid::new_v4(),
        reference: designator,
        value: comment,
        footprint_id: c.pattern.clone(),
        position: frame.world(c.position),
        // Altium turns counter-clockwise with Y up; on the Y-down board the
        // same visual turn is negative under the renderer's rotation matrix.
        rotation: deg(-c.rotation),
        layer: if c.layer == Layer::BOTTOM {
            "BottomCopper"
        } else {
            "TopCopper"
        }
        .into(),
        locked: c.locked,
        pads: Vec::new(),
        graphics: Vec::new(),
        properties,
        unknown: Default::default(),
        unknown_columns: Default::default(),
    }
}

fn free_pad_holder(pad: &apcb::Pad, frame: &Frame) -> Footprint {
    Footprint {
        uuid: Uuid::new_v4(),
        reference: String::new(),
        value: String::new(),
        footprint_id: "altium:free-pad".into(),
        position: frame.world(pad.position),
        rotation: 0.0,
        layer: "TopCopper".into(),
        locked: false,
        pads: Vec::new(),
        graphics: Vec::new(),
        properties: Vec::new(),
        unknown: Default::default(),
        unknown_columns: Default::default(),
    }
}

fn convert_pad(
    pad: &apcb::Pad,
    fp: &Footprint,
    comp_rotation: f64,
    frame: &Frame,
    names: &LayerNames,
    net: Option<PadNet>,
    report: &mut Report,
) -> Pad {
    let drilled = pad.hole > 0.0;
    let pad_type = match (drilled, pad.plated) {
        (true, true) => PadType::Thru,
        (true, false) => PadType::NpThru,
        _ => PadType::Smd,
    };
    let bottom = pad.common.layer == Layer::BOTTOM;
    let (size, shape) = if bottom {
        (pad.size_bottom, pad.shape_bottom)
    } else {
        (pad.size_top, pad.shape_top)
    };

    // A Signex pad takes its footprint's rotation; a pad turned against
    // its footprint survives only as a quarter turn (width/height swap).
    let rel = deg(pad.rotation - comp_rotation) % 180.0;
    let swap = (rel - 90.0).abs() < 0.01;
    if !swap && rel > 0.01 && rel < 179.99 {
        report.warn("pad rotated off a quarter turn from its footprint imported unrotated");
    }
    let (sx, sy) = if swap {
        (size.y, size.x)
    } else {
        (size.x, size.y)
    };
    let shape = match shape {
        APadShape::Round if (sx - sy).abs() < 1e-6 => PadShape::Circle,
        APadShape::Round => PadShape::Oval,
        APadShape::Rect => PadShape::Rect,
        APadShape::RoundedRect => PadShape::RoundRect,
        APadShape::Octagonal => {
            report.warn("octagonal pad imported as a rectangle");
            PadShape::Rect
        }
    };
    let layers: Vec<String> = match pad_type {
        PadType::Thru | PadType::NpThru => vec!["TopCopper".into(), "BottomCopper".into()],
        _ => {
            let (paste, mask) = if bottom {
                ("BottomPaste", "BottomSolderMask")
            } else {
                ("TopPaste", "TopSolderMask")
            };
            vec![names.of(pad.common.layer), paste.into(), mask.into()]
        }
    };
    let drill = drilled.then(|| DrillDef {
        diameter: mm(pad.hole),
        shape: match pad.hole_shape {
            apcb::HoleShape::Slot => "oval".into(),
            apcb::HoleShape::Square => "square".into(),
            apcb::HoleShape::Round => String::new(),
        },
    });
    Pad {
        uuid: Uuid::new_v4(),
        number: pad.name.clone(),
        pad_type,
        shape,
        position: fp_local(fp, frame.world(pad.position)),
        size: Point::new(mm(sx), mm(sy)),
        drill,
        layers,
        net,
        // Altium: radius as a percentage of half the short side.
        // Signex: radius as a fraction of the short side.
        roundrect_ratio: if shape == PadShape::RoundRect {
            f64::from(pad.corner_radius_pct.min(100)) / 200.0
        } else {
            0.0
        },
        unknown: Default::default(),
        unknown_columns: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fp_local_inverts_the_renderer_rotation() {
        let mut fp = free_pad_holder(
            &apcb::Pad::default(),
            &Frame {
                min_x: 0.0,
                max_y: 0.0,
            },
        );
        fp.position = Point::new(10.0, 20.0);
        fp.rotation = 90.0;
        let local = Point::new(1.0, 0.0);
        let r = rotate(local, fp.rotation);
        let world = Point::new(fp.position.x + r.x, fp.position.y + r.y);
        let back = fp_local(&fp, world);
        assert!((back.x - 1.0).abs() < 1e-9 && back.y.abs() < 1e-9);
    }

    #[test]
    fn arc_polyline_covers_the_sweep() {
        let pts = arc_polyline(Pt::new(0.0, 0.0), 10.0, 350.0, 10.0);
        assert_eq!(pts.len(), 3);
        assert!((pts[1].x - 10.0).abs() < 1e-9 && pts[1].y.abs() < 1e-9);
    }
}
