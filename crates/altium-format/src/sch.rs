//! `.SchDoc` and `.SchLib` into a typed record model.
//!
//! Geometry stays in Altium's frame: mils, Y up, absolute sheet
//! coordinates for documents and symbol-local coordinates for libraries.
//! Every typed record keeps its raw [`Params`] so a converter can reach a
//! property this model does not lift.

use std::collections::HashMap;

use crate::bin::Reader;
use crate::container::Container;
use crate::error::{Error, Result};
use crate::params::Params;
use crate::sch_stream::{Frame, frames};

/// A point in mils, Y up.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Record kinds, by the `RECORD=` number.
pub mod kind {
    pub const COMPONENT: i64 = 1;
    pub const PIN: i64 = 2;
    pub const LABEL: i64 = 4;
    pub const BEZIER: i64 = 5;
    pub const POLYLINE: i64 = 6;
    pub const POLYGON: i64 = 7;
    pub const ELLIPSE: i64 = 8;
    pub const PIE: i64 = 9;
    pub const ROUND_RECT: i64 = 10;
    pub const ELLIPTICAL_ARC: i64 = 11;
    pub const ARC: i64 = 12;
    pub const LINE: i64 = 13;
    pub const RECTANGLE: i64 = 14;
    pub const SHEET_SYMBOL: i64 = 15;
    pub const SHEET_ENTRY: i64 = 16;
    pub const POWER_PORT: i64 = 17;
    pub const PORT: i64 = 18;
    pub const NO_ERC: i64 = 22;
    pub const NET_LABEL: i64 = 25;
    pub const BUS: i64 = 26;
    pub const WIRE: i64 = 27;
    pub const TEXT_FRAME: i64 = 28;
    pub const JUNCTION: i64 = 29;
    pub const IMAGE: i64 = 30;
    pub const SHEET: i64 = 31;
    pub const SHEET_NAME: i64 = 32;
    pub const SHEET_FILE_NAME: i64 = 33;
    pub const DESIGNATOR: i64 = 34;
    pub const BUS_ENTRY: i64 = 37;
    pub const PARAMETER: i64 = 41;
    pub const IMPLEMENTATION_LIST: i64 = 44;
    pub const IMPLEMENTATION: i64 = 45;
}

/// Pin electrical type, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Electrical {
    Input,
    InputOutput,
    Output,
    OpenCollector,
    #[default]
    Passive,
    HiZ,
    OpenEmitter,
    Power,
}

impl Electrical {
    fn from_code(code: i64) -> Self {
        match code {
            0 => Self::Input,
            1 => Self::InputOutput,
            2 => Self::Output,
            3 => Self::OpenCollector,
            5 => Self::HiZ,
            6 => Self::OpenEmitter,
            7 => Self::Power,
            _ => Self::Passive,
        }
    }
}

/// Edge / inside decorations on a pin (IEEE symbol codes, as stored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PinSymbols {
    pub inner_edge: u8,
    pub outer_edge: u8,
    pub inside: u8,
    pub outside: u8,
}

impl PinSymbols {
    pub const CLOCK: u8 = 3;
    pub const DOT: u8 = 1;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pin {
    pub name: String,
    pub designator: String,
    pub description: String,
    pub electrical: Electrical,
    /// The body end of the pin. The electrical end is `length` further
    /// along `rotation`.
    pub location: Pt,
    pub length: f64,
    /// Quarter turns counter-clockwise: 0 points the electrical end to +X.
    pub rotation: u8,
    pub hidden: bool,
    pub show_name: bool,
    pub show_designator: bool,
    pub symbols: PinSymbols,
    pub owner_part_id: i32,
    pub display_mode: i32,
}

impl Pin {
    /// Bit layout of the "conglomerate" byte shared by the binary and
    /// text forms. See `docs/FORMAT_NOTES.md`.
    fn apply_conglomerate(&mut self, c: u8) {
        self.rotation = c & 0x03;
        self.hidden = c & 0x04 != 0;
        self.show_name = c & 0x08 != 0;
        self.show_designator = c & 0x10 != 0;
    }

    /// The point a wire attaches to.
    pub fn hot_end(&self) -> Pt {
        let (dx, dy) = quarter_turn_dir(self.rotation);
        Pt::new(
            self.location.x + dx * self.length,
            self.location.y + dy * self.length,
        )
    }

