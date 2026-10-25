//! `SchDoc` -> `SchematicSheet`.
//!
//! Frames: Altium sheets are mils, Y up, origin bottom-left. Signex sheets
//! are mm, Y down, origin top-left, so `world(p) = (mm(x), mm(H - y))`
//! with `H` the Altium sheet height. Library symbols stay Y up, in mm.

use std::collections::HashMap;

use altium_format::sch::{
    Component, Electrical, Justification, Pin as APin, PinSymbols, Port, PowerPort, Pt, SchDoc,
    Shape, SheetSymbol, Text,
};
use signex_types::schematic::{
    Bus, BusEntry, ChildSheet, FillType, Graphic, HAlign, Junction, Label, LabelType, LibGraphic,
    LibPin, LibSymbol, NoConnect, Pin, PinDirection, PinShapeStyle, Point, SchDrawing,
    SchematicSheet, SheetPin, Symbol, TextNote, TextProp, VAlign, Wire,
};
use uuid::Uuid;

use crate::report::Report;
use crate::units::{MM_PER_PT, deg, mm};

/// How Altium ports map onto Signex labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortScope {
    /// Ports link to sheet entries on the parent sheet.
    #[default]
    Hierarchical,
    /// Ports connect across every sheet of the project (flat designs).
    Global,
}

#[derive(Debug, Clone, Default)]
pub struct SchOptions {
    pub port_scope: PortScope,
    /// Child-sheet renames, keyed by lower-case Altium file name without
    /// its folder (`foo.schdoc` -> `Foo.snxsch`).
    pub sheet_files: HashMap<String, String>,
    /// Project-level parameters, used to resolve `=Name` special strings.
    pub project_parameters: Vec<(String, String)>,
}

pub fn convert(doc: &SchDoc, opts: &SchOptions, report: &mut Report) -> SchematicSheet {
    let size = doc.setup.size();
    let frame = Frame { height: size.y };
    let mut sheet = SchematicSheet {
        uuid: Uuid::new_v4(),
        version: 1,
        generator: "signex-altium-import".into(),
        generator_version: env!("CARGO_PKG_VERSION").into(),
        paper_size: paper_name(size).into(),
        ..Default::default()
    };
    let params = Params::new(doc, opts);
    sheet.title_block = title_block(&params);

    let mut libs = LibTable::default();
    for comp in &doc.components {
        let symbol = component(comp, &frame, &doc.setup, &params, &mut libs, report);
        sheet.symbols.push(symbol);
    }
    for pp in doc.power_ports.iter().filter(|p| !p.cross_sheet) {
        sheet.symbols.push(power_port(pp, &frame, &mut libs));
    }
    sheet.lib_symbols = libs.symbols;

    for run in &doc.wires {
        for seg in run.windows(2) {
            sheet.wires.push(Wire {
                uuid: Uuid::new_v4(),
                start: frame.world(seg[0]),
                end: frame.world(seg[1]),
                ..Default::default()
            });
        }
    }
    for run in &doc.buses {
        for seg in run.windows(2) {
            sheet.buses.push(Bus {
                uuid: Uuid::new_v4(),
                start: frame.world(seg[0]),
                end: frame.world(seg[1]),
            });
        }
    }
    for (a, b) in &doc.bus_entries {
        let (wa, wb) = (frame.world(*a), frame.world(*b));
        sheet.bus_entries.push(BusEntry {
            uuid: Uuid::new_v4(),
            position: wa,
            size: (wb.x - wa.x, wb.y - wa.y),
        });
    }
    for j in &doc.junctions {
        sheet.junctions.push(Junction {
            uuid: Uuid::new_v4(),
            position: frame.world(*j),
            diameter: 0.0,
            minted: false,
            unknown_columns: Default::default(),
        });
    }
    for nl in &doc.net_labels {
        sheet
            .labels
            .push(label(nl, LabelType::Net, "", &frame, &doc.setup));
    }
    for pp in doc.power_ports.iter().filter(|p| p.cross_sheet) {
        sheet.labels.push(Label {
            uuid: Uuid::new_v4(),
            text: pp.text.clone(),
            position: frame.world(pp.location),
            rotation: f64::from(pp.orientation) * 90.0,
            label_type: LabelType::Global,
            shape: "passive".into(),
            font_size: signex_types::schematic::SCHEMATIC_TEXT_MM,
            justify: HAlign::Left,
            justify_v: VAlign::Bottom,
            unknown_columns: Default::default(),
        });
    }
    let endpoints = connection_points(doc);
    for port in &doc.ports {
        sheet
            .labels
            .push(port_label(port, opts.port_scope, &endpoints, &frame));
    }
    for p in &doc.no_ercs {
        sheet.no_erc_directives.push(NoConnect {
            uuid: Uuid::new_v4(),
            position: frame.world(*p),
        });
    }
    for s in &doc.sheet_symbols {
        sheet.child_sheets.push(child_sheet(s, &frame, opts));
    }
    bridge_contacts(&mut sheet);
    for shape in &doc.shapes {
        sheet_shape(shape, &frame, &doc.setup, &params, &mut sheet, report);
    }
    for (kind, n) in &doc.skipped {
        let what = match *kind {
            altium_format::sch::kind::IMAGE => "embedded image".to_string(),
            k => format!("schematic record type {k}"),
        };
        for _ in 0..*n {
            report.warn(format!("{what} not imported"));
        }
    }
    sheet
}

