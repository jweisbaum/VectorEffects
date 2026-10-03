//! The heaps and the second B-tree: where dense links and attributes, the
//! names of old-style groups, and variable-length strings are kept. Section
//! references are to the HDF5 File Format Specification 3.0, III.

use crate::cursor::{Cursor, Format, limit_enc_size};
use crate::header::to_usize;
use crate::{Error, Result};

/// No B-tree in a real file is deeper than this; a damaged depth stops here.
const MAX_DEPTH: u16 = 16;

/// III.D, a local heap: "HEAP", version (1), reserved (3), data segment
/// size (L), free list offset (L), data segment address (O). Returns the
/// NUL-terminated name at `offset` into the data segment.
pub(crate) fn local_heap_name(file: &[u8], f: Format, heap: u64, offset: u64) -> Result<String> {
    let mut c = Cursor::at(file, heap)?;
    c.signature(b"HEAP")?;
    c.skip(4)?;
    let size = c.length(f)?;
    let _free = c.length(f)?;
    let data = c.required_address(f, "a local heap's data")?;
    if offset >= size {
        return Err(Error::Corrupt("a name past its local heap".into()));
    }
    let mut c = Cursor::at(
        file,
        data.checked_add(offset)
            .ok_or(Error::Truncated { at: data })?,
    )?;
    let rest = c.bytes(to_usize(size - offset)?.min(c.remaining()))?;
    Ok(crate::messages::text(rest))
}

/// III.E, an object in a global heap collection: "GCOL", version (1),
/// reserved (3), collection size (L), then objects — index (2), reference
/// count (2), reserved (4), size (L), data padded to 8 — until index 0.
pub(crate) fn global_object(file: &[u8], f: Format, collection: u64, index: u32) -> Result<&[u8]> {
    let mut c = Cursor::at(file, collection)?;
    let start = c.pos;
    c.signature(b"GCOL")?;
    c.skip(4)?;
    let size = to_usize(c.length(f)?)?;
    // Both headers are padded to 8 bytes, which with 4-byte lengths is not
    // where they end (`H5HG_SIZEOF_HDR`, `H5HG_SIZEOF_OBJHDR`).
    c.align(start, 8)?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| Error::Corrupt("a global heap past the address space".into()))?;
    while c.pos + 8 + usize::from(f.lengths) <= end {
        let id = c.u16()?;
        if id == 0 {
            break;
        }
        c.skip(6)?;
        let len = to_usize(c.length(f)?)?;
        c.align(start, 8)?;
        let data = c.bytes(len)?;
        if u32::from(id) == index {
            return Ok(data);
        }
        c.align(start, 8)?;
    }
    Err(Error::Corrupt(format!(
        "no object {index} in the global heap at {collection}"
    )))
}

/// III.G, a fractal heap's header, as much of it as reading an object needs.
#[derive(Debug, Clone)]
pub(crate) struct FractalHeap {
    width: u64,
    start_block: u64,
    max_direct_rows: u64,
    /// Bytes in a heap offset, and so in a block's offset field.
    offset_size: usize,
    /// Bytes in a managed object's length within its heap ID.
    length_size: usize,
    id_length: usize,
    root: Option<u64>,
    root_rows: u64,
}