    fn from_params(p: &Params) -> Self {
        let mut pin = Pin {
            name: p.string("Name"),
            designator: p.string("Designator"),
            description: p.string("Description"),
            electrical: Electrical::from_code(p.i64("Electrical").unwrap_or(0)),
            location: loc(p, "Location"),
            length: p.sch_mils("PinLength"),
            symbols: PinSymbols {
                inner_edge: p.i64("Symbol_InnerEdge").unwrap_or(0) as u8,
                outer_edge: p.i64("Symbol_OuterEdge").unwrap_or(0) as u8,
                inside: p.i64("Symbol_Inside").unwrap_or(0) as u8,
                outside: p.i64("Symbol_Outside").unwrap_or(0) as u8,
            },
            owner_part_id: p.i32_or("OwnerPartId", -1),
            display_mode: p.i32_or("OwnerPartDisplayMode", 0),
            ..Default::default()
        };
        pin.apply_conglomerate(p.i64("PinConglomerate").unwrap_or(0) as u8);
        pin
    }

    /// The packed library form. Layout notes: `docs/FORMAT_NOTES.md`.
    fn from_binary(buf: &[u8]) -> Result<Self> {
        let mut r = Reader::new(buf, "pin record");
        let record = r.u32("record id")?;
        if record != kind::PIN as u32 {
            return Err(Error::Malformed(format!(
                "binary schematic record {record} is not a pin"
            )));
        }
        r.skip(1, "pin reserved")?;
        let owner_part_id = i32::from(r.i16("owner part")?);
        let display_mode = i32::from(r.u8("display mode")?);
        let symbols = PinSymbols {
            inner_edge: r.u8("inner edge")?,
            outer_edge: r.u8("outer edge")?,
            inside: r.u8("inside")?,
            outside: r.u8("outside")?,
        };
        let description = r.pascal_str("description")?;
        r.skip(1, "formal type")?;
        let electrical = Electrical::from_code(i64::from(r.u8("electrical")?));
        let conglomerate = r.u8("conglomerate")?;
        let length = f64::from(r.i16("length")?) * 10.0;
        let x = f64::from(r.i16("x")?) * 10.0;
        let y = f64::from(r.i16("y")?) * 10.0;
        r.skip(4, "color")?;
        let name = r.pascal_str("name")?;
        let designator = r.pascal_str("designator")?;
        let mut pin = Pin {
            name,
            designator,
            description,
            electrical,
            location: Pt::new(x, y),
            length,
            symbols,
            owner_part_id,
            display_mode,
            ..Default::default()
        };
        pin.apply_conglomerate(conglomerate);
        Ok(pin)
    }
}

/// Unit vector of a quarter-turn rotation, Y up.
pub fn quarter_turn_dir(rotation: u8) -> (f64, f64) {
    match rotation & 3 {
        0 => (1.0, 0.0),
        1 => (0.0, 1.0),
        2 => (-1.0, 0.0),
        _ => (0.0, -1.0),
    }
}

/// Text justification, as Altium numbers it: 0..=8, bottom row first,
/// left to right (0 = bottom-left, 4 = center, 8 = top-right).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Justification(pub u8);

impl Justification {
    /// -1 left, 0 center, 1 right.
    pub fn horizontal(self) -> i8 {
        (self.0 % 3) as i8 - 1
    }
    /// -1 bottom, 0 center, 1 top.
    pub fn vertical(self) -> i8 {
        (self.0 / 3) as i8 - 1
    }
}

/// A free text item: labels, designators, parameters, sheet names.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub text: String,
    pub location: Pt,
    pub orientation: u8,
    pub justification: Justification,
    pub font_id: i64,
    pub hidden: bool,
}

impl Text {
    fn from_params(p: &Params) -> Self {
        Self {
            text: p.string("Text"),
            location: loc(p, "Location"),
            orientation: p.i64("Orientation").unwrap_or(0) as u8 & 3,
            justification: Justification(p.i64("Justification").unwrap_or(0).clamp(0, 8) as u8),
            font_id: p.i64("FontID").unwrap_or(1),
            hidden: p.bool("IsHidden"),
        }
    }
}

/// A component parameter (`RECORD=41`), or a sheet parameter when unowned.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parameter {
    pub name: String,
    pub text: Text,
}

/// A footprint (or other model) link (`RECORD=45`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Implementation {
    pub model_name: String,
    pub model_type: String,
    pub is_current: bool,
}