/// Altium connects anything that merely touches: a power port dropped on
/// a pin, two pins tip to tip, a label on a pin. Signex connects only
/// through wires. Each such bare contact gets a zero-length wire, which
/// keeps the drawing exact and the connection real.
fn bridge_contacts(sheet: &mut SchematicSheet) {
    let key = |p: Point| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64);
    let mut touches: HashMap<(i64, i64), (Point, usize)> = HashMap::new();
    let mut bump = |p: Point| touches.entry(key(p)).or_insert((p, 0)).1 += 1;
    for sym in &sheet.symbols {
        let Some(lib) = sheet.lib_symbols.get(&sym.lib_id) else {
            continue;
        };
        let t = signex_types::schematic::SymbolTransform::from_symbol(sym);
        for lp in lib
            .pins
            .iter()
            .filter(|p| p.unit == 0 || p.unit == sym.unit)
        {
            bump(t.apply(lp.pin.position));
        }
    }
    for l in &sheet.labels {
        bump(l.position);
    }
    for c in &sheet.child_sheets {
        for p in &c.pins {
            bump(p.position);
        }
    }
    let on_wire = |p: Point| {
        sheet.wires.iter().any(|w| {
            let (dx, dy) = (w.end.x - w.start.x, w.end.y - w.start.y);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (((p.x - w.start.x) * dx + (p.y - w.start.y) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (cx, cy) = (w.start.x + t * dx, w.start.y + t * dy);
            (p.x - cx).hypot(p.y - cy) < 1e-4
        })
    };
    let mut bridges: Vec<Point> = touches
        .into_values()
        .filter(|(p, n)| *n >= 2 && !on_wire(*p))
        .map(|(p, _)| p)
        .collect();
    bridges.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    for p in bridges {
        sheet.wires.push(Wire {
            uuid: Uuid::new_v4(),
            start: p,
            end: p,
            ..Default::default()
        });
    }
}

/// Altium sheet -> Signex sheet coordinates.
struct Frame {
    height: f64,
}

impl Frame {
    fn world(&self, p: Pt) -> Point {
        Point::new(mm(p.x), mm(self.height - p.y))
    }
}

/// Nearest Signex paper name for an Altium drawing size (mils).
fn paper_name(size: Pt) -> &'static str {
    let (w, h) = (size.x.max(size.y) * 0.0254, size.x.min(size.y) * 0.0254);
    const SIZES: &[(&str, f64, f64)] = &[
        ("A5", 210.0, 148.0),
        ("A4", 297.0, 210.0),
        ("Letter", 279.4, 215.9),
        ("Legal", 355.6, 215.9),
        ("A3", 420.0, 297.0),
        ("Tabloid", 431.8, 279.4),
        ("A2", 594.0, 420.0),
        ("A1", 841.0, 594.0),
        ("A0", 1189.0, 841.0),
    ];
    SIZES
        .iter()
        .find(|(_, pw, ph)| *pw >= w - 1.0 && *ph >= h - 1.0)
        .map_or("A0", |(n, _, _)| *n)
}

/// Sheet + project parameters for `=Name` special strings.
struct Params {
    map: HashMap<String, String>,
}

impl Params {
    fn new(doc: &SchDoc, opts: &SchOptions) -> Self {
        let mut map = HashMap::new();
        for (k, v) in &opts.project_parameters {
            map.insert(k.to_ascii_lowercase(), v.clone());
        }
        for p in &doc.parameters {
            let v = p.text.text.trim();
            if !v.is_empty() && v != "*" {
                map.insert(p.name.to_ascii_lowercase(), v.to_string());
            }
        }
        Self { map }
    }

    /// Resolve `=Name` against these parameters and `extra`, one level
    /// deep (enough for `Comment = =Value`). Anything else is literal.
    fn resolve(&self, text: &str, extra: &[(&str, &str)]) -> String {
        let Some(name) = text.strip_prefix('=') else {
            return text.to_string();
        };
        let key = name.trim().to_ascii_lowercase();
        extra
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
            .map(|(_, v)| v.to_string())
            .or_else(|| self.map.get(&key).cloned())
            .unwrap_or_else(|| text.to_string())
    }
}

