//! `.PcbDoc` and `.PcbLib` into a typed primitive model.
//!
//! Geometry stays in Altium's frame: mils, Y up, absolute board
//! coordinates (footprint-local for library footprints). Binary
//! coordinates are `i32` in 1/10000 mil; text-record lengths carry a unit
//! suffix. Record layouts: `docs/FORMAT_NOTES.md`.

use std::collections::HashMap;

use crate::bin::Reader;
use crate::container::Container;
use crate::error::Result;
use crate::params::Params;
use crate::sch::Pt;

/// Altium's layer numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Layer(pub u8);

impl Layer {
    pub const TOP: Layer = Layer(1);
    pub const BOTTOM: Layer = Layer(32);
    pub const TOP_OVERLAY: Layer = Layer(33);
    pub const BOTTOM_OVERLAY: Layer = Layer(34);
    pub const TOP_PASTE: Layer = Layer(35);
    pub const BOTTOM_PASTE: Layer = Layer(36);
    pub const TOP_SOLDER: Layer = Layer(37);
    pub const BOTTOM_SOLDER: Layer = Layer(38);
    pub const DRILL_GUIDE: Layer = Layer(55);
    pub const KEEP_OUT: Layer = Layer(56);
    pub const DRILL_DRAWING: Layer = Layer(73);
    pub const MULTI_LAYER: Layer = Layer(74);

    /// Mid layer `n` (1-based) for 1..=30.
    pub fn mid(self) -> Option<u8> {
        (2..=31).contains(&self.0).then(|| self.0 - 1)
    }
    /// Internal plane `n` (1-based) for 1..=16.
    pub fn plane(self) -> Option<u8> {
        (39..=54).contains(&self.0).then(|| self.0 - 38)
    }
    /// Mechanical layer `n` (1-based) for 1..=16.
    pub fn mechanical(self) -> Option<u8> {
        (57..=72).contains(&self.0).then(|| self.0 - 56)
    }

    pub fn is_copper(self) -> bool {
        self == Self::TOP || self == Self::BOTTOM || self.mid().is_some() || self.plane().is_some()
    }

    /// Parse the text-record spelling (`TOP`, `MID3`, `MECHANICAL1`, ...).
    pub fn from_name(name: &str) -> Option<Layer> {
        let n = name
            .trim()
            .to_ascii_uppercase()
            .replace([' ', '-', '_'], "");
        let num = |prefix: &str| n.strip_prefix(prefix).and_then(|r| r.parse::<u8>().ok());
        Some(match n.as_str() {
            "TOP" | "TOPLAYER" => Self::TOP,
            "BOTTOM" | "BOTTOMLAYER" => Self::BOTTOM,
            "TOPOVERLAY" => Self::TOP_OVERLAY,
            "BOTTOMOVERLAY" => Self::BOTTOM_OVERLAY,
            "TOPPASTE" => Self::TOP_PASTE,
            "BOTTOMPASTE" => Self::BOTTOM_PASTE,
            "TOPSOLDER" => Self::TOP_SOLDER,
            "BOTTOMSOLDER" => Self::BOTTOM_SOLDER,
            "DRILLGUIDE" => Self::DRILL_GUIDE,
            "KEEPOUT" | "KEEPOUTLAYER" => Self::KEEP_OUT,
            "DRILLDRAWING" => Self::DRILL_DRAWING,
            "MULTILAYER" => Self::MULTI_LAYER,
            _ => {
                let mid = num("MIDLAYER")
                    .or_else(|| num("MID"))
                    .filter(|m| (1..=30).contains(m));
                let plane = num("INTERNALPLANE")
                    .or_else(|| num("PLANE"))
                    .filter(|m| (1..=16).contains(m));
                let mech = num("MECHANICAL")
                    .or_else(|| num("MECH"))
                    .filter(|m| (1..=16).contains(m));
                mid.map(|m| Layer(m + 1))
                    .or(plane.map(|m| Layer(m + 38)))
                    .or(mech.map(|m| Layer(m + 56)))?
            }
        })
    }
}

/// Fields every binary primitive starts with.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Common {
    pub layer: Layer,
    pub locked: bool,
    pub keepout: bool,
    pub net: Option<u16>,
    pub polygon: Option<u16>,
    pub component: Option<u16>,
}