/// Body graphics, shared by symbols and sheets.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Line {
        a: Pt,
        b: Pt,
        width: f64,
    },
    Polyline {
        points: Vec<Pt>,
        width: f64,
    },
    Polygon {
        points: Vec<Pt>,
        width: f64,
        filled: bool,
    },
    Rectangle {
        a: Pt,
        b: Pt,
        width: f64,
        filled: bool,
    },
    RoundRect {
        a: Pt,
        b: Pt,
        rx: f64,
        ry: f64,
        width: f64,
        filled: bool,
    },
    /// Angles in degrees, counter-clockwise from +X.
    Arc {
        center: Pt,
        radius: f64,
        start: f64,
        end: f64,
        width: f64,
    },
    Ellipse {
        center: Pt,
        rx: f64,
        ry: f64,
        width: f64,
        filled: bool,
    },
    Bezier {
        points: Vec<Pt>,
        width: f64,
    },
    Label(Text),
    TextFrame {
        a: Pt,
        b: Pt,
        text: String,
        filled: bool,
    },
}

/// A placed or library component and everything it owns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Component {
    pub lib_reference: String,
    pub description: String,
    pub design_item_id: String,
    /// Number of parts (units). Altium stores this plus one.
    pub part_count: u32,
    pub current_part_id: i32,
    pub display_mode: i32,
    pub location: Pt,
    pub orientation: u8,
    pub mirrored: bool,
    pub unique_id: String,
    pub designator: Option<Text>,
    pub parameters: Vec<Parameter>,
    pub implementations: Vec<Implementation>,
    pub pins: Vec<Pin>,
    /// Body graphics with the part id they belong to (-1 = every part).
    pub shapes: Vec<(i32, i32, Shape)>,
    pub params: Params,
}

impl Component {
    fn from_params(p: &Params) -> Self {
        Self {
            lib_reference: p.string("LibReference"),
            description: p.string("ComponentDescription"),
            design_item_id: p.string("DesignItemId"),
            part_count: (p.i64("PartCount").unwrap_or(2) - 1).max(1) as u32,
            current_part_id: p.i32_or("CurrentPartId", 1),
            display_mode: p.i32_or("DisplayMode", 0),
            location: loc(p, "Location"),
            orientation: p.i64("Orientation").unwrap_or(0) as u8 & 3,
            mirrored: p.bool("IsMirrored"),
            unique_id: p.string("UniqueID"),
            params: p.clone(),
            ..Default::default()
        }
    }

