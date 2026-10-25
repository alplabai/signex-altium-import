//! `SchLib` -> `.snxsym`, `PcbLib` -> `.snxfpt`.
//!
//! Frames: Altium library items are local to their origin. Symbols stay
//! Y up (as Signex symbol libraries are); footprints flip to Y down, the
//! footprint editor's frame, so rotations negate. See `docs/CONVERSION.md`.

use altium_format::pcb::{self as apcb, Layer, PadShape as APadShape, PcbLib};
use altium_format::sch::{Component, Electrical, Pin as APin, PinSymbols, Pt, SchLib, Shape};
use signex_library::param::{ParamMap, ParamValue};
use signex_library::primitive::Footprint;
use signex_library::primitive::footprint::{
    Drill, FootprintFile, FpGraphic, FpGraphicKind, LayerId, Pad, PadKind, PadShape, Polygon,
};
use signex_library::primitive::symbol::{
    PinDirection, PinOrientation, PinSymbolKind, Symbol, SymbolFile, SymbolGraphic,
    SymbolGraphicKind, SymbolPin,
};

use crate::pcb::{LayerNames, arc_polyline, is_full};
use crate::report::Report;
use crate::units::{MM_PER_PT, deg, mm};

/// Fill colour for solid symbol bodies (Signex's default body fill).
const BODY_FILL: [u8; 4] = [255, 255, 176, 255];

/// Every symbol of a schematic library, in library order, in one file.
pub fn symbol_file(lib: &SchLib, name: &str, report: &mut Report) -> SymbolFile {
    let mut symbols = lib.components.iter().map(|c| symbol(c, report));
    let first = symbols
        .next()
        .unwrap_or_else(|| Symbol::empty(name.to_string()));
    let mut file = SymbolFile::from_symbol(first);
    file.symbols.extend(symbols);
    file.display_name = name.to_string();
    file
}

fn symbol(c: &Component, report: &mut Report) -> Symbol {
    let mut s = Symbol::empty(c.lib_reference.clone());
    s.description = c.description.clone();
    s.part_count = u8::try_from(c.part_count).unwrap_or(u8::MAX).max(1);
    if let Some(d) = &c.designator {
        if !d.text.is_empty() {
            s.designator = d.text.clone();
        }
        s.designator_pos = Some(xy(d.location));
    }
    if let Some(p) = c
        .parameters
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("Comment"))
    {
        if !p.text.text.is_empty() {
            s.comment = p.text.text.clone();
        }
        s.comment_pos = Some(xy(p.text.location));
    }
    let mut params = ParamMap::new();
    for p in &c.parameters {
        if p.name.eq_ignore_ascii_case("Comment") || p.text.text.is_empty() || p.text.text == "*" {
            continue;
        }
        params.insert(p.name.clone(), ParamValue::Text(p.text.text.clone()));
    }
    if let Some(fp) = c.footprint() {
        params.insert("Footprint".into(), ParamValue::Text(fp.to_string()));
    }
    s.schematic_params = params;

    let in_mode = |m: i32| m == c.display_mode;
    s.pins = c
        .pins
        .iter()
        .filter(|p| in_mode(p.display_mode))
        .map(|p| pin(p, report))
        .collect();
    for (part, mode, shape) in &c.shapes {
        if in_mode(*mode) {
            s.graphics.extend(graphics(shape, part_of(*part), report));
        }
    }
    s
}

/// Altium part -1 / 0 (every part) -> Signex unit 0 (shared).
fn part_of(part: i32) -> u8 {
    u8::try_from(part).ok().filter(|&p| p > 0).unwrap_or(0)
}

fn xy(p: Pt) -> [f64; 2] {
    [mm(p.x), mm(p.y)]
}

fn pin(p: &APin, report: &mut Report) -> SymbolPin {
    let mut pin = SymbolPin::new(p.designator.clone(), p.name.clone());
    pin.electrical = match p.electrical {
        Electrical::Input => PinDirection::Input,
        Electrical::InputOutput => PinDirection::Bidirectional,
        Electrical::Output => PinDirection::Output,
        Electrical::OpenCollector => PinDirection::OpenCollector,
        Electrical::Passive => PinDirection::Passive,
        Electrical::HiZ => PinDirection::Tristate,
        Electrical::OpenEmitter => PinDirection::OpenEmitter,
        Electrical::Power => PinDirection::Power,
    };
    // Signex: position at the electrical end, orientation toward the body.
    pin.position = xy(p.hot_end());
    pin.orientation = match (p.rotation + 2) & 3 {
        0 => PinOrientation::Right,
        1 => PinOrientation::Up,
        2 => PinOrientation::Left,
        _ => PinOrientation::Down,
    };
    pin.length = mm(p.length);
    pin.description = p.description.clone();
    pin.hidden = p.hidden;
    pin.name_visible = p.show_name;
    pin.designator_visible = p.show_designator;
    pin.part_number = part_of(p.owner_part_id);
    pin.inside_edge_symbol = edge_symbol(p.symbols.inner_edge, report);
    pin.outside_edge_symbol = edge_symbol(p.symbols.outer_edge, report);
    for code in [p.symbols.inside, p.symbols.outside] {
        if code != 0 {
            report.warn(format!("pin IEEE symbol code {code} not mapped"));
        }
    }
    pin
}