fn title_block(params: &Params) -> HashMap<String, String> {
    let mut tb = HashMap::new();
    for (signex, altium) in [
        ("title", "title"),
        ("rev", "revision"),
        ("date", "date"),
        ("company", "companyname"),
        ("comment1", "documentnumber"),
        ("comment2", "author"),
    ] {
        if let Some(v) = params.map.get(altium)
            && !v.starts_with('=')
        {
            tb.insert(signex.to_string(), v.clone());
        }
    }
    tb
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Library symbols, deduplicated by library reference and geometry.
#[derive(Default)]
struct LibTable {
    symbols: HashMap<String, LibSymbol>,
    /// Fingerprint of each registered symbol's geometry.
    shapes: HashMap<String, String>,
}

impl LibTable {
    /// Register `sym` under `base`, reusing an identical entry or adding
    /// a numbered variant when the same reference has different geometry.
    fn intern(&mut self, base: &str, mut sym: LibSymbol) -> String {
        let print = fingerprint(&sym);
        for n in 1.. {
            let id = if n == 1 {
                base.to_string()
            } else {
                format!("{base}~{n}")
            };
            match self.shapes.get(&id) {
                Some(existing) if *existing == print => return id,
                Some(_) => continue,
                None => {
                    sym.id = id.clone();
                    self.shapes.insert(id.clone(), print);
                    self.symbols.insert(id.clone(), sym);
                    return id;
                }
            }
        }
        unreachable!("1.. is unbounded")
    }
}

fn fingerprint(sym: &LibSymbol) -> String {
    format!("{:?}{:?}", sym.graphics, sym.pins)
}

/// Inverse of a placement: Altium absolute -> symbol-local (both mils, Y up).
///
/// The placement is taken as `abs = origin + R(orientation) * M(local)`
/// with `M` the X mirror. Converting back through Signex's matching
/// transform (see [`placement`]) lands every point exactly where it was,
/// whichever order Altium composes the two in.
struct Placement {
    origin: Pt,
    quarter_turns: u8,
    mirrored: bool,
}

impl Placement {
    fn local(&self, p: Pt) -> Pt {
        let (dx, dy) = (p.x - self.origin.x, p.y - self.origin.y);
        // R(-theta) for quarter turns.
        let (x, y) = match self.quarter_turns & 3 {
            0 => (dx, dy),
            1 => (dy, -dx),
            2 => (-dx, -dy),
            _ => (-dy, dx),
        };
        if self.mirrored {
            Pt::new(-x, y)
        } else {
            Pt::new(x, y)
        }
    }

    /// A direction (quarter turns, CCW) in local terms.
    fn local_turns(&self, abs_turns: u8) -> u8 {
        let r = (4 + (abs_turns & 3) - (self.quarter_turns & 3)) & 3;
        // Mirroring X maps angle a to 180 - a.
        if self.mirrored { (6 - r) & 3 } else { r }
    }
}

/// Signex `(rotation, mirror_x, mirror_y)` reproducing an Altium placement.
fn placement(orientation: u8, mirrored: bool) -> (f64, bool, bool) {
    let theta = f64::from(orientation & 3) * 90.0;
    if mirrored {
        (deg(-theta), false, true)
    } else {
        (theta, false, false)
    }
}

fn lib_pt(p: Pt) -> Point {
    Point::new(mm(p.x), mm(p.y))
}

fn component(
    comp: &Component,
    frame: &Frame,
    setup: &altium_format::sch::SheetSetup,
    params: &Params,
    libs: &mut LibTable,
    report: &mut Report,
) -> Symbol {
    let place = Placement {
        origin: comp.location,
        quarter_turns: comp.orientation,
        mirrored: comp.mirrored,
    };
    let in_mode = |mode: i32| mode == comp.display_mode;
    let unit_of = |part: i32| u32::try_from(part).ok().filter(|&p| p > 0).unwrap_or(0);

    let mut graphics = Vec::new();
    for (part, mode, shape) in &comp.shapes {
        if !in_mode(*mode) {
            continue;
        }
        for g in lib_graphic(shape, &place, setup, report) {
            graphics.push(LibGraphic {
                unit: unit_of(*part),
                body_style: 1,
                graphic: g,
            });
        }
    }
    let pins: Vec<LibPin> = comp
        .pins
        .iter()
        .filter(|p| in_mode(p.display_mode))
        .map(|p| LibPin {
            unit: unit_of(p.owner_part_id),
            body_style: 1,
            pin: lib_pin(p, &place),
        })
        .collect();

    let designator = comp
        .designator
        .as_ref()
        .map(|d| d.text.clone())
        .unwrap_or_default();
    let param_pairs: Vec<(&str, &str)> = comp
        .parameters
        .iter()
        .map(|p| (p.name.as_str(), p.text.text.as_str()))
        .collect();
    let comment = comp
        .parameter("Comment")
        .map(|c| params.resolve(c, &param_pairs));
    let value = comp
        .parameter("Value")
        .map(|v| params.resolve(v, &param_pairs))
        .or_else(|| comment.clone())
        .unwrap_or_default();
    let footprint = comp.footprint().unwrap_or_default().to_string();

    let lib = LibSymbol {
        id: String::new(),
        reference: designator_prefix(&designator),
        value: value.clone(),
        footprint: footprint.clone(),
        datasheet: String::new(),
        description: comp.description.clone(),
        keywords: String::new(),
        fp_filters: String::new(),
        in_bom: true,
        on_board: true,
        in_pos_files: true,
        duplicate_pin_numbers_are_jumpers: false,
        graphics,
        pins,
        show_pin_numbers: true,
        show_pin_names: true,
        pin_name_offset: 0.0,
    };
    let base = format!("altium:{}", comp.lib_reference);
    let lib_id = libs.intern(&base, lib);

    let mut fields = HashMap::new();
    let mut datasheet = String::new();
    for p in &comp.parameters {
        let name = p.name.as_str();
        if ["Comment", "Value"]
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let text = params.resolve(&p.text.text, &param_pairs);
        if text.is_empty() || text == "*" {
            continue;
        }
        if ["Datasheet", "HelpURL", "ComponentLink1URL"]
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name))
            && datasheet.is_empty()
        {
            datasheet = text.clone();
        }
        fields.insert(name.to_string(), text);
    }
    if !comp.design_item_id.is_empty() {
        fields.insert("DesignItemId".into(), comp.design_item_id.clone());
    }
    if let Some(c) = comment.as_ref().filter(|c| **c != value) {
        fields.insert("Comment".into(), c.clone());
    }

    let (rotation, mirror_x, mirror_y) = placement(comp.orientation, comp.mirrored);
    let value_text = comp
        .parameters
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("Value"))
        .or_else(|| {
            comp.parameters
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case("Comment"))
        })
        .map(|p| text_prop(&p.text, frame, setup));
    Symbol {
        uuid: Uuid::new_v4(),
        lib_id,
        reference: designator,
        value,
        footprint,
        datasheet,
        position: frame.world(comp.location),
        rotation,
        mirror_x,
        mirror_y,
        unit: u32::try_from(comp.current_part_id)
            .ok()
            .filter(|&u| u > 0)
            .unwrap_or(1),
        ref_text: comp.designator.as_ref().map(|d| text_prop(d, frame, setup)),
        val_text: value_text,
        fields_user_placed: true,
        fields,
        ..Default::default()
    }
}