    /// The footprint the component currently uses.
    pub fn footprint(&self) -> Option<&str> {
        let pcb = |i: &&Implementation| i.model_type.eq_ignore_ascii_case("PCBLIB");
        self.implementations
            .iter()
            .filter(pcb)
            .find(|i| i.is_current)
            .or_else(|| self.implementations.iter().find(pcb))
            .map(|i| i.model_name.as_str())
    }

    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .map(|p| p.text.text.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetEntry {
    pub name: String,
    /// 0 left, 1 right, 2 top, 3 bottom.
    pub side: u8,
    /// Offset from the symbol's top (or left) edge, in mils.
    pub distance: f64,
    pub io_type: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetSymbol {
    /// Top-left corner.
    pub location: Pt,
    pub size: Pt,
    pub name: String,
    pub file_name: String,
    pub entries: Vec<SheetEntry>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerPort {
    pub text: String,
    pub location: Pt,
    pub orientation: u8,
    pub style: i64,
    pub show_net_name: bool,
    /// An off-sheet connector: joins same-named nets across sheets.
    pub cross_sheet: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Port {
    pub name: String,
    pub location: Pt,
    pub width: f64,
    pub height: f64,
    /// 0 unspecified, 1 output, 2 input, 3 bidirectional.
    pub io_type: u8,
    /// Port style: 0..=3 horizontal shapes, 4..=7 vertical.
    pub style: i64,
}

/// Page setup from the `RECORD=31` sheet record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetSetup {
    pub style: i64,
    pub custom: Option<Pt>,
    pub fonts: HashMap<i64, f64>,
}

impl SheetSetup {
    /// Drawing size in mils.
    pub fn size(&self) -> Pt {
        if let Some(c) = self.custom {
            return c;
        }
        let (w, h) = match self.style {
            1 => (1550.0, 1110.0),
            2 => (2230.0, 1570.0),
            3 => (3150.0, 2230.0),
            4 => (4460.0, 3150.0),
            5 => (950.0, 750.0),
            6 => (1500.0, 950.0),
            7 => (2000.0, 1500.0),
            8 => (3200.0, 2000.0),
            9 => (4200.0, 3200.0),
            10 => (1100.0, 850.0),
            11 => (1400.0, 850.0),
            12 => (1700.0, 1100.0),
            _ => (1150.0, 760.0),
        };
        Pt::new(w * 10.0, h * 10.0)
    }

    /// Font size in points for a `FontID`.
    pub fn font_size(&self, id: i64) -> f64 {
        self.fonts.get(&id).copied().unwrap_or(10.0)
    }
}

/// One schematic document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchDoc {
    pub setup: SheetSetup,
    pub components: Vec<Component>,
    pub wires: Vec<Vec<Pt>>,
    pub buses: Vec<Vec<Pt>>,
    pub bus_entries: Vec<(Pt, Pt)>,
    pub junctions: Vec<Pt>,
    pub net_labels: Vec<Text>,
    pub power_ports: Vec<PowerPort>,
    pub ports: Vec<Port>,
    pub no_ercs: Vec<Pt>,
    pub sheet_symbols: Vec<SheetSymbol>,
    pub shapes: Vec<Shape>,
    /// Graphics owned by the sheet template (title block artwork).
    pub template_shapes: Vec<Shape>,
    pub parameters: Vec<Parameter>,
    /// Records this model does not lift, by `RECORD=` number.
    pub skipped: HashMap<i64, usize>,
}

impl SchDoc {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let mut c = Container::open(path)?;
        let stream = c.stream("FileHeader")?;
        Self::from_stream(&stream)
    }

    pub fn from_stream(stream: &[u8]) -> Result<Self> {
        let all = frames(stream, "FileHeader")?;
        // Frame 0 is the file header; `OwnerIndex` counts from frame 1.
        let records: Vec<Frame> = all.into_iter().skip(1).collect();
        let mut doc = SchDoc::default();
        // Owner index -> slot in `doc.components` / `doc.sheet_symbols`.
        let mut comp_slot: HashMap<usize, usize> = HashMap::new();
        let mut sheet_slot: HashMap<usize, usize> = HashMap::new();
        // Implementation lists sit between a component and its models.
        let mut impl_list_owner: HashMap<usize, usize> = HashMap::new();

        for (index, frame) in records.iter().enumerate() {
            let p = match frame {
                Frame::Text(p) => p,
                Frame::Binary(b) => {
                    // Binary pins are a library form; tolerate them anyway.
                    if let Ok(pin) = Pin::from_binary(b)
                        && let Some(c) = doc.components.last_mut()
                    {
                        c.pins.push(pin);
                    }
                    continue;
                }
            };
            let rec = p.i64("RECORD").unwrap_or(-1);
            let owner = p.i64("OwnerIndex").and_then(|o| usize::try_from(o).ok());
            let owner_comp = owner.and_then(|o| comp_slot.get(&o).copied());
            let part = p.i32_or("OwnerPartId", -1);
            let mode = p.i32_or("OwnerPartDisplayMode", 0);

            match rec {
                kind::COMPONENT => {
                    comp_slot.insert(index, doc.components.len());
                    doc.components.push(Component::from_params(p));
                }
                kind::PIN => {
                    if let Some(slot) = owner_comp {
                        doc.components[slot].pins.push(Pin::from_params(p));
                    }
                }
                kind::DESIGNATOR => {
                    if let Some(slot) = owner_comp {
                        doc.components[slot].designator = Some(Text::from_params(p));
                    }
                }
                kind::PARAMETER => {
                    let param = Parameter {
                        name: p.string("Name"),
                        text: Text::from_params(p),
                    };
                    match owner_comp {
                        Some(slot) => doc.components[slot].parameters.push(param),
                        None if owner.is_none() => doc.parameters.push(param),
                        None => {}
                    }
                }
                kind::IMPLEMENTATION_LIST => {
                    if let Some(slot) = owner_comp {
                        impl_list_owner.insert(index, slot);
                    }
                }
                kind::IMPLEMENTATION => {
                    if let Some(slot) = owner.and_then(|o| impl_list_owner.get(&o).copied()) {
                        doc.components[slot].implementations.push(Implementation {
                            model_name: p.string("ModelName"),
                            model_type: p.string("ModelType"),
                            is_current: p.bool("IsCurrent"),
                        });
                    }
                }
                kind::WIRE => doc.wires.push(points(p)),
                kind::BUS => doc.buses.push(points(p)),
                kind::BUS_ENTRY => doc.bus_entries.push((loc(p, "Location"), loc(p, "Corner"))),
                kind::JUNCTION => doc.junctions.push(loc(p, "Location")),
                kind::NET_LABEL => doc.net_labels.push(Text::from_params(p)),
                kind::NO_ERC => doc.no_ercs.push(loc(p, "Location")),
                kind::POWER_PORT => doc.power_ports.push(PowerPort {
                    text: p.string("Text"),
                    location: loc(p, "Location"),
                    orientation: p.i64("Orientation").unwrap_or(0) as u8 & 3,
                    style: p.i64("Style").unwrap_or(0),
                    show_net_name: p.bool("ShowNetName"),
                    cross_sheet: p.bool("IsCrossSheetConnector"),
                }),
                kind::PORT => doc.ports.push(Port {
                    name: p.string("Name"),
                    location: loc(p, "Location"),
                    width: p.sch_mils("Width"),
                    height: p.sch_mils("Height"),
                    io_type: p.i64("IOType").unwrap_or(0) as u8,
                    style: p.i64("Style").unwrap_or(0),
                }),
                kind::SHEET_SYMBOL => {
                    sheet_slot.insert(index, doc.sheet_symbols.len());
                    doc.sheet_symbols.push(SheetSymbol {
                        location: loc(p, "Location"),
                        size: Pt::new(p.sch_mils("XSize"), p.sch_mils("YSize")),
                        ..Default::default()
                    });
                }
                kind::SHEET_ENTRY | kind::SHEET_NAME | kind::SHEET_FILE_NAME => {
                    let Some(slot) = owner.and_then(|o| sheet_slot.get(&o).copied()) else {
                        continue;
                    };
                    let s = &mut doc.sheet_symbols[slot];
                    match rec {
                        kind::SHEET_ENTRY => s.entries.push(SheetEntry {
                            name: p.string("Name"),
                            side: p.i64("Side").unwrap_or(0) as u8 & 3,
                            // Stored in 100-mil steps plus a fraction.
                            distance: p.sch_mils("DistanceFromTop") * 10.0,
                            io_type: p.i64("IOType").unwrap_or(0) as u8,
                        }),
                        kind::SHEET_NAME => s.name = p.string("Text"),
                        _ => s.file_name = p.string("Text"),
                    }
                }
                kind::SHEET => doc.setup = sheet_setup(p),
                // Template link and model-map internals: metadata, not design.
                39 | 46 | 47 | 48 => {}
                _ => match shape(rec, p) {
                    Some(shape) => match (owner_comp, owner) {
                        (Some(slot), _) => doc.components[slot].shapes.push((part, mode, shape)),
                        (None, Some(_)) => doc.template_shapes.push(shape),
                        (None, None) => doc.shapes.push(shape),
                    },
                    None => *doc.skipped.entry(rec).or_default() += 1,
                },
            }
        }
        Ok(doc)
    }
}

/// One schematic library: a list of symbols.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchLib {
    pub components: Vec<Component>,
}

impl SchLib {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let mut c = Container::open(path)?;
        Self::from_container(&mut c)
    }

