//! A reader for HDF5, the container NetCDF-4 is written in — enough of it
//! to read the files NOAA, the Met Office and CMC publish (spec.md §4.10).
//!
//! What it reads: superblocks 0 to 3; version 1 and 2 object headers with
//! their continuations; old-style groups (symbol tables) and new-style
//! ones, compact or dense; attributes compact or dense, numeric or text,
//! variable-length strings included; datasets compact, contiguous or
//! chunked behind a version 1 B-tree, deflated, shuffled or Fletcher-32
//! checksummed. What it refuses, by name: the version 4 layout's chunk
//! indexes, SZIP and every other compression, huge heap objects, and
//! shared messages. Damage is an error, never a panic.
//!
//! Section references in the modules are to the HDF5 File Format
//! Specification, version 3.0.
//!
//! It reads from memory: a file is a few tens of megabytes, read whole.

mod btree1;
mod cursor;
mod data;
mod header;
mod heap;
mod messages;

use std::path::Path;

use cursor::{Cursor, Format};
use header::{Message, SHARED, kind};
use messages::{Dense, Filter, Layout};

pub use messages::Datatype;

/// Why a file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not an HDF5 file")]
    NotHdf5,
    #[error("the file ends early (at byte {at})")]
    Truncated { at: u64 },
    #[error("the file is damaged: {0}")]
    Corrupt(String),
    #[error("{0} is not supported")]
    Unsupported(String),
    #[error("no {0} in the file")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An HDF5 file, held in memory.
#[derive(Debug)]
pub struct File {
    bytes: Vec<u8>,
    format: Format,
    root: u64,
}

