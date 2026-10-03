//! The bodies of the header messages this reader acts on. Section
//! references are to the HDF5 File Format Specification 3.0, IV.A.2.

use crate::cursor::{Cursor, Format};
use crate::{Error, Result};

/// What a stored element is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Datatype {
    /// Class 0, a whole number of bytes with no bit padding.
    Integer {
        size: usize,
        signed: bool,
        big_endian: bool,
    },
    /// Class 1, IEEE single or double.
    Float { size: usize, big_endian: bool },
    /// Class 3: `size` bytes, NUL-padded, of text.
    FixedString { size: usize },
    /// Class 9 of strings: a length and a global heap reference per element.
    VarString { size: usize },
    /// Anything else — compound, enum, reference, a sequence. Kept so an
    /// attribute of that type can be skipped by its size instead of failing
    /// the object it is on.
    Other { class: u8, size: usize },
}

impl Datatype {
    /// Bytes per element as stored.
    pub fn size(&self) -> usize {
        match *self {
            Datatype::Integer { size, .. }
            | Datatype::Float { size, .. }
            | Datatype::FixedString { size }
            | Datatype::VarString { size }
            | Datatype::Other { size, .. } => size,
        }
    }
}

/// IV.A.2.d: class and version (1), class bit fields (3), size (4), then
/// the class's properties.
pub(crate) fn datatype(c: &mut Cursor) -> Result<Datatype> {
    let class_version = c.u8()?;
    let class = class_version & 0x0F;
    let bits = c.bytes(3)?;
    let size = c.u32()? as usize;
    Ok(match class {
        0 => {
            // Properties: bit offset (2), bit precision (2).
            let offset = c.u16()?;
            let precision = c.u16()?;
            let whole =
                matches!(size, 1 | 2 | 4 | 8) && offset == 0 && usize::from(precision) == size * 8;
            if whole {
                Datatype::Integer {
                    size,
                    signed: bits[0] & 0x08 != 0,
                    big_endian: bits[0] & 0x01 != 0,
                }
            } else {
                Datatype::Other { class, size }
            }
        }
        1 => {
            // Bit 6 with bit 0 is VAX order, which nothing here reads.
            let ieee = matches!(size, 4 | 8) && bits[0] & 0x40 == 0;
            if ieee {
                Datatype::Float {
                    size,
                    big_endian: bits[0] & 0x01 != 0,
                }
            } else {
                Datatype::Other { class, size }
            }
        }
        3 => Datatype::FixedString { size },
        // A variable-length type whose bits 0–3 say "string".
        9 if bits[0] & 0x0F == 1 => Datatype::VarString { size },
        _ => Datatype::Other { class, size },
    })
}

/// IV.A.2.b. `None` is the null dataspace (no elements); an empty shape is
/// a scalar.
pub(crate) fn dataspace(c: &mut Cursor, f: Format) -> Result<Option<Vec<u64>>> {
    let version = c.u8()?;
    let rank = c.u8()?;
    let flags = c.u8()?;
    match version {
        // Version 1: reserved (1), reserved (4), then the dimensions.
        1 => c.skip(5)?,
        // Version 2: type (1) — 0 scalar, 1 simple, 2 null.
        2 => {
            if c.u8()? == 2 {
                return Ok(None);
            }
        }
        _ => return Err(Error::Unsupported(format!("dataspace version {version}"))),
    }
    if rank > 32 {
        return Err(Error::Corrupt(format!("a dataspace of rank {rank}")));
    }
    let dims = (0..rank).map(|_| c.length(f)).collect::<Result<Vec<_>>>()?;
    // Maximum dimensions follow when bit 0 is set; nothing here needs them.
    let _ = flags;
    Ok(Some(dims))
}

/// Where a dataset's elements are (IV.A.2.i, layout version 3).
#[derive(Debug, Clone)]
pub(crate) enum Layout {
    Compact(Vec<u8>),
    Contiguous {
        address: Option<u64>,
        size: u64,
    },
    /// `chunk` is the chunk's shape in elements, without the trailing
    /// element-size dimension the message carries.
    Chunked {
        btree: Option<u64>,
        chunk: Vec<u64>,
    },
}