/// The edge codes the samples confirm; anything else is reported.
fn edge_symbol(code: u8, report: &mut Report) -> PinSymbolKind {
    match code {
        0 => PinSymbolKind::None,
        PinSymbols::DOT => PinSymbolKind::Dot,
        PinSymbols::CLOCK => PinSymbolKind::ClockEdge,
        other => {
            report.warn(format!("pin IEEE symbol code {other} not mapped"));
            PinSymbolKind::None
        }
    }
}

fn line(a: Pt, b: Pt) -> SymbolGraphicKind {
    SymbolGraphicKind::Line {
        from: xy(a),
        to: xy(b),
    }
}

fn graphics(shape: &Shape, part_number: u8, report: &mut Report) -> Vec<SymbolGraphic> {
    let g = |kind, width: f64, filled: bool| SymbolGraphic {
        kind,
        stroke_width: mm(width),
        fill: filled.then_some(BODY_FILL),
        part_number,
    };
    let lines = |pts: &[Pt], width: f64| -> Vec<SymbolGraphic> {
        pts.windows(2)
            .map(|w| g(line(w[0], w[1]), width, false))
            .collect()
    };
    match shape {
        Shape::Line { a, b, width } => vec![g(line(*a, *b), *width, false)],
        Shape::Polyline { points, width } => lines(points, *width),
        Shape::Bezier { points, width } => {
            report.warn("symbol bezier imported as its control polygon");
            lines(points, *width)
        }
        Shape::Polygon {
            points,
            width,
            filled,
        } => vec![g(
            SymbolGraphicKind::Polygon {
                vertices: points.iter().map(|p| xy(*p)).collect(),
            },
            *width,
            *filled,
        )],
        Shape::Rectangle {
            a,
            b,
            width,
            filled,
        } => vec![g(
            SymbolGraphicKind::Rectangle {
                from: xy(*a),
                to: xy(*b),
            },
            *width,
            *filled,
        )],
        Shape::RoundRect {
            a,
            b,
            width,
            filled,
            ..
        } => {
            report.warn("rounded rectangle imported with square corners");
            vec![g(
                SymbolGraphicKind::Rectangle {
                    from: xy(*a),
                    to: xy(*b),
                },
                *width,
                *filled,
            )]
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
            width,
        } => {
            let kind = if is_full(*start, *end) {
                SymbolGraphicKind::Circle {
                    center: xy(*center),
                    radius: mm(*radius),
                }
            } else {
                SymbolGraphicKind::Arc {
                    center: xy(*center),
                    radius: mm(*radius),
                    start_deg: deg(*start),
                    end_deg: deg(*end),
                }
            };
            vec![g(kind, *width, false)]
        }
        Shape::Ellipse {
            center,
            rx,
            ry,
            width,
            filled,
        } => {
            if (rx - ry).abs() < 1e-6 {
                vec![g(
                    SymbolGraphicKind::Circle {
                        center: xy(*center),
                        radius: mm(*rx),
                    },
                    *width,
                    *filled,
                )]
            } else {
                let vertices = (0..32)
                    .map(|i| {
                        let a = f64::from(i) / 32.0 * std::f64::consts::TAU;
                        xy(Pt::new(center.x + rx * a.cos(), center.y + ry * a.sin()))
                    })
                    .collect();
                vec![g(SymbolGraphicKind::Polygon { vertices }, *width, *filled)]
            }
        }
        Shape::Label(t) => vec![g(
            SymbolGraphicKind::Text {
                position: xy(t.location),
                content: t.text.clone(),
                size: 10.0 * MM_PER_PT,
            },
            0.0,
            false,
        )],
        Shape::TextFrame { a, b, text, .. } => vec![g(
            SymbolGraphicKind::Text {
                position: xy(Pt::new(a.x.min(b.x), a.y.max(b.y))),
                content: text.clone(),
                size: 10.0 * MM_PER_PT,
            },
            0.0,
            false,
        )],
    }
}

// ---------------------------------------------------------------------------
// Footprints
// ---------------------------------------------------------------------------