impl File {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_bytes(std::fs::read(path)?)
    }

    /// Finds the superblock — at byte 0, or 512, 1024, 2048, … when a user
    /// block comes first — and the root group it names.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        const SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";
        let mut at = 0usize;
        while bytes.get(at..at + 8) != Some(SIGNATURE) {
            at = if at == 0 { 512 } else { at * 2 };
            if at + 8 > bytes.len() {
                return Err(Error::NotHdf5);
            }
        }
        let mut c = Cursor::at(&bytes, at as u64)?;
        c.skip(8)?;
        let version = c.u8()?;
        let (format, root) = match version {
            // II.A, version 0 and 1: free-space, root symbol table and
            // shared header versions, a reserved byte, the two sizes, a
            // reserved byte, the group K values (2 + 2), the consistency
            // flags (4), and in version 1 the indexed storage K (2) and two
            // reserved bytes. Then base, free-space, end-of-file and driver
            // addresses, and the root group's symbol table entry: link
            // name offset (O), then its object header's address.
            0 | 1 => {
                c.skip(4)?;
                let offsets = c.u8()?;
                let lengths = c.u8()?;
                check_sizes(offsets, lengths)?;
                c.skip(1 + 4 + 4)?;
                if version == 1 {
                    c.skip(4)?;
                }
                let raw = Format {
                    offsets,
                    lengths,
                    base: 0,
                };
                let base = c.address(raw)?.unwrap_or(0);
                let format = Format { base, ..raw };
                c.skip(3 * usize::from(offsets))?;
                c.skip(usize::from(offsets))?;
                (format, c.required_address(format, "the root group")?)
            }
            // II.A, version 2 and 3: the two sizes, flags (1), then base,
            // superblock extension, end-of-file and root group addresses.
            2 | 3 => {
                let offsets = c.u8()?;
                let lengths = c.u8()?;
                check_sizes(offsets, lengths)?;
                c.skip(1)?;
                let raw = Format {
                    offsets,
                    lengths,
                    base: 0,
                };
                let base = c.address(raw)?.unwrap_or(0);
                let format = Format { base, ..raw };
                c.skip(2 * usize::from(offsets))?;
                (format, c.required_address(format, "the root group")?)
            }
            v => return Err(Error::Unsupported(format!("superblock version {v}"))),
        };
        Ok(Self {
            bytes,
            format,
            root,
        })
    }

    /// The names in the root group.
    pub fn names(&self) -> Result<Vec<String>> {
        Ok(self
            .members(self.root)?
            .into_iter()
            .map(|(name, _)| name)
            .collect())
    }

    /// The root group's attributes: a NetCDF file's global attributes.
    pub fn attributes(&self) -> Result<Vec<Attribute>> {
        let messages = header::messages(&self.bytes, self.format, self.root)?;
        self.attributes_of(&messages)
    }

    /// The dataset at `path`, its components separated by `/`.
    pub fn dataset(&self, path: &str) -> Result<Dataset<'_>> {
        let mut at = self.root;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            at = self
                .members(at)?
                .into_iter()
                .find(|(name, _)| name == part)
                .map(|(_, address)| address)
                .ok_or_else(|| Error::NotFound(path.to_string()))?;
        }
        Dataset::read(self, at, path)
    }

    /// A group's links to objects, by name: from its symbol table, its
    /// link messages, or its dense storage. Soft and external links are
    /// left out — they name no object in this file.
    fn members(&self, group: u64) -> Result<Vec<(String, u64)>> {
        let (file, f) = (&self.bytes[..], self.format);
        let messages = header::messages(file, f, group)?;
        let mut out = Vec::new();
        for m in &messages {
            let mut c = Cursor::new(m.data);
            match m.kind {
                kind::LINK => {
                    if let (name, Some(address)) = messages::link(&mut c, f)? {
                        out.push((name, address));
                    }
                }
                kind::LINK_INFO => {
                    let dense = messages::link_info(&mut c, f)?;
                    for record in self.dense(dense, 5)? {
                        let (name, address) = messages::link(&mut Cursor::new(record), f)?;
                        out.extend(address.map(|a| (name, a)));
                    }
                }
                // III.C: a symbol table node is "SNOD", version (1),
                // reserved (1), symbol count (2), then entries of link name
                // offset (O), object header (O), cache type (4), reserved
                // (4) and scratch-pad (16).
                kind::SYMBOL_TABLE => {
                    let btree = c.required_address(f, "a group's B-tree")?;
                    let heap = c.required_address(f, "a group's heap")?;
                    for node in btree1::symbol_nodes(file, f, btree)? {
                        let mut s = Cursor::at(file, node)?;
                        s.signature(b"SNOD")?;
                        s.skip(2)?;
                        for _ in 0..s.u16()? {
                            let name = s.length_of_offset(f)?;
                            let address = s.required_address(f, "a group member")?;
                            s.skip(24)?;
                            out.push((heap::local_heap_name(file, f, heap, name)?, address));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// The heap objects of a dense store, through its name index: link
    /// records (type 5) are a hash (4) then the heap ID; attribute records
    /// (type 8) are the heap ID (8), message flags (1), creation order (4)
    /// and hash (4).
    fn dense(&self, dense: Dense, expected: u8) -> Result<Vec<&[u8]>> {
        let (Some(heap), Some(names)) = (dense.heap, dense.names) else {
            return Ok(Vec::new());
        };
        let (file, f) = (&self.bytes[..], self.format);
        let heap = heap::FractalHeap::read(file, f, heap)?;
        let (kind, records) = heap::btree2_records(file, f, names)?;
        if kind != expected {
            return Err(Error::Corrupt(format!("a name index of type {kind}")));
        }
        let mut out = Vec::with_capacity(records.len());
        for record in records {
            let id = match kind {
                5 => record.get(4..),
                _ => {
                    if record.get(8).is_some_and(|flags| flags & SHARED != 0) {
                        continue;
                    }
                    record.get(..8)
                }
            };
            let id = id.ok_or_else(|| Error::Corrupt("a short name index record".into()))?;
            out.push(heap.object(file, f, id)?);
        }
        Ok(out)
    }

    fn attributes_of(&self, messages: &[Message]) -> Result<Vec<Attribute>> {
        let f = self.format;
        let mut raw = Vec::new();
        for m in messages {
            if m.flags & SHARED != 0 {
                continue;
            }
            let mut c = Cursor::new(m.data);
            match m.kind {
                kind::ATTRIBUTE => raw.extend(messages::attribute(&mut c, f)?),
                kind::ATTRIBUTE_INFO => {
                    let dense = messages::attribute_info(&mut c, f)?;
                    for record in self.dense(dense, 8)? {
                        raw.extend(messages::attribute(&mut Cursor::new(record), f)?);
                    }
                }
                _ => {}
            }
        }
        raw.into_iter().map(|a| self.value_of(a)).collect()
    }

    fn value_of(&self, raw: messages::RawAttribute) -> Result<Attribute> {
        let value = match raw.datatype {
            Datatype::Integer { .. } | Datatype::Float { .. } => {
                Value::Numbers(data::numbers(raw.data, &raw.datatype)?)
            }
            Datatype::FixedString { size } if size > 0 => {
                Value::Text(raw.data.chunks(size).map(messages::text).collect())
            }
            Datatype::FixedString { .. } => Value::Text(Vec::new()),
            // Each element: length (4), the global heap collection (O),
            // and the object's index in it (4).
            Datatype::VarString { size } if size > 0 => {
                let mut strings = Vec::with_capacity(raw.count);
                for element in raw.data.chunks_exact(size) {
                    let mut c = Cursor::new(element);
                    let len = c.u32()? as usize;
                    let collection = c.address(self.format)?;
                    let index = c.u32()?;
                    strings.push(match collection {
                        Some(at) if len > 0 => {
                            let object = heap::global_object(&self.bytes, self.format, at, index)?;
                            messages::text(object.get(..len).unwrap_or(object))
                        }
                        _ => String::new(),
                    });
                }
                Value::Text(strings)
            }
            _ => Value::Unsupported,
        };
        Ok(Attribute {
            name: raw.name,
            value,
        })
    }
}

fn check_sizes(offsets: u8, lengths: u8) -> Result<()> {
    let ok = |n: u8| matches!(n, 2 | 4 | 8);
    if ok(offsets) && ok(lengths) {
        Ok(())
    } else {
        Err(Error::Corrupt(format!(
            "addresses of {offsets} bytes and lengths of {lengths}"
        )))
    }
}

impl Cursor<'_> {
    /// A symbol table entry's link name offset, which is address-sized.
    fn length_of_offset(&mut self, f: Format) -> Result<u64> {
        self.uint(usize::from(f.offsets))
    }
}

/// An attribute's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Numbers(Vec<f64>),
    Text(Vec<String>),
    /// A type this reader does not decode: a reference, a compound.
    Unsupported,
}

/// A named value on a dataset or group.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    name: String,
    value: Value,
}

impl Attribute {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Every element of a numeric attribute.
    pub fn numbers(&self) -> Option<&[f64]> {
        match &self.value {
            Value::Numbers(n) => Some(n),
            _ => None,
        }
    }

    /// The first element of a numeric attribute — how NetCDF stores a
    /// `scale_factor` or a `_FillValue`.
    pub fn number(&self) -> Option<f64> {
        self.numbers().and_then(|n| n.first().copied())
    }

    /// The first string of a text attribute.
    pub fn text(&self) -> Option<&str> {
        match &self.value {
            Value::Text(t) => t.first().map(String::as_str),
            _ => None,
        }
    }
}

/// A dataset: a NetCDF variable.
#[derive(Debug)]
pub struct Dataset<'f> {
    file: &'f File,
    shape: Vec<u64>,
    datatype: Datatype,
    layout: Layout,
    filters: Vec<Filter>,
    fill: Option<Vec<u8>>,
    attributes: Vec<Attribute>,
}

impl<'f> Dataset<'f> {
    fn read(file: &'f File, address: u64, path: &str) -> Result<Self> {
        let f = file.format;
        let messages = header::messages(&file.bytes, f, address)?;
        let (mut shape, mut datatype, mut layout) = (None, None, None);
        let (mut filters, mut fill, mut old_fill) = (Vec::new(), None, None);
        for m in &messages {
            let mut c = Cursor::new(m.data);
            let shared = m.flags & SHARED != 0;
            match m.kind {
                kind::DATASPACE | kind::DATATYPE | kind::LAYOUT if shared => {
                    return Err(Error::Unsupported("a shared header message".into()));
                }
                kind::DATASPACE => shape = Some(messages::dataspace(&mut c, f)?),
                kind::DATATYPE => datatype = Some(messages::datatype(&mut c)?),
                kind::LAYOUT => layout = Some(messages::layout(&mut c, f)?),
                kind::FILTERS => filters = messages::filters(&mut c)?,
                kind::FILL if !shared => fill = messages::fill(m.kind, &mut c)?,
                kind::FILL_OLD if !shared => old_fill = messages::fill(m.kind, &mut c)?,
                _ => {}
            }
        }
        let (Some(shape), Some(datatype), Some(layout)) = (shape, datatype, layout) else {
            return Err(Error::NotFound(format!("dataset {path}")));
        };
        Ok(Self {
            file,
            shape: shape.unwrap_or_default(),
            datatype,
            layout,
            filters,
            fill: fill.or(old_fill),
            attributes: file.attributes_of(&messages)?,
        })
    }

    /// The dimensions, slowest-varying first. Empty for a scalar.
    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    pub fn datatype(&self) -> &Datatype {
        &self.datatype
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// The elements as stored, row-major, with the fill value wherever
    /// nothing was written. No packing or fill handling is applied: that
    /// is the NetCDF conventions' business, and the caller's.
    pub fn read_raw(&self) -> Result<Vec<u8>> {
        data::raw(
            &self.file.bytes,
            self.file.format,
            &self.shape,
            self.datatype.size(),
            &self.layout,
            &self.filters,
            self.fill.as_deref(),
        )
    }

    /// The elements as doubles.
    pub fn read_f64(&self) -> Result<Vec<f64>> {
        data::numbers(&self.read_raw()?, &self.datatype)
    }

    /// The elements as singles: exact for floats and for integers to 2^24.
    pub fn read_f32(&self) -> Result<Vec<f32>> {
        Ok(self.read_f64()?.into_iter().map(|v| v as f32).collect())
    }
}