impl FractalHeap {
    /// "FRHP", version (1), heap ID length (2), I/O filter length (2),
    /// flags (1), max managed object size (4), next huge ID (L), huge B-tree
    /// (O), free space (L), free-space manager (O), managed space (L),
    /// allocated (L), iterator offset (L), managed count (L), huge size (L),
    /// huge count (L), tiny size (L), tiny count (L), table width (2),
    /// starting block size (L), maximum direct block size (L), maximum heap
    /// size in bits (2), starting rows (2), root block (O), current rows (2).
    pub fn read(file: &[u8], f: Format, address: u64) -> Result<Self> {
        let mut c = Cursor::at(file, address)?;
        c.signature(b"FRHP")?;
        c.skip(1)?;
        let id_length = usize::from(c.u16()?);
        let filters = c.u16()?;
        c.skip(1)?; // flags
        let max_managed = u64::from(c.u32()?);
        c.length(f)?;
        c.address(f)?;
        c.length(f)?;
        c.address(f)?;
        for _ in 0..8 {
            c.length(f)?;
        }
        let width = u64::from(c.u16()?);
        let start_block = c.length(f)?;
        let max_direct = c.length(f)?;
        let max_bits = c.u16()?;
        c.skip(2)?;
        let root = c.address(f)?;
        let root_rows = u64::from(c.u16()?);
        if filters != 0 {
            return Err(Error::Unsupported("a filtered fractal heap".into()));
        }
        let power = |n: u64| n.is_power_of_two();
        if !power(width)
            || !power(start_block)
            || !power(max_direct)
            || max_direct < start_block
            || max_bits == 0
            || max_bits > 64
        {
            return Err(Error::Corrupt(format!("the fractal heap at {address}")));
        }
        Ok(Self {
            width,
            start_block,
            max_direct_rows: u64::from(max_direct.ilog2() - start_block.ilog2()) + 2,
            offset_size: usize::from(max_bits).div_ceil(8),
            length_size: (u64::from(max_direct.ilog2()).div_ceil(8) as usize)
                .min(limit_enc_size(max_managed)),
            id_length,
            root,
            root_rows,
        })
    }

    /// The object a heap ID names (III.G, "Fractal Heap ID"): the first
    /// byte's bits 4–5 are the type — 0 managed, 1 huge, 2 tiny.
    pub fn object<'a>(&self, file: &'a [u8], f: Format, id: &'a [u8]) -> Result<&'a [u8]> {
        let mut c = Cursor::new(id);
        let first = c.u8()?;
        match (first >> 4) & 0x03 {
            0 => {
                let offset = c.uint(self.offset_size)?;
                let len = to_usize(c.uint(self.length_size)?)?;
                let at = self.locate(file, f, offset)?;
                Cursor::at(file, at)?.bytes(len)
            }
            2 => {
                // Tiny: the object is in the ID. Its length less one is the
                // low nibble, widened by the next byte past 18-byte IDs.
                let len = if self.id_length <= 18 {
                    usize::from(first & 0x0F) + 1
                } else {
                    (usize::from(first & 0x0F) << 8 | usize::from(c.u8()?)) + 1
                };
                c.bytes(len)
            }
            _ => Err(Error::Unsupported("a huge fractal heap object".into())),
        }
    }

    /// The file position of the managed object at heap `offset`.
    fn locate(&self, file: &[u8], f: Format, offset: u64) -> Result<u64> {
        let root = self
            .root
            .ok_or_else(|| Error::Corrupt("an object in an empty fractal heap".into()))?;
        if self.root_rows == 0 {
            return self.in_direct(file, f, root, 0, self.start_block, offset);
        }
        self.in_indirect(file, f, root, self.root_rows, 0, offset, 0)
    }

    fn row_size(&self, row: u64) -> Result<u64> {
        if row == 0 {
            return Ok(self.start_block);
        }
        u32::try_from(row - 1)
            .ok()
            .and_then(|shift| 1u64.checked_shl(shift))
            .and_then(|scale| self.start_block.checked_mul(scale))
            .ok_or_else(|| Error::Corrupt("a fractal heap row past the address space".into()))
    }

    /// "FHDB", version (1), heap header (O), block offset, and a checksum
    /// when the heap's flags ask for one. Heap offsets count from the start
    /// of the block, header included, so the checksum needs no skipping.
    fn in_direct(
        &self,
        file: &[u8],
        f: Format,
        at: u64,
        block_offset: u64,
        size: u64,
        offset: u64,
    ) -> Result<u64> {
        let mut c = Cursor::at(file, at)?;
        c.signature(b"FHDB")?;
        c.skip(1)?;
        c.address(f)?;
        let stored = c.uint(self.offset_size)?;
        if stored != block_offset || offset < block_offset || offset - block_offset >= size {
            return Err(Error::Corrupt(
                "a fractal heap direct block out of place".into(),
            ));
        }
        at.checked_add(offset - block_offset)
            .ok_or(Error::Truncated { at })
    }