    pub fn from_container(c: &mut Container) -> Result<Self> {
        let header = frames(&c.stream("FileHeader")?, "FileHeader")?;
        let Some(Frame::Text(h)) = header.first() else {
            return Err(Error::Malformed(
                "SchLib FileHeader has no header record".into(),
            ));
        };
        let count = h.i64("CompCount").unwrap_or(0).max(0) as usize;
        let section_keys = section_keys(c)?;
        let storages = c.root_storages();
        let mut lib = SchLib::default();
        for i in 0..count {
            let libref = h.string(&format!("LibRef{i}"));
            let storage = section_keys
                .get(&libref)
                .cloned()
                .unwrap_or_else(|| storage_name(&libref));
            if !storages.iter().any(|s| s == &storage) {
                return Err(Error::MissingStream(format!("{storage}/Data")));
            }
            let data = c.stream(&format!("{storage}/Data"))?;
            let mut comp = parse_symbol_stream(&data)?;
            apply_pin_fracs(c, &storage, &mut comp)?;
            if comp.lib_reference.is_empty() {
                comp.lib_reference = libref;
            }
            lib.components.push(comp);
        }
        Ok(lib)
    }
}

/// Storage names are capped at 31 characters and cannot hold `/`.
fn storage_name(libref: &str) -> String {
    libref.replace('/', "_").chars().take(31).collect()
}