/// Every footprint of a PCB library, in one file.
pub fn footprint_file(lib: &PcbLib, name: &str, report: &mut Report) -> FootprintFile {
    let names = LayerNames::from_params(&lib.params, &lib.layer_kinds);
    let mut fps = lib.footprints.iter().map(|f| footprint(f, &names, report));
    let first = fps
        .next()
        .unwrap_or_else(|| Footprint::empty(name.to_string()));
    let mut file = FootprintFile::from_footprint(first);
    file.footprints.extend(fps);
    file.display_name = name.to_string();
    file
}

/// Footprint-local Altium (mils, Y up) -> editor mm, Y down.
fn fxy(p: Pt) -> [f64; 2] {
    [mm(p.x), mm(-p.y)]
}

/// Where a footprint graphic lands, by the layer's role.
enum Slot {
    SilkTop,
    SilkBottom,
    FabTop,
    FabBottom,
    Courtyard,
}

fn slot(layer: Layer, names: &LayerNames) -> Option<Slot> {
    match layer {
        Layer::TOP_OVERLAY => Some(Slot::SilkTop),
        Layer::BOTTOM_OVERLAY => Some(Slot::SilkBottom),
        _ => match names.of(layer).as_str() {
            "TopFab" => Some(Slot::FabTop),
            "BottomFab" => Some(Slot::FabBottom),
            "TopCourtyard" | "BottomCourtyard" => Some(Slot::Courtyard),
            _ => None,
        },
    }
}

fn footprint(f: &apcb::Footprint, names: &LayerNames, report: &mut Report) -> Footprint {
    let mut fp = Footprint::empty(f.name.clone());
    let p = &f.primitives;
    fp.pads = p.pads.iter().map(|x| pad(x, report)).collect();

    let mut courtyard: Vec<([f64; 2], [f64; 2])> = Vec::new();
    let push = |fp: &mut Footprint, layer: Layer, g: FpGraphic, report: &mut Report| {
        match slot(layer, names) {
            Some(Slot::SilkTop) => fp.silk_f.push(g),
            Some(Slot::SilkBottom) => fp.silk_b.push(g),
            Some(Slot::FabTop) => fp.fab_f.push(g),
            Some(Slot::FabBottom) => fp.fab_b.push(g),
            // Lines and arcs on a courtyard layer build the outline;
            // anything else there (text) has no place in it.
            Some(Slot::Courtyard) => report.warn("text on a courtyard layer not imported"),
            None => report.warn(format!(
                "footprint graphic on {} not imported",
                names.of(layer)
            )),
        }
    };

    for t in &p.tracks {
        if matches!(slot(t.common.layer, names), Some(Slot::Courtyard)) {
            courtyard.push((fxy(t.a), fxy(t.b)));
            continue;
        }
        if t.common.layer.is_copper() {
            report.warn("copper track inside a library footprint not imported");
            continue;
        }
        let g = FpGraphic {
            kind: FpGraphicKind::Line {
                from: fxy(t.a),
                to: fxy(t.b),
            },
            stroke_width: mm(t.width),
            filled: false,
        };
        push(&mut fp, t.common.layer, g, report);
    }
    for a in &p.arcs {
        if matches!(slot(a.common.layer, names), Some(Slot::Courtyard)) {
            let pts = arc_polyline(a.center, a.radius, a.start, a.end);
            courtyard.extend(pts.windows(2).map(|w| (fxy(w[0]), fxy(w[1]))));
            continue;
        }
        let kind = if is_full(a.start, a.end) {
            FpGraphicKind::Circle {
                center: fxy(a.center),
                radius: mm(a.radius),
            }
        } else {
            // The Y flip mirrors the sweep: CCW a..b becomes CCW -b..-a.
            FpGraphicKind::Arc {
                center: fxy(a.center),
                radius: mm(a.radius),
                start_deg: deg(-a.end),
                end_deg: deg(-a.start),
            }
        };
        let g = FpGraphic {
            kind,
            stroke_width: mm(a.width),
            filled: false,
        };
        push(&mut fp, a.common.layer, g, report);
    }
    for t in &p.texts {
        if t.is_designator || t.is_comment || t.text.is_empty() {
            continue;
        }
        let g = FpGraphic {
            kind: FpGraphicKind::Text {
                position: fxy(t.position),
                content: t.text.clone(),
                size: mm(t.height),
                frame: None,
            },
            stroke_width: mm(t.stroke_width),
            filled: false,
        };
        push(&mut fp, t.common.layer, g, report);
    }
    for _ in &p.fills {
        report.warn("fill inside a library footprint not imported");
    }
    for _ in &p.regions {
        report.warn("region inside a library footprint (custom pad shape) not imported");
    }
    if !courtyard.is_empty() {
        match chain(&courtyard) {
            Some(points) => fp.courtyard = Polygon::new(points),
            None => {
                report.warn("open courtyard outline replaced by its bounding box");
                fp.courtyard = Polygon::new(bounding_box(&courtyard));
            }
        }
    }
    fp
}

