//! Unit and frame conversion helpers.

pub const MM_PER_MIL: f64 = 0.0254;

/// Altium schematic font points to mm: 10 pt is 1.27 mm.
pub const MM_PER_PT: f64 = signex_types::schematic::SCHEMATIC_PT_TO_MM;

pub fn mm(mils: f64) -> f64 {
    // Round to 10 nm. Altium's own unit is 1/10000 mil (2.54 nm), so a
    // board drawn in mm stores 60 mm as 59.99999938 mm; rounding at 10 nm
    // gives the designed value back while staying far below anything a
    // fab resolves. It also keeps 100-mil grid points exact.
    (mils * MM_PER_MIL * 1e5).round() / 1e5
}

/// Normalise an angle to [0, 360).
pub fn deg(a: f64) -> f64 {
    let r = a.rem_euclid(360.0);
    if (r - 360.0).abs() < 1e-9 { 0.0 } else { r }
}