/// Long or unusual names map to their storage through `SectionKeys`.
fn section_keys(c: &mut Container) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let Some(raw) = c.stream_opt("SectionKeys")? else {
        return Ok(map);
    };
    for frame in frames(&raw, "SectionKeys")? {
        if let Frame::Text(p) = frame {
            let n = p.i64("KeyCount").unwrap_or(0).max(0);
            for i in 0..n {
                let lib = p.string(&format!("LibRef{i}"));
                let key = p.string(&format!("SectionKey{i}"));
                if !lib.is_empty() && !key.is_empty() {
                    map.insert(lib, key);
                }
            }
        }
    }
    Ok(map)
}

/// A library symbol's `Data` stream: the component record, then its
/// children with no `OwnerIndex`.
pub fn parse_symbol_stream(data: &[u8]) -> Result<Component> {
    let mut comp: Option<Component> = None;
    let mut in_impl_list = false;
    for frame in frames(data, "symbol Data")? {
        match frame {
            Frame::Binary(b) => {
                let pin = Pin::from_binary(&b)?;
                if let Some(c) = comp.as_mut() {
                    c.pins.push(pin);
                }
            }
            Frame::Text(p) => {
                let rec = p.i64("RECORD").unwrap_or(-1);
                if rec == kind::COMPONENT {
                    comp = Some(Component::from_params(&p));
                    continue;
                }
                let Some(c) = comp.as_mut() else { continue };
                let part = p.i32_or("OwnerPartId", -1);
                let mode = p.i32_or("OwnerPartDisplayMode", 0);
                match rec {
                    kind::PIN => c.pins.push(Pin::from_params(&p)),
                    kind::DESIGNATOR => c.designator = Some(Text::from_params(&p)),
                    kind::PARAMETER => c.parameters.push(Parameter {
                        name: p.string("Name"),
                        text: Text::from_params(&p),
                    }),
                    kind::IMPLEMENTATION_LIST => in_impl_list = true,
                    kind::IMPLEMENTATION if in_impl_list => {
                        c.implementations.push(Implementation {
                            model_name: p.string("ModelName"),
                            model_type: p.string("ModelType"),
                            is_current: p.bool("IsCurrent"),
                        })
                    }
                    _ => {
                        if let Some(s) = shape(rec, &p) {
                            c.shapes.push((part, mode, s));
                        }
                    }
                }
            }
        }
    }
    comp.ok_or_else(|| Error::Malformed("symbol stream has no component record".into()))
}

