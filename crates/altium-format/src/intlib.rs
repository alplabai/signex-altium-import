//! Integrated libraries (`.IntLib`): a compound file that embeds a schematic
//! library and a footprint library, each as a compressed compound file of its
//! own. Layout notes (observed in a private corpus file): `docs/FORMAT_NOTES.md`.

use crate::container::Container;
use crate::error::{Error, Result};
use crate::limits::Budget;
use crate::pcb::PcbLib;
use crate::sch::SchLib;

/// Inflate an embedded stream: a type byte, then a zlib stream. Limits:
/// `limits.rs`.
pub fn inflate_embedded(stream: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    inflate_embedded_with(&mut Budget::default(), stream, bytes)
}

/// As [`inflate_embedded`], charging a run-wide budget.
pub fn inflate_embedded_with(budget: &mut Budget, stream: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let Some((&kind, body)) = bytes.split_first() else {
        return Err(Error::Truncated {
            stream: stream.to_string(),
            offset: 0,
            what: "embedded stream type byte",
        });
    };
    if kind != 2 {
        return Err(Error::Malformed(format!(
            "embedded stream `{stream}` has type {kind}, expected 2 (zlib)"
        )));
    }
    budget.inflate_zlib(stream, body)
}

/// One embedded library, with the stream it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded<T> {
    pub stream: String,
    pub lib: T,
}

/// An opened integrated library.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IntLib {
    pub schlibs: Vec<Embedded<SchLib>>,
    pub pcblibs: Vec<Embedded<PcbLib>>,
    /// The inflated cross-reference stream, when the file has one, for
    /// cross-checks only.
    pub cross_ref: Option<Vec<u8>>,
    /// The container's root-entry modified time, if it records one.
    pub modified: Option<std::time::SystemTime>,
}

impl IntLib {
    /// Cross-check against the cross-reference stream: the number of
    /// entries it announces (its first four bytes) against the components
    /// of the embedded schematic libraries, and each component name
    /// against the stream's text. Returns what disagrees, empty when all
    /// agree or the file has no such stream. The stream is never a source
    /// of bindings.
    pub fn cross_ref_mismatches(&self) -> Vec<String> {
        let Some(x) = &self.cross_ref else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let comps: Vec<&str> = self
            .schlibs
            .iter()
            .flat_map(|s| s.lib.components.iter().map(|c| c.lib_reference.as_str()))
            .collect();
        if let Some(head) = x.first_chunk::<4>() {
            let n = u32::from_le_bytes(*head) as usize;
            if n != comps.len() {
                out.push(format!(
                    "cross-reference lists {n} entries, the schematic library has {} components",
                    comps.len()
                ));
            }
        }
        for name in comps {
            let b = name.as_bytes();
            if !b.is_empty() && !x.windows(b.len()).any(|w| w == b) {
                out.push(format!("component {name:?} is not in the cross-reference"));
            }
        }
        out
    }
}

fn embedded_names(c: &Container, storage: &str, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = c
        .streams_in(storage)
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(ext))
        .map(|n| format!("{storage}/{n}"))
        .collect();
    v.sort();
    v
}

impl IntLib {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let mut c = Container::open(path)?;
        Self::from_container(&mut c)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let mut c = Container::from_bytes(std::path::PathBuf::from("<intlib>"), bytes)?;
        Self::from_container(&mut c)
    }

    pub fn from_container(c: &mut Container) -> Result<Self> {
        let mut lib = IntLib {
            modified: c.modified(),
            ..Default::default()
        };
        let sch = embedded_names(c, "SchLib", ".schlib");
        let pcb = embedded_names(c, "PCBLib", ".pcblib");
        if sch.is_empty() && pcb.is_empty() {
            return Err(Error::MissingStream(
                "SchLib/*.schlib or PCBLib/*.pcblib".into(),
            ));
        }
        let mut budget = Budget::default();
        let open = |c: &mut Container, budget: &mut Budget, name: &str| -> Result<Container> {
            let raw = c.stream(name)?;
            let inner = inflate_embedded_with(budget, name, &raw)?;
            Container::from_bytes(std::path::PathBuf::from(name), inner)
        };
        for name in sch {
            let mut inner = open(c, &mut budget, &name)?;
            let l = SchLib::from_container(&mut inner)?;
            lib.schlibs.push(Embedded {
                stream: name,
                lib: l,
            });
        }
        for name in pcb {
            let mut inner = open(c, &mut budget, &name)?;
            let l = PcbLib::from_container_with(&mut inner, &mut budget)?;
            lib.pcblibs.push(Embedded {
                stream: name,
                lib: l,
            });
        }
        // Names of the cross-reference stream vary in case.
        let xref = c
            .streams_in("/")
            .into_iter()
            .find(|n| n.eq_ignore_ascii_case("LibCrossRef.Txt"));
        if let Some(n) = xref
            && let Ok(raw) = c.stream(&n)
        {
            lib.cross_ref = inflate_embedded_with(&mut budget, &n, &raw).ok();
        }
        Ok(lib)
    }
}

/// Build an integrated library from the bytes of a schematic library and a
/// footprint library (and, optionally, a cross-reference payload). Only for
/// tests and fixtures: it writes the layout the reader expects.
pub fn pack(
    sch: Option<&[u8]>,
    pcb: Option<&[u8]>,
    cross_ref: Option<&[u8]>,
) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let deflate = |b: &[u8]| -> std::io::Result<Vec<u8>> {
        let mut z = flate2::write::ZlibEncoder::new(vec![2u8], flate2::Compression::default());
        z.write_all(b)?;
        z.finish()
    };
    let mut c = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new()))?;
    let put = |c: &mut cfb::CompoundFile<std::io::Cursor<Vec<u8>>>, path: &str, bytes: &[u8]| {
        c.create_stream(path).and_then(|mut s| s.write_all(bytes))
    };
    if let Some(b) = sch {
        c.create_storage("/SchLib")?;
        put(&mut c, "/SchLib/0.schlib", &deflate(b)?)?;
    }
    if let Some(b) = pcb {
        c.create_storage("/PCBLib")?;
        put(&mut c, "/PCBLib/0.pcblib", &deflate(b)?)?;
    }
    if let Some(b) = cross_ref {
        put(&mut c, "/LibCrossRef.Txt", &deflate(b)?)?;
    }
    put(&mut c, "/Version.Txt", &[0, 2, 0, 0, 0])?;
    c.flush()?;
    Ok(c.into_inner().into_inner())
}