/// "R?" / "U12" / "IC3A" -> "R" / "U" / "IC".
fn designator_prefix(designator: &str) -> String {
    let prefix: String = designator
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '#' || *c == '_')
        .collect();
    if prefix.is_empty() {
        "U".into()
    } else {
        prefix
    }
}

fn text_prop(t: &Text, frame: &Frame, setup: &altium_format::sch::SheetSetup) -> TextProp {
    let (h, v) = justify(t.justification);
    TextProp {
        position: frame.world(t.location),
        rotation: f64::from(t.orientation) * 90.0,
        font_size: setup.font_size(t.font_id) * MM_PER_PT,
        justify_h: h,
        justify_v: v,
        hidden: t.hidden,
    }
}

fn justify(j: Justification) -> (HAlign, VAlign) {
    let h = match j.horizontal() {
        -1 => HAlign::Left,
        0 => HAlign::Center,
        _ => HAlign::Right,
    };
    let v = match j.vertical() {
        -1 => VAlign::Bottom,
        0 => VAlign::Center,
        _ => VAlign::Top,
    };
    (h, v)
}

fn lib_pin(p: &APin, place: &Placement) -> Pin {
    // Altium: location is the body end, rotation points at the hot end.
    // Signex: position is the hot end, rotation points at the body.
    let hot = place.local(p.hot_end());
    let turns = place.local_turns(p.rotation);
    Pin {
        direction: direction(p.electrical),
        shape_style: pin_shape(p.symbols),
        position: lib_pt(hot),
        rotation: f64::from((turns + 2) & 3) * 90.0,
        length: mm(p.length),
        name: p.name.clone(),
        number: p.designator.clone(),
        visible: !p.hidden,
        name_visible: p.show_name,
        number_visible: p.show_designator,
    }
}

fn direction(e: Electrical) -> PinDirection {
    match e {
        Electrical::Input => PinDirection::Input,
        Electrical::InputOutput => PinDirection::Bidirectional,
        Electrical::Output => PinDirection::Output,
        Electrical::OpenCollector => PinDirection::OpenDrainLow,
        Electrical::Passive => PinDirection::Passive,
        Electrical::HiZ => PinDirection::ThreeStatable,
        Electrical::OpenEmitter => PinDirection::OpenDrainHigh,
        Electrical::Power => PinDirection::PowerInput,
    }
}

