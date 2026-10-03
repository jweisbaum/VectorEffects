//! Bounds-checked little-endian reads over the file's bytes. Every read
//! that would pass the end is an error, so a damaged length or address is
//! reported rather than indexed with.

use crate::{Error, Result};

/// The widths the superblock fixes for the whole file, and the base every
/// stored address is relative to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Format {
    /// Bytes in an address ("size of offsets").
    pub offsets: u8,
    /// Bytes in a length ("size of lengths").
    pub lengths: u8,
    /// The absolute position of address 0: the superblock's base address.
    pub base: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct Cursor<'a> {
    buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// A cursor at an absolute position in `buf`.
    pub fn at(buf: &'a [u8], pos: u64) -> Result<Self> {
        let pos = usize::try_from(pos).map_err(|_| Error::Truncated { at: pos })?;
        if pos > buf.len() {
            return Err(Error::Truncated { at: pos as u64 });
        }
        Ok(Self { buf, pos })
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.buf.len())
            .ok_or(Error::Truncated {
                at: self.pos as u64,
            })?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(self.uint(2)? as u16)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(self.uint(4)? as u32)
    }

    /// An unsigned little-endian integer of `n` bytes, `n` at most 8.
    pub fn uint(&mut self, n: usize) -> Result<u64> {
        if n > 8 {
            return Err(Error::Corrupt(format!("a {n}-byte integer")));
        }
        Ok(self
            .bytes(n)?
            .iter()
            .rev()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
    }

    pub fn length(&mut self, f: Format) -> Result<u64> {
        self.uint(usize::from(f.lengths))
    }

    /// An address, absolute; `None` for the undefined address (all bits set).
    pub fn address(&mut self, f: Format) -> Result<Option<u64>> {
        let width = usize::from(f.offsets);
        let raw = self.uint(width)?;
        let undefined = if width == 8 {
            u64::MAX
        } else {
            (1u64 << (width * 8)) - 1
        };
        if raw == undefined {
            return Ok(None);
        }
        raw.checked_add(f.base)
            .map(Some)
            .ok_or_else(|| Error::Corrupt("an address past the end of the address space".into()))
    }

    /// An address that must be defined.
    pub fn required_address(&mut self, f: Format, what: &str) -> Result<u64> {
        self.address(f)?
            .ok_or_else(|| Error::Corrupt(format!("{what} has no address")))
    }

    pub fn signature(&mut self, expected: &[u8; 4]) -> Result<()> {
        let at = self.pos as u64;
        if self.bytes(4)? != expected {
            return Err(Error::Corrupt(format!(
                "expected {} at {at}",
                String::from_utf8_lossy(expected)
            )));
        }
        Ok(())
    }

    /// Pads the position to a multiple of `to` counted from `from`.
    pub fn align(&mut self, from: usize, to: usize) -> Result<()> {
        let misalign = (self.pos - from) % to;
        if misalign != 0 {
            self.skip(to - misalign)?;
        }
        Ok(())
    }
}

/// The bytes an integer up to `max` needs, as the library encodes counts
/// whose width depends on the structure (`H5VM_limit_enc_size`).
pub(crate) fn limit_enc_size(max: u64) -> usize {
    (max.max(1).ilog2() / 8 + 1) as usize
}
