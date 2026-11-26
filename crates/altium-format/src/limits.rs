//! Limits on inflating untrusted compressed streams.
//!
//! A compressed stream may not inflate to more than [`MAX_RATIO`] times its
//! own size (with a [`RATIO_FLOOR`] allowance for small streams), and one run
//! may inflate at most [`RUN_BUDGET`] bytes in all (every embedded library
//! and model together).

use std::io::Read;

use crate::error::{Error, Result};

/// Inflated bytes one run may produce in total: 2 GiB.
pub const RUN_BUDGET: u64 = 2 * 1024 * 1024 * 1024;
/// Inflated size over compressed size beyond which a stream is refused.
pub const MAX_RATIO: u64 = 200;
/// Streams may always inflate to this much, whatever their ratio.
pub const RATIO_FLOOR: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Budget {
    remaining: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            remaining: RUN_BUDGET,
        }
    }
}

impl Budget {
    pub fn with_limit(bytes: u64) -> Self {
        Budget { remaining: bytes }
    }

    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Inflate a zlib stream, charging the result to the budget.
    pub fn inflate_zlib(&mut self, what: &str, data: &[u8]) -> Result<Vec<u8>> {
        let ratio_limit = (data.len() as u64)
            .saturating_mul(MAX_RATIO)
            .max(RATIO_FLOOR);
        let limit = ratio_limit.min(self.remaining);
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(data)
            .take(limit + 1)
            .read_to_end(&mut out)
            .map_err(|e| Error::Malformed(format!("`{what}` does not inflate: {e}")))?;
        if out.len() as u64 > limit {
            let why = if limit == self.remaining && self.remaining < ratio_limit {
                "the run's inflate budget is spent"
            } else {
                "it inflates past the allowed ratio"
            };
            return Err(Error::Malformed(format!("`{what}` refused: {why}")));
        }
        self.remaining -= out.len() as u64;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn z(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn ordinary_streams_inflate_and_are_charged() {
        let mut b = Budget::default();
        let data: Vec<u8> = (0..10_000u32).map(|i| (i * 31) as u8).collect();
        assert_eq!(b.inflate_zlib("x", &z(&data)).unwrap(), data);
        assert_eq!(b.remaining(), RUN_BUDGET - 10_000);
    }

    #[test]
    fn a_bomb_is_refused_by_ratio_and_by_budget() {
        let zeros = vec![0u8; 50 * 1024 * 1024];
        assert!(Budget::default().inflate_zlib("bomb", &z(&zeros)).is_err());
        let data = vec![7u8, 1, 9, 3].repeat(200);
        let mut tiny = Budget::with_limit(100);
        assert!(tiny.inflate_zlib("x", &z(&data)).is_err());
    }
}
