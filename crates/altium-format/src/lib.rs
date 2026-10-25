//! Clean-room reader for Altium Designer's binary design files.
//!
//! Layering, bottom up:
//!
//! - [`container`] — the OLE2 compound-file wrapper every binary file uses.
//! - [`params`] / [`bin`] / [`sch_stream`] — record framing and decoding.
//! - [`sch`] — `.SchDoc` / `.SchLib` into a typed record model.
//! - [`pcb`] — `.PcbDoc` / `.PcbLib` into a typed primitive model.
//! - [`prj`] — `.PrjPcb` project files (INI text).
//!
//! Nothing here knows about Signex; the conversion lives in
//! `signex-altium-convert`. Format notes and provenance: `docs/FORMAT_NOTES.md`.

pub mod bin;
pub mod container;
pub mod error;
pub mod params;
pub mod pcb;
pub mod prj;
pub mod sch;
pub mod sch_stream;

pub use container::Container;
pub use error::{Error, Result};
pub use params::Params;