pub(crate) fn layout(c: &mut Cursor, f: Format) -> Result<Layout> {
    let version = c.u8()?;
    if version != 3 {
        // Version 4 brings the extensible-array and v2 B-tree chunk
        // indexes, which netCDF-4 does not write; 1 and 2 predate 1.6.
        return Err(Error::Unsupported(format!("data layout version {version}")));
    }
    match c.u8()? {
        0 => {
            let size = c.u16()?;
            Ok(Layout::Compact(c.bytes(usize::from(size))?.to_vec()))
        }
        1 => Ok(Layout::Contiguous {
            address: c.address(f)?,
            size: c.length(f)?,
        }),
        2 => {
            // Dimensionality (1) is the rank plus one; address (O); then
            // that many 4-byte sizes, the last the element size.
            let n = c.u8()?;
            if !(2..=33).contains(&n) {
                return Err(Error::Corrupt(format!("a chunk of dimensionality {n}")));
            }
            let btree = c.address(f)?;
            let mut chunk = (0..n)
                .map(|_| c.u32().map(u64::from))
                .collect::<Result<Vec<_>>>()?;
            chunk.pop();
            if chunk.contains(&0) {
                return Err(Error::Corrupt("a chunk with an empty dimension".into()));
            }
            Ok(Layout::Chunked { btree, chunk })
        }
        class => Err(Error::Unsupported(format!("layout class {class}"))),
    }
}

/// One stage of a filter pipeline.
#[derive(Debug, Clone)]
pub(crate) struct Filter {
    pub id: u16,
    pub params: Vec<u32>,
}

/// IV.A.2.l.
pub(crate) fn filters(c: &mut Cursor) -> Result<Vec<Filter>> {
    let version = c.u8()?;
    let n = c.u8()?;
    let mut out = Vec::with_capacity(usize::from(n));
    match version {
        1 => {
            // Reserved (6); each filter: id (2), name length (2), flags
            // (2), value count (2), name (already padded to 8), values,
            // and four bytes of padding after an odd count.
            c.skip(6)?;
            for _ in 0..n {
                let id = c.u16()?;
                let name = c.u16()?;
                let _flags = c.u16()?;
                let count = c.u16()?;
                c.skip(usize::from(name))?;
                let params = (0..count).map(|_| c.u32()).collect::<Result<Vec<_>>>()?;
                if count % 2 == 1 {
                    c.skip(4)?;
                }
                out.push(Filter { id, params });
            }
        }
        2 => {
            // The name length is present only for ids of 256 and up, and
            // nothing is padded.
            for _ in 0..n {
                let id = c.u16()?;
                let name = if id >= 256 { c.u16()? } else { 0 };
                let _flags = c.u16()?;
                let count = c.u16()?;
                c.skip(usize::from(name))?;
                let params = (0..count).map(|_| c.u32()).collect::<Result<Vec<_>>>()?;
                out.push(Filter { id, params });
            }
        }
        _ => {
            return Err(Error::Unsupported(format!(
                "filter pipeline version {version}"
            )));
        }
    }
    Ok(out)
}

/// The fill value, from the new message (IV.A.2.f) or the old (IV.A.2.e).
pub(crate) fn fill(kind: u16, c: &mut Cursor) -> Result<Option<Vec<u8>>> {
    if kind == crate::header::kind::FILL_OLD {
        let size = c.u32()? as usize;
        return Ok(Some(c.bytes(size)?.to_vec()));
    }
    let version = c.u8()?;
    let defined = match version {
        1 => {
            c.skip(2)?;
            c.skip(1)?;
            true
        }
        2 => {
            c.skip(2)?;
            c.u8()? != 0
        }
        3 => c.u8()? & 0x20 != 0,
        _ => return Err(Error::Unsupported(format!("fill value version {version}"))),
    };
    if !defined {
        return Ok(None);
    }
    let size = c.u32()? as usize;
    (size > 0)
        .then(|| c.bytes(size).map(<[u8]>::to_vec))
        .transpose()
}