fn index(raw: u16) -> Option<u16> {
    (raw != u16::MAX).then_some(raw)
}

fn common(r: &mut Reader) -> Result<Common> {
    let layer = Layer(r.u8("layer")?);
    let f1 = r.u8("flags")?;
    let f2 = r.u8("flags")?;
    let net = index(r.u16("net")?);
    let polygon = index(r.u16("polygon")?);
    let component = index(r.u16("component")?);
    r.skip(4, "reserved")?;
    Ok(Common {
        layer,
        // Bit 2 of the first flag byte is *unlocked*.
        locked: f1 & 0x04 == 0,
        keepout: f2 & 0x02 != 0,
        net,
        polygon,
        component,
    })
}

/// Binary coordinate unit: 1/10000 mil.
fn coord(r: &mut Reader, what: &'static str) -> Result<f64> {
    Ok(f64::from(r.i32(what)?) / 10_000.0)
}

fn pt(r: &mut Reader, what: &'static str) -> Result<Pt> {
    Ok(Pt::new(coord(r, what)?, coord(r, what)?))
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    pub common: Common,
    pub a: Pt,
    pub b: Pt,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arc {
    pub common: Common,
    pub center: Pt,
    pub radius: f64,
    /// Degrees, counter-clockwise from +X.
    pub start: f64,
    pub end: f64,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Via {
    pub common: Common,
    pub position: Pt,
    pub diameter: f64,
    pub hole: f64,
    pub start_layer: Layer,
    pub end_layer: Layer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PadShape {
    #[default]
    Round,
    Rect,
    Octagonal,
    RoundedRect,
}

impl PadShape {
    fn from_code(code: u8) -> Self {
        match code {
            2 => Self::Rect,
            3 => Self::Octagonal,
            9 => Self::RoundedRect,
            _ => Self::Round,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HoleShape {
    #[default]
    Round,
    Square,
    Slot,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pad {
    pub common: Common,
    pub name: String,
    pub position: Pt,
    pub size_top: Pt,
    pub size_mid: Pt,
    pub size_bottom: Pt,
    pub hole: f64,
    pub shape_top: PadShape,
    pub shape_bottom: PadShape,
    pub rotation: f64,
    pub plated: bool,
    pub hole_shape: HoleShape,
    /// Slot length for [`HoleShape::Slot`].
    pub slot_length: f64,
    /// Top-layer corner radius, percent of the shorter side's half.
    pub corner_radius_pct: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fill {
    pub common: Common,
    pub a: Pt,
    pub b: Pt,
    pub rotation: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub common: Common,
    pub position: Pt,
    pub height: f64,
    pub rotation: f64,
    pub mirrored: bool,
    pub stroke_width: f64,
    pub is_comment: bool,
    pub is_designator: bool,
    pub text: String,
}

/// A copper region, cutout, or board cutout.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Region {
    pub common: Common,
    /// 0 copper, 1 polygon cutout, 2 board cutout (per `KIND=`).
    pub kind: i64,
    pub outline: Vec<Pt>,
    pub params: Params,
}

/// One vertex of a text-record outline (board shape, polygon).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vertex {
    pub at: Pt,
    /// `Some` when the edge leaving this vertex is an arc.
    pub arc: Option<VertexArc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VertexArc {
    pub center: Pt,
    pub radius: f64,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Polygon {
    pub layer: Layer,
    pub net: Option<u16>,
    pub vertices: Vec<Vertex>,
    pub params: Params,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Component {
    pub layer: Layer,
    pub position: Pt,
    pub rotation: f64,
    pub pattern: String,
    pub locked: bool,
    pub unique_id: String,
    pub source_library: String,
    pub description: String,
    pub params: Params,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Board {
    pub origin: Pt,
    pub outline: Vec<Vertex>,
    /// Copper layers, top to bottom, with their display names.
    pub copper_stack: Vec<(Layer, String)>,
    pub params: Params,
}

/// Every primitive kind, as they appear in a footprint stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Arc(Arc),
    Pad(Pad),
    Via(Via),
    Track(Track),
    Text(Text),
    Fill(Fill),
    Region(Region),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Primitives {
    pub arcs: Vec<Arc>,
    pub pads: Vec<Pad>,
    pub vias: Vec<Via>,
    pub tracks: Vec<Track>,
    pub texts: Vec<Text>,
    pub fills: Vec<Fill>,
    pub regions: Vec<Region>,
    /// Primitive type ids this model skips, with counts.
    pub skipped: HashMap<u8, usize>,
}

impl Primitives {
    fn push(&mut self, p: Primitive) {
        match p {
            Primitive::Arc(x) => self.arcs.push(x),
            Primitive::Pad(x) => self.pads.push(x),
            Primitive::Via(x) => self.vias.push(x),
            Primitive::Track(x) => self.tracks.push(x),
            Primitive::Text(x) => self.texts.push(x),
            Primitive::Fill(x) => self.fills.push(x),
            Primitive::Region(x) => self.regions.push(x),
        }
    }
}

/// One `.PcbDoc`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PcbDoc {
    pub board: Board,
    pub nets: Vec<String>,
    pub components: Vec<Component>,
    pub polygons: Vec<Polygon>,
    pub primitives: Primitives,
}

impl PcbDoc {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let mut c = Container::open(path)?;
        Self::from_container(&mut c)
    }

    pub fn from_container(c: &mut Container) -> Result<Self> {
        let wide = wide_strings(c.stream_opt("WideStrings6/Data")?.as_deref());
        let mut prims = Primitives::default();
        for storage in [
            "Arcs6", "Pads6", "Vias6", "Tracks6", "Texts6", "Fills6", "Regions6",
        ] {
            if let Some(data) = c.stream_opt(&format!("{storage}/Data"))? {
                parse_primitives(&data, storage, &wide, &mut prims)?;
            }
        }
        let nets = text_records(c.stream_opt("Nets6/Data")?.as_deref(), "Nets6")?
            .iter()
            .map(|p| p.string("NAME"))
            .collect();
        let components = text_records(c.stream_opt("Components6/Data")?.as_deref(), "Components6")?
            .iter()
            .map(component)
            .collect();
        let polygons = text_records(c.stream_opt("Polygons6/Data")?.as_deref(), "Polygons6")?
            .iter()
            .map(polygon)
            .collect();
        let board = text_records(c.stream_opt("Board6/Data")?.as_deref(), "Board6")?
            .first()
            .map(board)
            .unwrap_or_default();
        Ok(Self {
            board,
            nets,
            components,
            polygons,
            primitives: prims,
        })
    }

    pub fn net_name(&self, net: Option<u16>) -> Option<&str> {
        net.and_then(|n| self.nets.get(usize::from(n)))
            .map(String::as_str)
    }
}

/// One footprint in a `.PcbLib`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Footprint {
    pub name: String,
    pub primitives: Primitives,
    pub params: Params,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PcbLib {
    pub footprints: Vec<Footprint>,
}

impl PcbLib {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let mut c = Container::open(path)?;
        Self::from_container(&mut c)
    }

    pub fn from_container(c: &mut Container) -> Result<Self> {
        let mut lib = PcbLib::default();
        for storage in c.root_storages() {
            if storage == "Library" {
                continue;
            }
            let Some(data) = c.stream_opt(&format!("{storage}/Data"))? else {
                continue;
            };
            let wide = wide_strings(c.stream_opt(&format!("{storage}/WideStrings"))?.as_deref());
            let mut r = Reader::new(&data, "footprint Data");
            let name_block = r.block("footprint name")?;
            let name = Reader::new(name_block, "footprint name")
                .pascal_str("footprint name")
                .unwrap_or_else(|_| storage.clone());
            let mut primitives = Primitives::default();
            parse_primitives(&data[r.pos()..], &storage, &wide, &mut primitives)?;
            let params = c
                .stream_opt(&format!("{storage}/Parameters"))?
                .and_then(|raw| text_records(Some(&raw), "Parameters").ok())
                .and_then(|mut v| (!v.is_empty()).then(|| v.remove(0)))
                .unwrap_or_default();
            lib.footprints.push(Footprint {
                name,
                primitives,
                params,
            });
        }
        Ok(lib)
    }
}

/// `[u32 len][text]` records, as used by Nets6, Components6, Board6 ...
pub fn text_records(data: Option<&[u8]>, stream: &str) -> Result<Vec<Params>> {
    let Some(data) = data else {
        return Ok(Vec::new());
    };
    let mut r = Reader::new(data, stream);
    let mut out = Vec::new();
    while r.remaining() >= 4 {
        out.push(Params::parse(r.block("text record")?));
    }
    Ok(out)
}

/// `WideStrings6`: `[u32 index][u32 byte len][UTF-16LE]` entries.
fn wide_strings(data: Option<&[u8]>) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let Some(data) = data else { return map };
    // Some files store this as a single text record of ENCODEDTEXTn keys.
    if data.len() > 4 && data[4] == b'|' {
        if let Ok(recs) = text_records(Some(data), "WideStrings") {
            for p in recs {
                for (k, v) in p.iter() {
                    if let Some(n) = k.to_ascii_uppercase().strip_prefix("ENCODEDTEXT")
                        && let Ok(i) = n.parse::<u32>()
                    {
                        let text: String = v
                            .split(',')
                            .filter_map(|c| c.trim().parse::<u32>().ok())
                            .filter_map(char::from_u32)
                            .collect();
                        map.insert(i, text);
                    }
                }
            }
        }
        return map;
    }
    let mut r = Reader::new(data, "WideStrings");
    while r.remaining() >= 8 {
        let (Ok(i), Ok(bytes)) = (r.u32("index"), r.block("text")) else {
            break;
        };
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        map.insert(i, String::from_utf16_lossy(&units));
    }
    map
}

/// Parse a run of `[u8 type][blocks...]` primitive records.
pub fn parse_primitives(
    data: &[u8],
    stream: &str,
    wide: &HashMap<u32, String>,
    out: &mut Primitives,
) -> Result<()> {
    let mut r = Reader::new(data, stream);
    while !r.is_empty() {
        let ty = r.u8("primitive type")?;
        match ty {
            1 => out.push(Primitive::Arc(arc(r.block("arc")?, stream)?)),
            2 => out.push(Primitive::Pad(pad(&mut r)?)),
            3 => out.push(Primitive::Via(via(r.block("via")?, stream)?)),
            4 => out.push(Primitive::Track(track(r.block("track")?, stream)?)),
            5 => {
                let main = r.block("text")?;
                let string = r.block("text string")?;
                out.push(Primitive::Text(text(main, string, wide, stream)?));
            }
            6 => out.push(Primitive::Fill(fill(r.block("fill")?, stream)?)),
            11 => out.push(Primitive::Region(region(r.block("region")?, stream)?)),
            // 12 component body, 16 dimension, ...: single-block records.
            0 => break,
            other => {
                r.block("skipped primitive")?;
                *out.skipped.entry(other).or_default() += 1;
            }
        }
    }
    Ok(())
}

fn track(b: &[u8], s: &str) -> Result<Track> {
    let mut r = Reader::new(b, s);
    let common = common(&mut r)?;
    Ok(Track {
        common,
        a: pt(&mut r, "track start")?,
        b: pt(&mut r, "track end")?,
        width: coord(&mut r, "track width")?,
    })
}

fn arc(b: &[u8], s: &str) -> Result<Arc> {
    let mut r = Reader::new(b, s);
    let common = common(&mut r)?;
    Ok(Arc {
        common,
        center: pt(&mut r, "arc center")?,
        radius: coord(&mut r, "arc radius")?,
        start: r.f64_finite("arc start")?,
        end: r.f64_finite("arc end")?,
        width: coord(&mut r, "arc width")?,
    })
}

fn via(b: &[u8], s: &str) -> Result<Via> {
    let mut r = Reader::new(b, s);
    let common = common(&mut r)?;
    let position = pt(&mut r, "via position")?;
    let diameter = coord(&mut r, "via diameter")?;
    let hole = coord(&mut r, "via hole")?;
    let start_layer = Layer(r.u8("via start layer")?);
    let end_layer = Layer(r.u8("via end layer")?);
    Ok(Via {
        common,
        position,
        diameter,
        hole,
        start_layer,
        end_layer,
    })
}

fn fill(b: &[u8], s: &str) -> Result<Fill> {
    let mut r = Reader::new(b, s);
    let common = common(&mut r)?;
    Ok(Fill {
        common,
        a: pt(&mut r, "fill a")?,
        b: pt(&mut r, "fill b")?,
        rotation: r.f64_finite("fill rotation")?,
    })
}

fn pad(r: &mut Reader) -> Result<Pad> {
    let name_block = r.block("pad name")?;
    let name = Reader::new(name_block, "pad name")
        .pascal_str("pad name")
        .unwrap_or_default();
    r.block("pad block 1")?;
    r.block("pad block 2")?;
    r.block("pad block 3")?;
    let main = r.block("pad main")?;
    let shape_block = r.block("pad size-and-shape")?;

    let mut m = Reader::new(main, "pad main");
    let common = common(&mut m)?;
    let position = pt(&mut m, "pad position")?;
    let size_top = pt(&mut m, "pad top size")?;
    let size_mid = pt(&mut m, "pad mid size")?;
    let size_bottom = pt(&mut m, "pad bottom size")?;
    let hole = coord(&mut m, "pad hole")?;
    let shape_top = PadShape::from_code(m.u8("pad top shape")?);
    m.skip(1, "pad mid shape")?;
    let shape_bottom = PadShape::from_code(m.u8("pad bottom shape")?);
    let rotation = m.f64_finite("pad rotation")?;
    let plated = m.u8("pad plated")? != 0;

    let mut pad = Pad {
        common,
        name,
        position,
        size_top,
        size_mid,
        size_bottom,
        hole,
        shape_top,
        shape_bottom,
        rotation,
        plated,
        ..Default::default()
    };
    if !shape_block.is_empty() {
        // Ignore a short or odd block: the main block already has a pad.
        let _ = apply_size_and_shape(shape_block, &mut pad);
    }
    Ok(pad)
}

/// The optional per-layer block: 29 mid-layer sizes and shapes, the hole
/// shape, slot length, per-layer offsets, then per-layer alternate shapes
/// and corner radii (index 0 = top).
fn apply_size_and_shape(b: &[u8], pad: &mut Pad) -> Result<()> {
    let mut r = Reader::new(b, "pad size-and-shape");
    r.skip(29 * 4 * 2, "mid sizes")?;
    r.skip(29, "mid shapes")?;
    r.skip(1, "reserved")?;
    pad.hole_shape = match r.u8("hole shape")? {
        1 => HoleShape::Square,
        2 => HoleShape::Slot,
        _ => HoleShape::Round,
    };
    pad.slot_length = coord(&mut r, "slot length")?;
    r.skip(8, "hole rotation")?;
    r.skip(32 * 4 * 2, "offsets")?;
    let has_alt = r.u8("alt shapes present")? != 0;
    let alt: Vec<u8> = r.take(32, "alt shapes")?.to_vec();
    let radius: Vec<u8> = r.take(32, "corner radius")?.to_vec();
    if has_alt && alt[0] == 9 {
        pad.shape_top = PadShape::RoundedRect;
        pad.corner_radius_pct = radius[0];
    }
    if has_alt && alt[31] == 9 {
        pad.shape_bottom = PadShape::RoundedRect;
    }
    Ok(())
}

fn text(main: &[u8], string: &[u8], wide: &HashMap<u32, String>, s: &str) -> Result<Text> {
    let mut r = Reader::new(main, s);
    let common = common(&mut r)?;
    let position = pt(&mut r, "text position")?;
    let height = coord(&mut r, "text height")?;
    r.skip(2, "stroke font")?;
    let rotation = r.f64_finite("text rotation")?;
    let mirrored = r.u8("text mirror")? != 0;
    let stroke_width = coord(&mut r, "text stroke width")?;
    let is_comment = r.u8("is comment")? != 0;
    let is_designator = r.u8("is designator")? != 0;
    // The wide-string index sits at a fixed offset in current files.
    let wide_text = main
        .get(115..119)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .and_then(|i| wide.get(&i))
        .filter(|t| !t.is_empty())
        .cloned();
    let ansi = Reader::new(string, s)
        .pascal_str("text")
        .unwrap_or_default();
    Ok(Text {
        common,
        position,
        height,
        rotation,
        mirrored,
        stroke_width,
        is_comment,
        is_designator,
        text: wide_text.unwrap_or(ansi),
    })
}

fn region(b: &[u8], s: &str) -> Result<Region> {
    let mut r = Reader::new(b, s);
    let common = common(&mut r)?;
    r.skip(5, "region reserved")?;
    let params = Params::parse(r.block("region params")?);
    let n = r.u32("region vertex count")? as usize;
    let mut outline = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let x = r.f64("region x")? / 10_000.0;
        let y = r.f64("region y")? / 10_000.0;
        if x.is_finite() && y.is_finite() {
            outline.push(Pt::new(x, y));
        }
    }
    Ok(Region {
        common,
        kind: params.i64("KIND").unwrap_or(0),
        outline,
        params,
    })
}

fn mils(p: &Params, key: &str) -> f64 {
    p.pcb_mils(key).unwrap_or(0.0)
}

fn component(p: &Params) -> Component {
    Component {
        layer: Layer::from_name(p.str("LAYER")).unwrap_or(Layer::TOP),
        position: Pt::new(mils(p, "X"), mils(p, "Y")),
        rotation: p.f64_or("ROTATION", 0.0),
        pattern: p.string("PATTERN"),
        locked: p.bool("LOCKED"),
        unique_id: p.string("UNIQUEID"),
        source_library: p.string("SOURCEFOOTPRINTLIBRARY"),
        description: p.string("FOOTPRINTDESCRIPTION"),
        params: p.clone(),
    }
}

fn vertices(p: &Params) -> Vec<Vertex> {
    let mut out = Vec::new();
    for i in 0.. {
        let (Some(x), Some(y)) = (p.pcb_mils(&format!("VX{i}")), p.pcb_mils(&format!("VY{i}")))
        else {
            break;
        };
        let arc = (p.i64(&format!("KIND{i}")) == Some(1)).then(|| VertexArc {
            center: Pt::new(mils(p, &format!("CX{i}")), mils(p, &format!("CY{i}"))),
            radius: mils(p, &format!("R{i}")),
            start: p.f64_or(&format!("SA{i}"), 0.0),
            end: p.f64_or(&format!("EA{i}"), 0.0),
        });
        out.push(Vertex {
            at: Pt::new(x, y),
            arc,
        });
    }
    out
}

fn polygon(p: &Params) -> Polygon {
    Polygon {
        layer: Layer::from_name(p.str("LAYER")).unwrap_or(Layer::TOP),
        net: p
            .i64("NET")
            .and_then(|n| u16::try_from(n).ok())
            .and_then(index),
        vertices: vertices(p),
        params: p.clone(),
    }
}

fn board(p: &Params) -> Board {
    // Copper order is a linked list: LAYERnNEXT from the top layer.
    let name = |id: u8| {
        let n = p.string(&format!("LAYER{id}NAME"));
        if n.is_empty() {
            format!("Layer {id}")
        } else {
            n
        }
    };
    let mut stack = vec![(Layer::TOP, name(Layer::TOP.0))];
    let mut id = Layer::TOP.0;
    while id != Layer::BOTTOM.0 && stack.len() < 34 {
        let next = p
            .i64(&format!("LAYER{id}NEXT"))
            .and_then(|n| u8::try_from(n).ok())
            .filter(|&n| n != 0 && Layer(n).is_copper())
            .unwrap_or(Layer::BOTTOM.0);
        stack.push((Layer(next), name(next)));
        id = next;
    }
    Board {
        origin: Pt::new(mils(p, "ORIGINX"), mils(p, "ORIGINY")),
        outline: vertices(p),
        copper_stack: stack,
        params: p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_names() {
        assert_eq!(Layer::from_name("TOP"), Some(Layer::TOP));
        assert_eq!(Layer::from_name("MID3"), Some(Layer(4)));
        assert_eq!(Layer::from_name("MECHANICAL1"), Some(Layer(57)));
        assert_eq!(Layer::from_name("Mechanical 13"), Some(Layer(69)));
        assert_eq!(Layer::from_name("PLANE2"), Some(Layer(40)));
        assert!(Layer(40).is_copper() && !Layer::TOP_OVERLAY.is_copper());
        assert_eq!(Layer::from_name("MECHANICAL250"), None);
        assert_eq!(Layer::from_name("MID31"), None);
    }
}