    /// "FHIB", version (1), heap header (O), block offset, then an address
    /// per child — every direct block row first, then every indirect one —
    /// in row order. A row's blocks each span the row's size; an indirect
    /// child in row `r` has `r - log2(width)` rows of its own.
    #[allow(clippy::too_many_arguments)]
    fn in_indirect(
        &self,
        file: &[u8],
        f: Format,
        at: u64,
        rows: u64,
        block_offset: u64,
        offset: u64,
        depth: u16,
    ) -> Result<u64> {
        if depth > MAX_DEPTH {
            return Err(Error::Corrupt("fractal heap blocks nest too deep".into()));
        }
        let mut c = Cursor::at(file, at)?;
        c.signature(b"FHIB")?;
        c.skip(1)?;
        c.address(f)?;
        let stored = c.uint(self.offset_size)?;
        if stored != block_offset {
            return Err(Error::Corrupt(
                "a fractal heap indirect block out of place".into(),
            ));
        }
        let mut start = block_offset;
        for row in 0..rows {
            let size = self.row_size(row)?;
            for _ in 0..self.width {
                let child = c.address(f)?;
                let end = start.checked_add(size).ok_or_else(|| {
                    Error::Corrupt("a fractal heap past the address space".into())
                })?;
                if (start..end).contains(&offset) {
                    let child = child.ok_or_else(|| {
                        Error::Corrupt("an object in an unallocated heap block".into())
                    })?;
                    return if row < self.max_direct_rows {
                        self.in_direct(file, f, child, start, size, offset)
                    } else {
                        let child_rows = row
                            .checked_sub(u64::from(self.width.ilog2()))
                            .filter(|&r| r > 0 && r < rows)
                            .ok_or_else(|| {
                                Error::Corrupt("a fractal heap indirect block's rows".into())
                            })?;
                        self.in_indirect(file, f, child, child_rows, start, offset, depth + 1)
                    };
                }
                start = end;
            }
        }
        Err(Error::Corrupt(
            "a fractal heap offset past its blocks".into(),
        ))
    }
}

/// III.A.2, every record of a version 2 B-tree, in key order: "BTHD",
/// version (1), type (1), node size (4), record size (2), depth (2), split
/// and merge percentages (2), root (O), root record count (2), total (L).
pub(crate) fn btree2_records(file: &[u8], f: Format, address: u64) -> Result<(u8, Vec<&[u8]>)> {
    let mut c = Cursor::at(file, address)?;
    c.signature(b"BTHD")?;
    c.skip(1)?;
    let kind = c.u8()?;
    let node_size = u64::from(c.u32()?);
    let record_size = c.u16()?;
    let depth = c.u16()?;
    c.skip(2)?;
    let root = c.address(f)?;
    let root_records = c.u16()?;
    if depth > MAX_DEPTH || record_size == 0 {
        return Err(Error::Corrupt(format!("the v2 B-tree at {address}")));
    }
    let tree = Btree2 {
        record_size: usize::from(record_size),
        kind,
        widths: Btree2::widths(node_size, u64::from(record_size), depth, f)?,
        seen: std::cell::RefCell::new(std::collections::BTreeSet::new()),
    };
    let mut out = Vec::new();
    if let Some(root) = root {
        tree.walk(file, f, root, u64::from(root_records), depth, &mut out)?;
    }
    Ok((kind, out))
}

struct Btree2 {
    record_size: usize,
    kind: u8,
    /// Per depth: bytes in a child's record count, and in its total
    /// (`max_nrec_size` and `cum_max_nrec_size` in the library).
    widths: Vec<(usize, usize)>,
    /// Nodes walked: one reached twice is damage, as in a version 1 tree.
    seen: std::cell::RefCell<std::collections::BTreeSet<u64>>,
}