/// An attribute as stored: its value still raw.
#[derive(Debug, Clone)]
pub(crate) struct RawAttribute<'a> {
    pub name: String,
    pub datatype: Datatype,
    pub count: usize,
    pub data: &'a [u8],
}

/// IV.A.2.m. `None` for an attribute this reader cannot hold — one whose
/// datatype or dataspace is shared — which is skipped, not fatal.
pub(crate) fn attribute<'a>(c: &mut Cursor<'a>, f: Format) -> Result<Option<RawAttribute<'a>>> {
    let version = c.u8()?;
    let flags = c.u8()?;
    let name_size = usize::from(c.u16()?);
    let type_size = usize::from(c.u16()?);
    let space_size = usize::from(c.u16()?);
    let pad = |n: usize| if version == 1 { n.div_ceil(8) * 8 } else { n };
    match version {
        1 | 2 => {}
        3 => c.skip(1)?,
        _ => return Err(Error::Unsupported(format!("attribute version {version}"))),
    }
    let name = c.bytes(pad(name_size))?;
    let name = text(&name[..name_size.min(name.len())]);
    let type_bytes = c.bytes(pad(type_size))?;
    let space_bytes = c.bytes(pad(space_size))?;
    if version > 1 && flags & 0x03 != 0 {
        return Ok(None);
    }
    let datatype = datatype(&mut Cursor::new(type_bytes))?;
    let count = match dataspace(&mut Cursor::new(space_bytes), f)? {
        None => 0,
        Some(dims) => element_count(&dims)?,
    };
    let size = count
        .checked_mul(datatype.size())
        .ok_or_else(|| Error::Corrupt("an attribute's size overflows".into()))?;
    let data = c.bytes(size)?;
    Ok(Some(RawAttribute {
        name,
        datatype,
        count,
        data,
    }))
}

/// IV.A.2.g: a link's name and, for a hard link, the object it names.
/// Soft and external links come back with no address.
pub(crate) fn link(c: &mut Cursor, f: Format) -> Result<(String, Option<u64>)> {
    let version = c.u8()?;
    if version != 1 {
        return Err(Error::Unsupported(format!("link version {version}")));
    }
    let flags = c.u8()?;
    let kind = if flags & 0x08 != 0 { c.u8()? } else { 0 };
    if flags & 0x04 != 0 {
        c.skip(8)?;
    }
    if flags & 0x10 != 0 {
        c.skip(1)?;
    }
    let len = c.uint(1 << (flags & 0x03))?;
    let name = text(c.bytes(crate::header::to_usize(len)?)?);
    let address = if kind == 0 { c.address(f)? } else { None };
    Ok((name, address))
}

/// Where a new-style group keeps links or an object keeps attributes once
/// there are too many for the header: a fractal heap and the v2 B-tree
/// that indexes it by name.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dense {
    pub heap: Option<u64>,
    pub names: Option<u64>,
}

/// IV.A.2.c, link info: version (1), flags (1), maximum creation index (8,
/// if bit 0), heap (O), name index (O), creation-order index (O, if bit 1).
pub(crate) fn link_info(c: &mut Cursor, f: Format) -> Result<Dense> {
    c.skip(1)?;
    let flags = c.u8()?;
    if flags & 0x01 != 0 {
        c.skip(8)?;
    }
    Ok(Dense {
        heap: c.address(f)?,
        names: c.address(f)?,
    })
}

/// IV.A.2.v, attribute info: as link info, but the creation index is 2 bytes.
pub(crate) fn attribute_info(c: &mut Cursor, f: Format) -> Result<Dense> {
    c.skip(1)?;
    let flags = c.u8()?;
    if flags & 0x01 != 0 {
        c.skip(2)?;
    }
    Ok(Dense {
        heap: c.address(f)?,
        names: c.address(f)?,
    })
}

pub(crate) fn element_count(dims: &[u64]) -> Result<usize> {
    dims.iter()
        .try_fold(1u64, |acc, &d| acc.checked_mul(d))
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Error::Corrupt("a shape whose size overflows".into()))
}

/// Text as stored: NUL-terminated or NUL-padded, UTF-8 or ASCII.
pub(crate) fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}
