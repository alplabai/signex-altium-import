//! A footprint's 3D model: the body record names a model of the library's
//! model table, whose inflated bytes become the footprint's STEP attachment
//! and its embedded `[[models]]` entry (`LIBRARY_FORMAT.md` section 8).
//!
//! Placement. Signex draws a model in its own y-up frame: it turns the model
//! about x, then y, then z (degrees, counter-clockwise), adds the offset
//! (mm), and only then flips y into the footprint's y-down frame
//! (`signex-renderer`, `model_point_on_footprint`). Altium's placement is
//! already in a y-up frame, so offsets and angles carry over without
//! negation: offset = (`MODEL.2D.X`, `MODEL.2D.Y`, `MODEL.3D.DZ`) in mm,
//! rotation = (`ROTX`, `ROTY`, `ROTZ` + `MODEL.2D.ROTATION`). That the turn
//! order matches Altium's is not verified against a 3D view; a body with a
//! 2D rotation, or on the bottom side, is reported as approximated.

use altium_format::pcb::{Body, Footprint as SrcFootprint, LibModel, PcbLib};
use serde_json::json;

use crate::report::{Item, ItemKind, Report, Severity};
use crate::units::MM_PER_MIL;

/// A model ready to attach.
pub struct Resolved {
    pub filename: String,
    pub bytes: Vec<u8>,
    pub offset_xyz: [f64; 3],
    pub rotation_xyz: [f64; 3],
}

fn round6(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

fn item(code: &'static str, sev: Severity, source: &str, message: String) -> Item {
    Item {
        kind: ItemKind::Model,
        source: source.to_string(),
        id: None,
        code,
        severity: sev,
        message,
        detail: serde_json::Value::Null,
    }
}

/// A STEP file starts with its ISO 10303-21 header.
fn is_step(bytes: &[u8]) -> bool {
    let head: Vec<u8> = bytes.iter().take(256).copied().collect();
    String::from_utf8_lossy(&head)
        .trim_start()
        .starts_with("ISO-10303-21")
}

fn find<'a>(lib: &'a PcbLib, id: &str) -> Option<&'a LibModel> {
    lib.models.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// Counters a footprint's model handling adds to.
#[derive(Default)]
pub struct ModelCounts {
    pub source: usize,
    pub written: usize,
}

/// The model of `src`, if it has one that can be attached. Every model
/// placement the footprint has is accounted for: counted as a source model
/// and either written (the return value) or reported with severity `error`.
/// With `include` false the models are skipped and reported as `info`, and
/// are not counted.
pub fn resolve(
    src: &SrcFootprint,
    lib: &PcbLib,
    include: bool,
    source: &str,
    report: &mut Report,
    counts: &mut ModelCounts,
) -> Option<Resolved> {
    let (with, without): (Vec<&Body>, Vec<&Body>) = src
        .primitives
        .bodies
        .iter()
        .partition(|b| !b.params.string("MODELID").trim().is_empty());
    if !without.is_empty() {
        report.push(item(
            "model.body_without_model",
            Severity::Info,
            source,
            format!("{} body without a 3D model not imported", without.len()),
        ));
    }
    if with.is_empty() {
        return None;
    }
    if !include {
        for b in &with {
            report.push(item(
                "model.skipped",
                Severity::Info,
                source,
                format!(
                    "model {} skipped (--models skip)",
                    b.params.string("MODEL.NAME")
                ),
            ));
        }
        return None;
    }
    counts.source += with.len();
    for extra in with.iter().skip(1) {
        report.push(item(
            "model.extra_body",
            Severity::Error,
            source,
            format!(
                "a footprint holds one model; {} not imported",
                extra.params.string("MODEL.NAME")
            ),
        ));
    }
    let body = with[0];
    let id = body.params.string("MODELID");
    let name = body.params.string("MODEL.NAME");
    let fail = |code: &'static str, msg: String, report: &mut Report| {
        report.push(item(code, Severity::Error, source, msg));
        None
    };
    // A body that does not say its model is embedded names a model kept
    // outside the file (observed in a private corpus: integrated libraries
    // whose bodies name 3D models stored nowhere in the file).
    if !body
        .params
        .string("MODEL.EMBED")
        .eq_ignore_ascii_case("TRUE")
        && find(lib, &id).is_none()
    {
        return fail(
            "model.external",
            format!("model {name} is not stored in the library"),
            report,
        );
    }
    let Some(m) = find(lib, &id) else {
        return fail(
            "model.missing",
            format!("model {name} is not in the library's model table"),
            report,
        );
    };
    let Some(bytes) = m.data.as_ref() else {
        let (code, why) = match &m.problem {
            Some(p) => ("model.unreadable", p.clone()),
            None => (
                "model.external",
                "the model is not embedded in the library".to_string(),
            ),
        };
        return fail(code, format!("model {name} not imported: {why}"), report);
    };
    if !is_step(bytes) {
        return fail(
            "model.not_step",
            format!("model {name} is not a STEP file; not imported"),
            report,
        );
    }
    let mil = |key: &str| body.params.pcb_mils(key).unwrap_or(0.0) * MM_PER_MIL;
    let deg = |key: &str| body.params.f64(key).unwrap_or(0.0);
    let rot2d = deg("MODEL.2D.ROTATION");
    let bottom = body.params.i64("BODYPROJECTION").unwrap_or(0) == 1;
    let turned = ['X', 'Y', 'Z']
        .iter()
        .filter(|a| deg(&format!("MODEL.3D.ROT{a}")) != 0.0)
        .count();
    if rot2d != 0.0 || bottom || turned >= 2 {
        report.push(Item {
            kind: ItemKind::Model,
            source: source.to_string(),
            id: None,
            code: "model.placement_approximated",
            severity: Severity::Approximated,
            message: "the turn order of several rotations is not verified, a 2D rotation is added to the z turn, and a bottom-side projection is not applied".into(),
            detail: json!({ "rotation_2d": rot2d, "bottom": bottom, "axes_turned": turned }),
        });
    }
    let base = name.rsplit(['/', '\\']).next().unwrap_or(&name);
    let filename = crate::text::cell_safe(if base.is_empty() { "model.step" } else { base }).0;
    counts.written += 1;
    Some(Resolved {
        filename,
        bytes: bytes.clone(),
        offset_xyz: [
            round6(mil("MODEL.2D.X")),
            round6(mil("MODEL.2D.Y")),
            round6(mil("MODEL.3D.DZ")),
        ],
        rotation_xyz: [
            round6(deg("MODEL.3D.ROTX")),
            round6(deg("MODEL.3D.ROTY")),
            round6(deg("MODEL.3D.ROTZ") + rot2d),
        ],
    })
}
