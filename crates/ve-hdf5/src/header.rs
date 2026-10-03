//! Object headers: the list of messages that says what an object is.
//!
//! Section references are to the HDF5 File Format Specification, version
//! 3.0: IV.A.1.a (version 1 headers) and IV.A.1.b (version 2).

use crate::cursor::{Cursor, Format};
use crate::{Error, Result};

/// One header message, its body borrowed from the file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Message<'a> {
    pub kind: u16,
    pub flags: u8,
    pub data: &'a [u8],
}

/// Message types this reader acts on (IV.A.2).
pub(crate) mod kind {
    pub const DATASPACE: u16 = 0x01;
    pub const LINK_INFO: u16 = 0x02;
    pub const DATATYPE: u16 = 0x03;
    pub const FILL_OLD: u16 = 0x04;
    pub const FILL: u16 = 0x05;
    pub const LINK: u16 = 0x06;
    pub const LAYOUT: u16 = 0x08;
    pub const FILTERS: u16 = 0x0B;
    pub const ATTRIBUTE: u16 = 0x0C;
    pub const CONTINUATION: u16 = 0x10;
    pub const SYMBOL_TABLE: u16 = 0x11;
    pub const ATTRIBUTE_INFO: u16 = 0x15;
}

/// Bit 1 of a message's flags: the body is a reference to a message kept
/// elsewhere (IV.A.2, "shared"). NetCDF-4 never writes one.
pub(crate) const SHARED: u8 = 0x02;

/// No object has more continuation blocks than this; a damaged header that
/// chains them in a loop stops here.
const MAX_BLOCKS: usize = 4096;

/// Every message of the object header at `address`, continuations followed.
pub(crate) fn messages(file: &[u8], f: Format, address: u64) -> Result<Vec<Message<'_>>> {
    let mut c = Cursor::at(file, address)?;
    if file.get(c.pos..c.pos + 4) == Some(b"OHDR") {
        version_2(file, f, c)
    } else {
        let version = c.u8()?;
        if version != 1 {
            return Err(Error::Corrupt(format!(
                "no object header at {address} (version {version})"
            )));
        }
        version_1(file, f, c)
    }
}

/// IV.A.1.a: version (1), reserved (1), message count (2), reference count
/// (4), header size (4), padding to 8 — then messages, each type (2), size
/// (2), flags (1), reserved (3), body, all aligned to 8 bytes.
fn version_1<'a>(file: &'a [u8], f: Format, mut c: Cursor<'a>) -> Result<Vec<Message<'a>>> {
    c.skip(1)?;
    let _count = c.u16()?;
    c.skip(4)?;
    let size = c.u32()?;
    c.skip(4)?;
    let mut blocks = vec![(c.pos as u64, u64::from(size))];
    let mut out = Vec::new();
    let mut next = 0;
    while next < blocks.len() {
        let (start, len) = blocks[next];
        next += 1;
        let mut c = Cursor::at(file, start)?;
        let block = c.bytes(to_usize(len)?)?;
        let mut m = Cursor::new(block);
        while m.remaining() >= 8 {
            let kind = m.u16()?;
            let size = m.u16()?;
            let flags = m.u8()?;
            m.skip(3)?;
            let data = m.bytes(usize::from(size))?;
            push(&mut out, &mut blocks, f, Message { kind, flags, data })?;
        }
    }
    Ok(out)
}

/// IV.A.1.b: "OHDR", version (1), flags (1), the optional times and
/// attribute phase-change values, the first chunk's size (1, 2, 4 or 8
/// bytes by flags bits 0–1), the messages, a checksum. A message is type
/// (1), size (2), flags (1), creation order (2, if flags bit 2), body.
/// Continuation blocks are "OCHK", messages, checksum.
fn version_2<'a>(file: &'a [u8], f: Format, mut c: Cursor<'a>) -> Result<Vec<Message<'a>>> {
    c.signature(b"OHDR")?;
    let version = c.u8()?;
    if version != 2 {
        return Err(Error::Unsupported(format!(
            "object header version {version}"
        )));
    }
    let flags = c.u8()?;
    if flags & 0x20 != 0 {
        c.skip(16)?;
    }
    if flags & 0x10 != 0 {
        c.skip(4)?;
    }
    let size = c.uint(1 << (flags & 0x03))?;
    let ordered = flags & 0x04 != 0;
    let mut blocks = vec![(c.pos as u64, size)];
    let mut out = Vec::new();
    let mut next = 0;
    while next < blocks.len() {
        let (start, len) = blocks[next];
        let first = next == 0;
        next += 1;
        let mut c = Cursor::at(file, start)?;
        let len = if first {
            to_usize(len)?
        } else {
            c.signature(b"OCHK")?;
            // The stated length covers the signature and the checksum.
            to_usize(len)?.checked_sub(8).ok_or_else(|| {
                Error::Corrupt("a continuation block shorter than its frame".into())
            })?
        };
        let block = c.bytes(len)?;
        let mut m = Cursor::new(block);
        let header = if ordered { 6 } else { 4 };
        // Anything shorter than a message header at the end is the gap.
        while m.remaining() >= header {
            let kind = u16::from(m.u8()?);
            let size = m.u16()?;
            let flags = m.u8()?;
            if ordered {
                m.skip(2)?;
            }
            let data = m.bytes(usize::from(size))?;
            push(&mut out, &mut blocks, f, Message { kind, flags, data })?;
        }
    }
    Ok(out)
}

fn push<'a>(
    out: &mut Vec<Message<'a>>,
    blocks: &mut Vec<(u64, u64)>,
    f: Format,
    message: Message<'a>,
) -> Result<()> {
    if message.kind == kind::CONTINUATION {
        // IV.A.2.q: offset (O), length (L).
        let mut c = Cursor::new(message.data);
        let at = c.required_address(f, "a continuation block")?;
        let len = c.length(f)?;
        if blocks.iter().any(|&(start, _)| start == at) || blocks.len() >= MAX_BLOCKS {
            return Err(Error::Corrupt("object header continuations loop".into()));
        }
        blocks.push((at, len));
    } else if message.kind != 0 {
        out.push(message);
    }
    Ok(())
}

pub(crate) fn to_usize(n: u64) -> Result<usize> {
    usize::try_from(n).map_err(|_| Error::Corrupt(format!("a size of {n} bytes")))
}