/// `PinFrac` adds sub-10-mil precision to binary pins: per pin index, a
/// zlib block of three `i32` fractions (x, y, length) in 1/100000 units.
fn apply_pin_fracs(c: &mut Container, storage: &str, comp: &mut Component) -> Result<()> {
    let Some(raw) = c.stream_opt(&format!("{storage}/PinFrac"))? else {
        return Ok(());
    };
    for frame in frames(&raw, "PinFrac")? {
        let Frame::Binary(b) = frame else { continue };
        let mut r = Reader::new(&b, "PinFrac");
        let Ok(_) = r.u8("tag") else { continue };
        let Ok(index) = r.pascal_str("pin index") else {
            continue;
        };
        let Ok(block) = r.block("frac data") else {
            continue;
        };
        let Ok(index) = index.trim().parse::<usize>() else {
            continue;
        };
        let Some(raw) = inflate(block) else { continue };
        let mut fr = Reader::new(&raw, "PinFrac data");
        let (Ok(fx), Ok(fy), Ok(fl)) = (fr.i32("x"), fr.i32("y"), fr.i32("len")) else {
            continue;
        };
        if let Some(pin) = comp.pins.get_mut(index) {
            let unit = |f: i32| f64::from(f) / 100_000.0 * 10.0;
            pin.location.x += unit(fx);
            pin.location.y += unit(fy);
            pin.length += unit(fl);
        }
    }
    Ok(())
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

fn sheet_setup(p: &Params) -> SheetSetup {
    let n = p.i64("FontIdCount").unwrap_or(0);
    let fonts = (1..=n)
        .map(|i| (i, p.f64_or(&format!("Size{i}"), 10.0)))
        .collect();
    let custom = p.bool("UseCustomSheet").then(|| {
        Pt::new(
            p.i64("CustomX").unwrap_or(1150) as f64 * 10.0,
            p.i64("CustomY").unwrap_or(760) as f64 * 10.0,
        )
    });
    SheetSetup {
        style: p.i64("SheetStyle").unwrap_or(0),
        custom,
        fonts,
    }
}

fn loc(p: &Params, base: &str) -> Pt {
    Pt::new(
        p.sch_mils(&format!("{base}.X")),
        p.sch_mils(&format!("{base}.Y")),
    )
}

fn points(p: &Params) -> Vec<Pt> {
    let n = p.i64("LocationCount").unwrap_or(0).max(0);
    (1..=n)
        .map(|i| Pt::new(p.sch_mils(&format!("X{i}")), p.sch_mils(&format!("Y{i}"))))
        .collect()
}

/// Line width codes: smallest, small, medium, large.
fn line_width(p: &Params) -> f64 {
    match p.i64("LineWidth").unwrap_or(0) {
        0 => 0.0,
        1 => 10.0,
        2 => 30.0,
        _ => 50.0,
    }
}

fn shape(rec: i64, p: &Params) -> Option<Shape> {
    let w = line_width(p);
    let filled = p.bool("IsSolid");
    Some(match rec {
        kind::LINE => Shape::Line {
            a: loc(p, "Location"),
            b: loc(p, "Corner"),
            width: w,
        },
        kind::POLYLINE => Shape::Polyline {
            points: points(p),
            width: w,
        },
        kind::POLYGON => Shape::Polygon {
            points: points(p),
            width: w,
            filled,
        },
        kind::RECTANGLE => Shape::Rectangle {
            a: loc(p, "Location"),
            b: loc(p, "Corner"),
            width: w,
            filled,
        },
        kind::ROUND_RECT => Shape::RoundRect {
            a: loc(p, "Location"),
            b: loc(p, "Corner"),
            rx: p.sch_mils("CornerXRadius"),
            ry: p.sch_mils("CornerYRadius"),
            width: w,
            filled,
        },
        kind::ARC | kind::ELLIPTICAL_ARC | kind::PIE => Shape::Arc {
            center: loc(p, "Location"),
            radius: p.sch_mils("Radius"),
            start: p.f64_or("StartAngle", 0.0),
            end: p.f64_or("EndAngle", 360.0),
            width: w,
        },
        kind::ELLIPSE => {
            let rx = p.sch_mils("Radius");
            let ry = if p.get("SecondaryRadius").is_some() {
                p.sch_mils("SecondaryRadius")
            } else {
                rx
            };
            Shape::Ellipse {
                center: loc(p, "Location"),
                rx,
                ry,
                width: w,
                filled,
            }
        }
        kind::BEZIER => Shape::Bezier {
            points: points(p),
            width: w,
        },
        kind::LABEL => Shape::Label(Text::from_params(p)),
        kind::TEXT_FRAME => Shape::TextFrame {
            a: loc(p, "Location"),
            b: loc(p, "Corner"),
            text: p.string("Text").replace("~1", "\n"),
            filled,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_pin_decodes() {
        // Pin "2" of a resistor: 270 degrees, length 50 mil, at (0, -250).
        let raw: Vec<u8> = [
            0x02, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x04, 0x3b, 0x05, 0, 0, 0, 0xe7,
            0xff, 0, 0, 0, 0, 0x01, b'2', 0x01, b'2', 0, 0, 0,
        ]
        .to_vec();
        let pin = Pin::from_binary(&raw).unwrap();
        assert_eq!(pin.designator, "2");
        assert_eq!(pin.rotation, 3);
        assert_eq!(pin.length, 50.0);
        assert_eq!(pin.location, Pt::new(0.0, -250.0));
        assert_eq!(pin.electrical, Electrical::Passive);
        assert!(pin.show_name && pin.show_designator && !pin.hidden);
        assert_eq!(pin.hot_end(), Pt::new(0.0, -300.0));
    }

    #[test]
    fn justification_grid() {
        assert_eq!(Justification(0).horizontal(), -1);
        assert_eq!(Justification(0).vertical(), -1);
        assert_eq!(Justification(4).horizontal(), 0);
        assert_eq!(Justification(8).vertical(), 1);
    }
}