impl Btree2 {
    /// The widths of the counts in each level's child pointers, which
    /// depend on how many records a node of each level could hold
    /// (`H5B2__hdr_init`).
    fn widths(
        node_size: u64,
        record_size: u64,
        depth: u16,
        f: Format,
    ) -> Result<Vec<(usize, usize)>> {
        const PREFIX: u64 = 10; // signature, version, type, checksum
        let leaf_max = node_size
            .checked_sub(PREFIX)
            .map(|n| n / record_size)
            .filter(|&n| n > 0)
            .ok_or_else(|| Error::Corrupt("a v2 B-tree node too small".into()))?;
        let nrec_size = limit_enc_size(leaf_max);
        let mut cum_max = vec![leaf_max];
        let mut cum_size = vec![0usize];
        for level in 1..=usize::from(depth) {
            let pointer = u64::from(f.offsets)
                + nrec_size as u64
                + if level > 1 {
                    cum_size[level - 1] as u64
                } else {
                    0
                };
            let max = node_size
                .checked_sub(PREFIX + pointer)
                .map(|n| n / (record_size + pointer))
                .ok_or_else(|| Error::Corrupt("a v2 B-tree node too small".into()))?;
            let cum = (max + 1)
                .checked_mul(cum_max[level - 1])
                .and_then(|n| n.checked_add(max))
                .unwrap_or(u64::MAX);
            cum_max.push(cum);
            cum_size.push(limit_enc_size(cum));
        }
        Ok(cum_size.into_iter().map(|cum| (nrec_size, cum)).collect())
    }

    /// A leaf is "BTLF", version, type, records. An internal node is
    /// "BTIN", version, type, records, then a pointer per child: address
    /// (O), record count, and below depth 1 the subtree's total.
    fn walk<'a>(
        &self,
        file: &'a [u8],
        f: Format,
        at: u64,
        records: u64,
        depth: u16,
        out: &mut Vec<&'a [u8]>,
    ) -> Result<()> {
        if !self.seen.borrow_mut().insert(at) {
            return Err(Error::Corrupt(format!(
                "the v2 B-tree node at {at} is reached twice"
            )));
        }
        let mut c = Cursor::at(file, at)?;
        c.signature(if depth == 0 { b"BTLF" } else { b"BTIN" })?;
        c.skip(1)?;
        if c.u8()? != self.kind {
            return Err(Error::Corrupt("a v2 B-tree node of another type".into()));
        }
        let records = to_usize(records)?;
        let mut own = Vec::with_capacity(records.min(c.remaining() / self.record_size + 1));
        for _ in 0..records {
            own.push(c.bytes(self.record_size)?);
        }
        if depth == 0 {
            out.extend(own);
            return Ok(());
        }
        let (nrec_size, _) = self.widths[0];
        let total_size = self.widths[usize::from(depth) - 1].1;
        let mut children = Vec::with_capacity(records + 1);
        for _ in 0..=records {
            let child = c.required_address(f, "a v2 B-tree child")?;
            let count = c.uint(nrec_size)?;
            if depth > 1 {
                c.uint(total_size)?;
            }
            children.push((child, count));
        }
        for (k, (child, count)) in children.into_iter().enumerate() {
            self.walk(file, f, child, count, depth - 1, out)?;
            if let Some(record) = own.get(k) {
                out.push(record);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With 4-byte lengths the collection header and each object header
    /// are 12 bytes, padded to 16 on disk (`H5HG_SIZEOF_HDR`,
    /// `H5HG_SIZEOF_OBJHDR`).
    #[test]
    fn global_heap_headers_are_padded_to_eight_with_short_lengths() {
        let f = Format {
            offsets: 8,
            lengths: 4,
            base: 0,
        };
        let mut file = b"GCOL\x01\0\0\0".to_vec();
        file.extend(64u32.to_le_bytes());
        file.extend([0; 4]);
        for (id, text) in [(1u16, &b"abc"[..]), (2, &b"hello"[..])] {
            file.extend(id.to_le_bytes());
            file.extend([1, 0, 0, 0, 0, 0]);
            file.extend((text.len() as u32).to_le_bytes());
            file.extend([0; 4]);
            file.extend(text);
            file.resize(file.len().div_ceil(8) * 8, 0);
        }
        file.resize(64, 0);
        assert_eq!(global_object(&file, f, 0, 1).expect("one"), b"abc");
        assert_eq!(global_object(&file, f, 0, 2).expect("two"), b"hello");
    }
}