/// The largest closed loop the segments form, ignoring strays such as a
/// centre cross drawn on the courtyard layer.
fn chain(segs: &[([f64; 2], [f64; 2])]) -> Option<Vec<[f64; 2]>> {
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
    let mut best: Option<(f64, Vec<[f64; 2]>)> = None;
    for first in 0..segs.len() {
        let mut used = vec![false; segs.len()];
        used[first] = true;
        let (start, mut at) = segs[first];
        let mut points = vec![start];
        let closed = loop {
            if near(at, start) {
                break true;
            }
            let Some(i) =
                (0..segs.len()).find(|&i| !used[i] && (near(segs[i].0, at) || near(segs[i].1, at)))
            else {
                break false;
            };
            used[i] = true;
            points.push(at);
            at = if near(segs[i].0, at) {
                segs[i].1
            } else {
                segs[i].0
            };
        };
        if closed && points.len() >= 3 {
            let area = shoelace(&points).abs();
            if best.as_ref().is_none_or(|(a, _)| area > *a) {
                best = Some((area, points));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn shoelace(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
        .sum::<f64>()
        / 2.0
}

fn bounding_box(segs: &[([f64; 2], [f64; 2])]) -> Vec<[f64; 2]> {
    let pts = segs.iter().flat_map(|(a, b)| [*a, *b]);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for [x, y] in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

fn pad(p: &apcb::Pad, report: &mut Report) -> Pad {
    let drilled = p.hole > 0.0;
    let bottom = p.common.layer == Layer::BOTTOM;
    let kind = match (drilled, p.plated) {
        (true, true) => PadKind::Tht,
        (true, false) => PadKind::NptHole,
        _ => PadKind::Smd,
    };
    let (size, shape) = if bottom {
        (p.size_bottom, p.shape_bottom)
    } else {
        (p.size_top, p.shape_top)
    };
    let shape = match shape {
        APadShape::Round if (size.x - size.y).abs() < 1e-6 => PadShape::Round,
        APadShape::Round => PadShape::Oval,
        APadShape::Rect => PadShape::Rect,
        APadShape::RoundedRect => PadShape::RoundRect {
            radius_ratio: f64::from(p.corner_radius_pct.min(100)) / 200.0,
        },
        APadShape::Octagonal => {
            report.warn("octagonal pad imported as a rectangle");
            PadShape::Rect
        }
    };
    // The footprint editor's own layer names for new pads.
    let layers = match kind {
        PadKind::Tht | PadKind::NptHole => vec!["*.Cu", "F.Mask", "B.Mask"],
        _ if bottom => vec!["B.Cu", "B.Mask", "B.Paste"],
        _ => vec!["F.Cu", "F.Mask", "F.Paste"],
    };
    let drill = drilled.then(|| Drill {
        diameter: mm(p.hole),
        slot_length: (p.hole_shape == apcb::HoleShape::Slot && p.slot_length > p.hole)
            .then(|| mm(p.slot_length)),
    });
    Pad {
        number: p.name.clone(),
        kind,
        shape,
        size: [mm(size.x), mm(size.y)],
        position: fxy(p.position),
        rotation: deg(-p.rotation),
        layers: layers.into_iter().map(LayerId::new).collect(),
        drill,
        corner_radius_pct: (p.shape_top == APadShape::RoundedRect)
            .then(|| f64::from(p.corner_radius_pct)),
        // `Pad::default()` turns these off; the editor's new-pad default is on.
        paste_enabled_top: kind == PadKind::Smd && !bottom,
        paste_enabled_bottom: kind == PadKind::Smd && bottom,
        thermal_relief: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_closes_a_rectangle_in_any_segment_order() {
        let segs = [
            ([0.0, 0.0], [1.0, 0.0]),
            ([1.0, 1.0], [0.0, 1.0]),
            ([1.0, 0.0], [1.0, 1.0]),
            ([0.0, 0.0], [0.0, 1.0]),
        ];
        let pts = chain(&segs).unwrap();
        assert_eq!(pts.len(), 4);
    }

    #[test]
    fn chain_ignores_a_centre_cross() {
        let segs = [
            ([0.0, 0.0], [2.0, 0.0]),
            ([0.9, 1.0], [1.1, 1.0]),
            ([2.0, 0.0], [2.0, 2.0]),
            ([1.0, 0.9], [1.0, 1.1]),
            ([2.0, 2.0], [0.0, 2.0]),
            ([0.0, 2.0], [0.0, 0.0]),
        ];
        assert_eq!(chain(&segs).unwrap().len(), 4);
    }

    #[test]
    fn chain_rejects_an_open_outline() {
        assert!(chain(&[([0.0, 0.0], [1.0, 0.0]), ([1.0, 0.0], [1.0, 1.0])]).is_none());
    }
}