fn pin_shape(s: PinSymbols) -> PinShapeStyle {
    let clock = s.inner_edge == PinSymbols::CLOCK;
    let dot = s.outer_edge == PinSymbols::DOT;
    match (clock, dot) {
        (true, true) => PinShapeStyle::InvertedClockBubble,
        (true, false) => PinShapeStyle::ClockTriangle,
        (false, true) => PinShapeStyle::InvertedBubble,
        (false, false) => PinShapeStyle::Plain,
    }
}

fn fill(filled: bool) -> FillType {
    if filled {
        FillType::Background
    } else {
        FillType::None
    }
}

/// Three points on an arc: start, middle, end (CCW from `start` to `end`).
fn arc_points(center: Pt, radius: f64, start: f64, end: f64) -> (Pt, Pt, Pt) {
    let mut sweep = (end - start).rem_euclid(360.0);
    if sweep < 1e-9 {
        sweep = 360.0;
    }
    let at = |a: f64| {
        let r = a.to_radians();
        Pt::new(center.x + radius * r.cos(), center.y + radius * r.sin())
    };
    (at(start), at(start + sweep / 2.0), at(start + sweep))
}

fn is_full_circle(start: f64, end: f64) -> bool {
    let sweep = (end - start).rem_euclid(360.0);
    sweep < 1e-6 || (sweep - 360.0).abs() < 1e-6
}

fn ellipse_points(center: Pt, rx: f64, ry: f64) -> Vec<Pt> {
    (0..=32)
        .map(|i| {
            let a = f64::from(i) / 32.0 * std::f64::consts::TAU;
            Pt::new(center.x + rx * a.cos(), center.y + ry * a.sin())
        })
        .collect()
}

fn closed(mut pts: Vec<Pt>) -> Vec<Pt> {
    if let (Some(a), Some(b)) = (pts.first().copied(), pts.last().copied())
        && a != b
    {
        pts.push(a);
    }
    pts
}

fn lib_graphic(
    shape: &Shape,
    place: &Placement,
    setup: &altium_format::sch::SheetSetup,
    report: &mut Report,
) -> Vec<Graphic> {
    let l = |p: Pt| lib_pt(place.local(p));
    let poly = |pts: &[Pt]| pts.iter().map(|p| l(*p)).collect::<Vec<_>>();
    vec![match shape {
        Shape::Line { a, b, width } => Graphic::Polyline {
            points: vec![l(*a), l(*b)],
            width: mm(*width),
            fill: FillType::None,
        },
        Shape::Polyline { points, width } => Graphic::Polyline {
            points: poly(points),
            width: mm(*width),
            fill: FillType::None,
        },
        Shape::Polygon {
            points,
            width,
            filled,
        } => Graphic::Polyline {
            points: poly(&closed(points.clone())),
            width: mm(*width),
            fill: fill(*filled),
        },
        Shape::Rectangle {
            a,
            b,
            width,
            filled,
        } => Graphic::Rectangle {
            start: l(*a),
            end: l(*b),
            width: mm(*width),
            fill: fill(*filled),
        },
        Shape::RoundRect {
            a,
            b,
            width,
            filled,
            ..
        } => {
            report.warn("rounded rectangle imported with square corners");
            Graphic::Rectangle {
                start: l(*a),
                end: l(*b),
                width: mm(*width),
                fill: fill(*filled),
            }
        }
        Shape::Arc {
            center,
            radius,
            start,
            end,
            width,
        } => {
            if is_full_circle(*start, *end) {
                Graphic::Circle {
                    center: l(*center),
                    radius: mm(*radius),
                    width: mm(*width),
                    fill: FillType::None,
                }
            } else {
                let (s, m, e) = arc_points(*center, *radius, *start, *end);
                Graphic::Arc {
                    start: l(s),
                    mid: l(m),
                    end: l(e),
                    width: mm(*width),
                    fill: FillType::None,
                }
            }
        }
        Shape::Ellipse {
            center,
            rx,
            ry,
            width,
            filled,
        } => {
            if (rx - ry).abs() < 1e-6 {
                Graphic::Circle {
                    center: l(*center),
                    radius: mm(*rx),
                    width: mm(*width),
                    fill: fill(*filled),
                }
            } else {
                Graphic::Polyline {
                    points: poly(&ellipse_points(*center, *rx, *ry)),
                    width: mm(*width),
                    fill: fill(*filled),
                }
            }
        }
        Shape::Bezier { points, width } => Graphic::Bezier {
            points: poly(points),
            width: mm(*width),
            fill: FillType::None,
        },
        Shape::Label(t) => {
            let (h, v) = justify(t.justification);
            let turns = place.local_turns(t.orientation);
            Graphic::Text {
                text: t.text.clone(),
                position: l(t.location),
                rotation: f64::from(turns) * 90.0,
                font_size: setup.font_size(t.font_id) * MM_PER_PT,
                bold: false,
                italic: false,
                justify_h: h,
                justify_v: v,
            }
        }
        Shape::TextFrame { a, b, text, filled } => {
            let (la, lb) = (l(*a), l(*b));
            Graphic::TextBox {
                text: text.clone(),
                position: Point::new(la.x.min(lb.x), la.y.max(lb.y)),
                rotation: 0.0,
                size: Point::new((lb.x - la.x).abs(), (lb.y - la.y).abs()),
                font_size: signex_types::schematic::SCHEMATIC_TEXT_MM,
                bold: false,
                italic: false,
                width: 0.0,
                fill: fill(*filled),
            }
        }
    }]
}

