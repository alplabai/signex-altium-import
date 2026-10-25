//! Map the `altium-format` model onto Signex native types.
//!
//! - [`sch`]: `SchDoc` -> `SchematicSheet` (symbols become embedded
//!   `lib_symbols`, one per distinct Altium library reference).
//! - [`pcb`]: `PcbDoc` -> `PcbBoard`.
//! - [`Report`]: what was approximated or dropped, so the CLI can say so.
//!
//! Conventions this crate bridges are written up in
//! `docs/CONVERSION.md`.

pub mod library;
pub mod pcb;
pub mod report;
pub mod sch;
mod units;

pub use report::Report;
