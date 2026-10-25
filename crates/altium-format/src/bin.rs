//! Bounds-checked little-endian reader for the binary records.

use crate::error::{Error, Result};

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    stream: &'a str,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], stream: &'a str) -> Self {
        Self {
            buf,
            pos: 0,
            stream,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::Truncated {
                stream: self.stream.to_string(),
                offset: self.pos,
                what,
            });
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize, what: &'static str) -> Result<()> {
        self.take(n, what).map(|_| ())
    }

    pub fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    pub fn u16(&mut self, what: &'static str) -> Result<u16> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self, what: &'static str) -> Result<i16> {
        Ok(self.u16(what)? as i16)
    }

    pub fn u32(&mut self, what: &'static str) -> Result<u32> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self, what: &'static str) -> Result<i32> {
        Ok(self.u32(what)? as i32)
    }

    pub fn f64(&mut self, what: &'static str) -> Result<f64> {
        let b = self.take(8, what)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_le_bytes(a))
    }

    /// An `f64` with NaN / infinity read as 0. For angles and rotations,
    /// where a corrupt value must not reach the writers.
    pub fn f64_finite(&mut self, what: &'static str) -> Result<f64> {
        let v = self.f64(what)?;
        Ok(if v.is_finite() { v } else { 0.0 })
    }

    /// A `u32` length followed by that many bytes.
    pub fn block(&mut self, what: &'static str) -> Result<&'a [u8]> {
        let n = self.u32(what)? as usize;
        self.take(n, what)
    }

    /// A `u8` length followed by that many ANSI bytes.
    pub fn pascal_str(&mut self, what: &'static str) -> Result<String> {
        let n = self.u8(what)? as usize;
        Ok(crate::params::decode_ansi(self.take(n, what)?))
    }
}