// ---------------------------------------------------------------------------
// Power ports, ports, labels, sheets
// ---------------------------------------------------------------------------

/// Altium power-port style -> Signex built-in power symbol, plus the
/// orientation (quarter turns) that symbol is drawn at unrotated.
fn power_style(style: i64) -> (&'static str, u8) {
    match style {
        0 => ("power:circle", 1),
        1 => ("power:arrow", 1),
        3 => ("power:wave", 1),
        4 => ("power:GND", 3),
        5 => ("power:GNDREF", 3),
        6 => ("power:Earth", 3),
        _ => ("power:bar", 1),
    }
}

fn power_port(pp: &PowerPort, frame: &Frame, libs: &mut LibTable) -> Symbol {
    let (lib_id, natural) = power_style(pp.style);
    libs.symbols
        .entry(lib_id.to_string())
        .or_insert_with(|| LibSymbol {
            id: lib_id.to_string(),
            reference: "#PWR".into(),
            value: String::new(),
            footprint: String::new(),
            datasheet: String::new(),
            description: String::new(),
            keywords: String::new(),
            fp_filters: String::new(),
            in_bom: false,
            on_board: false,
            in_pos_files: false,
            duplicate_pin_numbers_are_jumpers: false,
            graphics: Vec::new(),
            pins: vec![LibPin {
                unit: 0,
                body_style: 1,
                pin: Pin {
                    direction: PinDirection::PowerInput,
                    shape_style: PinShapeStyle::Plain,
                    position: Point::ZERO,
                    rotation: 0.0,
                    length: 0.0,
                    name: String::new(),
                    number: "1".into(),
                    visible: false,
                    name_visible: false,
                    number_visible: false,
                },
            }],
            show_pin_numbers: false,
            show_pin_names: false,
            pin_name_offset: 0.0,
        });
    let rotation = f64::from((4 + pp.orientation - natural) & 3) * 90.0;
    let position = frame.world(pp.location);
    Symbol {
        uuid: Uuid::new_v4(),
        lib_id: lib_id.into(),
        reference: "#PWR?".into(),
        value: pp.text.clone(),
        position,
        rotation,
        is_power: true,
        in_bom: false,
        val_text: Some(TextProp {
            position: Point::new(position.x, position.y - 2.54),
            rotation: 0.0,
            font_size: signex_types::schematic::SCHEMATIC_TEXT_MM,
            justify_h: HAlign::Center,
            justify_v: VAlign::Center,
            hidden: !pp.show_net_name,
        }),
        fields_autoplaced: true,
        ..Default::default()
    }
}

fn io_direction(io: u8) -> &'static str {
    match io {
        1 => "output",
        2 => "input",
        3 => "bidirectional",
        _ => "passive",
    }
}

/// Every point a port could attach to: wire ends and pin hot ends.
fn connection_points(doc: &SchDoc) -> Vec<Pt> {
    let mut pts: Vec<Pt> = doc
        .wires
        .iter()
        .flat_map(|w| [w.first().copied(), w.last().copied()])
        .flatten()
        .collect();
    pts.extend(
        doc.components
            .iter()
            .flat_map(|c| c.pins.iter().map(APin::hot_end)),
    );
    pts
}

