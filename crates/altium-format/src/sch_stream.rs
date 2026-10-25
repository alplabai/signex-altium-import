//! Framing of schematic record streams (`FileHeader`, `<Symbol>/Data`).
//!
//! Each frame is a little-endian `u32`: the low 24 bits are the payload
//! length, the high byte is the payload kind. Kind 0 is a `|KEY=VALUE|`
//! property list; kind 1 is a packed binary record (pins in libraries).

use crate::bin::Reader;
use crate::error::Result;
use crate::params::Params;

#[derive(Debug, Clone)]
pub enum Frame {
    Text(Params),
    Binary(Vec<u8>),
}

pub fn frames(stream: &[u8], name: &str) -> Result<Vec<Frame>> {
    let mut r = Reader::new(stream, name);
    let mut out = Vec::new();
    while r.remaining() >= 4 {
        let header = r.u32("frame header")?;
        let len = (header & 0x00FF_FFFF) as usize;
        let kind = (header >> 24) as u8;
        let payload = r.take(len, "frame payload")?;
        out.push(match kind {
            0 => Frame::Text(Params::parse(payload)),
            _ => Frame::Binary(payload.to_vec()),
        });
    }
    Ok(out)
}