fn port_label(port: &Port, scope: PortScope, endpoints: &[Pt], frame: &Frame) -> Label {
    // A port connects at either end of its body; use the end something
    // touches, defaulting to its location.
    let far = if port.style >= 4 {
        Pt::new(port.location.x, port.location.y + port.width)
    } else {
        Pt::new(port.location.x + port.width, port.location.y)
    };
    let near = |a: Pt, b: Pt| (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5;
    let at_far = endpoints.iter().any(|e| near(*e, far))
        && !endpoints.iter().any(|e| near(*e, port.location));
    let (position, rotation) = match (port.style >= 4, at_far) {
        (false, false) => (port.location, 180.0),
        (false, true) => (far, 0.0),
        (true, false) => (port.location, 270.0),
        (true, true) => (far, 90.0),
    };
    Label {
        uuid: Uuid::new_v4(),
        text: port.name.clone(),
        position: frame.world(position),
        rotation,
        label_type: match scope {
            PortScope::Hierarchical => LabelType::Hierarchical,
            PortScope::Global => LabelType::Global,
        },
        shape: io_direction(port.io_type).into(),
        font_size: signex_types::schematic::SCHEMATIC_TEXT_MM,
        justify: HAlign::Left,
        justify_v: VAlign::Bottom,
        unknown_columns: Default::default(),
    }
}

fn label(
    t: &Text,
    label_type: LabelType,
    shape: &str,
    frame: &Frame,
    setup: &altium_format::sch::SheetSetup,
) -> Label {
    let (h, v) = justify(t.justification);
    Label {
        uuid: Uuid::new_v4(),
        text: t.text.clone(),
        position: frame.world(t.location),
        rotation: f64::from(t.orientation) * 90.0,
        label_type,
        shape: shape.into(),
        font_size: setup.font_size(t.font_id) * MM_PER_PT,
        justify: h,
        justify_v: v,
        unknown_columns: Default::default(),
    }
}

fn child_sheet(s: &SheetSymbol, frame: &Frame, opts: &SchOptions) -> ChildSheet {
    let top_left = frame.world(s.location);
    let size = (mm(s.size.x), mm(s.size.y));
    let filename = opts
        .sheet_files
        .get(&base_name(&s.file_name).to_ascii_lowercase())
        .cloned()
        .unwrap_or_else(|| snx_sheet_name(&s.file_name));
    let pins = s
        .entries
        .iter()
        .map(|e| {
            let d = mm(e.distance);
            let (position, rotation) = match e.side {
                1 => (Point::new(top_left.x + size.0, top_left.y + d), 180.0),
                2 => (Point::new(top_left.x + d, top_left.y), 270.0),
                3 => (Point::new(top_left.x + d, top_left.y + size.1), 90.0),
                _ => (Point::new(top_left.x, top_left.y + d), 0.0),
            };
            SheetPin {
                uuid: Uuid::new_v4(),
                name: e.name.clone(),
                direction: io_direction(e.io_type).into(),
                position,
                rotation,
                auto_generated: false,
                user_moved: true,
            }
        })
        .collect();
    ChildSheet {
        uuid: Uuid::new_v4(),
        name: if s.name.is_empty() {
            stem(&s.file_name)
        } else {
            s.name.clone()
        },
        filename,
        position: top_left,
        size,
        stroke_width: 0.0,
        fill: FillType::None,
        stroke_color: None,
        fill_color: None,
        fields_autoplaced: false,
        pins,
        instances: Vec::new(),
    }
}

/// `Sub\Power.SchDoc` -> `Power.SchDoc`.
pub fn base_name(altium_path: &str) -> &str {
    altium_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(altium_path)
}

fn stem(file: &str) -> String {
    std::path::Path::new(&file.replace('\\', "/"))
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file)
        .to_string()
}

/// `Power\Supply.SchDoc` -> `Supply.snxsch`.
pub fn snx_sheet_name(file: &str) -> String {
    format!("{}.snxsch", stem(file))
}

fn sheet_shape(
    shape: &Shape,
    frame: &Frame,
    setup: &altium_format::sch::SheetSetup,
    params: &Params,
    sheet: &mut SchematicSheet,
    report: &mut Report,
) {
    let w = |p: Pt| frame.world(p);
    let poly = |pts: &[Pt]| pts.iter().map(|p| w(*p)).collect::<Vec<_>>();
    let id = Uuid::new_v4;
    let drawing = match shape {
        Shape::Line { a, b, width } => SchDrawing::Line {
            uuid: id(),
            start: w(*a),
            end: w(*b),
            width: mm(*width),
            stroke_color: None,
        },
        Shape::Polyline { points, width } | Shape::Bezier { points, width } => {
            if matches!(shape, Shape::Bezier { .. }) {
                report.warn("sheet bezier imported as its control polygon");
            }
            SchDrawing::Polyline {
                uuid: id(),
                points: poly(points),
                width: mm(*width),
                fill: FillType::None,
                stroke_color: None,
            }
        }
        Shape::Polygon {
            points,
            width,
            filled,
        } => SchDrawing::Polyline {
            uuid: id(),
            points: poly(&closed(points.clone())),
            width: mm(*width),
            fill: fill(*filled),
            stroke_color: None,
        },
        Shape::Rectangle {
            a,
            b,
            width,
            filled,
        }
        | Shape::RoundRect {
            a,
            b,
            width,
            filled,
            ..
        } => SchDrawing::Rect {
            uuid: id(),
            start: w(*a),
            end: w(*b),
            width: mm(*width),
            fill: fill(*filled),
            stroke_color: None,
        },
        Shape::Arc {
            center,
            radius,
            start,
            end,
            width,
        } => {
            if is_full_circle(*start, *end) {
                SchDrawing::Circle {
                    uuid: id(),
                    center: w(*center),
                    radius: mm(*radius),
                    width: mm(*width),
                    fill: FillType::None,
                    stroke_color: None,
                }
            } else {
                let (s, m, e) = arc_points(*center, *radius, *start, *end);
                SchDrawing::Arc {
                    uuid: id(),
                    start: w(s),
                    mid: w(m),
                    end: w(e),
                    width: mm(*width),
                    fill: FillType::None,
                    stroke_color: None,
                }
            }
        }
        Shape::Ellipse {
            center,
            rx,
            ry,
            width,
            filled,
        } => {
            if (rx - ry).abs() < 1e-6 {
                SchDrawing::Circle {
                    uuid: id(),
                    center: w(*center),
                    radius: mm(*rx),
                    width: mm(*width),
                    fill: fill(*filled),
                    stroke_color: None,
                }
            } else {
                SchDrawing::Polyline {
                    uuid: id(),
                    points: poly(&ellipse_points(*center, *rx, *ry)),
                    width: mm(*width),
                    fill: fill(*filled),
                    stroke_color: None,
                }
            }
        }
        Shape::Label(t) => {
            let (h, v) = justify(t.justification);
            sheet.text_notes.push(TextNote {
                uuid: id(),
                text: params.resolve(&t.text, &[]),
                position: w(t.location),
                rotation: f64::from(t.orientation) * 90.0,
                font_size: setup.font_size(t.font_id) * MM_PER_PT,
                justify_h: h,
                justify_v: v,
            });
            return;
        }
        Shape::TextFrame { a, b, text, .. } => {
            let top_left = Pt::new(a.x.min(b.x), a.y.max(b.y));
            sheet.text_notes.push(TextNote {
                uuid: id(),
                text: text.clone(),
                position: w(top_left),
                rotation: 0.0,
                font_size: signex_types::schematic::SCHEMATIC_TEXT_MM,
                justify_h: HAlign::Left,
                justify_v: VAlign::Top,
            });
            return;
        }
    };
    sheet.drawings.push(drawing);
}

#[cfg(test)]
mod tests {
    use super::*;
    use signex_types::schematic::SymbolTransform;

    /// Every placement round-trips: local -> Signex world == Altium world.
    #[test]
    fn placement_round_trips_through_symbol_transform() {
        let frame = Frame { height: 8000.0 };
        let origin = Pt::new(1000.0, 2000.0);
        let probe = Pt::new(1300.0, 1900.0);
        for turns in 0..4u8 {
            for mirrored in [false, true] {
                let place = Placement {
                    origin,
                    quarter_turns: turns,
                    mirrored,
                };
                let local = place.local(probe);
                let (rotation, mirror_x, mirror_y) = placement(turns, mirrored);
                let t = SymbolTransform {
                    origin: frame.world(origin),
                    rotation_deg: rotation,
                    mirror_x,
                    mirror_y,
                };
                let got = t.apply(lib_pt(local));
                let want = frame.world(probe);
                assert!(
                    (got.x - want.x).abs() < 1e-6 && (got.y - want.y).abs() < 1e-6,
                    "turns={turns} mirrored={mirrored}: {got:?} != {want:?}"
                );
            }
        }
    }

    #[test]
    fn local_turns_follow_points() {
        // A direction maps like the vector it names.
        let origin = Pt::new(0.0, 0.0);
        for turns in 0..4u8 {
            for mirrored in [false, true] {
                let place = Placement {
                    origin,
                    quarter_turns: turns,
                    mirrored,
                };
                for d in 0..4u8 {
                    let (dx, dy) = altium_format::sch::quarter_turn_dir(d);
                    let v = place.local(Pt::new(dx, dy));
                    let (ex, ey) = altium_format::sch::quarter_turn_dir(place.local_turns(d));
                    assert!(
                        (v.x - ex).abs() < 1e-9 && (v.y - ey).abs() < 1e-9,
                        "turns={turns} mirrored={mirrored} d={d}"
                    );
                }
            }
        }
    }

    #[test]
    fn designator_prefixes() {
        assert_eq!(designator_prefix("R?"), "R");
        assert_eq!(designator_prefix("IC12"), "IC");
        assert_eq!(designator_prefix("?"), "U");
    }

    #[test]
    fn paper_names() {
        assert_eq!(paper_name(Pt::new(11500.0, 7600.0)), "A4");
        assert_eq!(paper_name(Pt::new(15500.0, 11100.0)), "A3");
        assert_eq!(paper_name(Pt::new(42000.0, 32000.0)), "A0");
    }
}
