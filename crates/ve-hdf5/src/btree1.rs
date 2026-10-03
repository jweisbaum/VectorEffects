//! Version 1 B-trees (HDF5 File Format Specification 3.0, III.A.1): the
//! index of a chunked dataset's chunks, and of an old-style group's symbol
//! nodes.

use std::collections::BTreeSet;

use crate::cursor::{Cursor, Format};
use crate::{Error, Result};

/// One stored chunk.
#[derive(Debug, Clone)]
pub(crate) struct Chunk {
    /// Bytes as stored, after the filters.
    pub size: u32,
    /// Bit `i` set: filter `i` of the pipeline was not applied.
    pub mask: u32,
    /// The chunk's first element, per dimension.
    pub offset: Vec<u64>,
    pub address: u64,
}

/// No version 1 tree in a real file has more levels than this.
const MAX_LEVEL: u8 = 32;

/// Every chunk of a dataset of `rank` dimensions. A type 1 key is the
/// chunk's size (4), filter mask (4) and offset — `rank + 1` 8-byte
/// values, the last always 0 for the element-size dimension.
pub(crate) fn chunks(file: &[u8], f: Format, address: u64, rank: usize) -> Result<Vec<Chunk>> {
    let mut out = Vec::new();
    walk(
        file,
        f,
        address,
        1,
        8 + 8 * (rank + 1),
        None,
        &mut BTreeSet::new(),
        &mut |key, address| {
            let mut c = Cursor::new(key);
            let size = c.u32()?;
            let mask = c.u32()?;
            let offset = (0..rank).map(|_| c.uint(8)).collect::<Result<Vec<_>>>()?;
            out.push(Chunk {
                size,
                mask,
                offset,
                address,
            });
            Ok(())
        },
    )?;
    Ok(out)
}

/// Every symbol table node of an old-style group. A type 0 key is a heap
/// offset (L), which nothing here needs.
pub(crate) fn symbol_nodes(file: &[u8], f: Format, address: u64) -> Result<Vec<u64>> {
    let mut out = Vec::new();
    walk(
        file,
        f,
        address,
        0,
        usize::from(f.lengths),
        None,
        &mut BTreeSet::new(),
        &mut |_, address| {
            out.push(address);
            Ok(())
        },
    )?;
    Ok(out)
}

/// "TREE", type (1), level (1), entries used (2), left and right siblings
/// (O each), then key, child, key, child, …, key. `leaf` is handed each
/// leaf entry's key and the child after it. A node reached twice is damage:
/// in a real tree every node has one parent, and a damaged one whose
/// entries share a subtree would otherwise be walked exponentially often.
#[allow(clippy::too_many_arguments)]
fn walk(
    file: &[u8],
    f: Format,
    address: u64,
    kind: u8,
    key_len: usize,
    expected_level: Option<u8>,
    seen: &mut BTreeSet<u64>,
    leaf: &mut dyn FnMut(&[u8], u64) -> Result<()>,
) -> Result<()> {
    if !seen.insert(address) {
        return Err(Error::Corrupt(format!(
            "the B-tree node at {address} is reached twice"
        )));
    }
    let mut c = Cursor::at(file, address)?;
    c.signature(b"TREE")?;
    if c.u8()? != kind {
        return Err(Error::Corrupt(format!(
            "a B-tree node of another type at {address}"
        )));
    }
    let level = c.u8()?;
    if level > MAX_LEVEL || expected_level.is_some_and(|l| l != level) {
        return Err(Error::Corrupt(format!("a B-tree node at level {level}")));
    }
    let entries = c.u16()?;
    c.address(f)?;
    c.address(f)?;
    for _ in 0..entries {
        let key = c.bytes(key_len)?;
        let child = c.required_address(f, "a B-tree child")?;
        if level == 0 {
            leaf(key, child)?;
        } else {
            walk(file, f, child, kind, key_len, Some(level - 1), seen, leaf)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: Format = Format {
        offsets: 8,
        lengths: 8,
        base: 0,
    };

    /// A type 0 node: "TREE", type, level, entries, two siblings, then
    /// key, child pairs and a last key.
    fn node(level: u8, children: &[u64]) -> Vec<u8> {
        let mut out = b"TREE".to_vec();
        out.extend([0, level]);
        out.extend((children.len() as u16).to_le_bytes());
        out.extend([0xFF; 16]);
        for &child in children {
            out.extend(0u64.to_le_bytes());
            out.extend(child.to_le_bytes());
        }
        out.extend(0u64.to_le_bytes());
        out
    }

    /// Two entries naming one subtree: in a damaged file, the same thing at
    /// every level is exponential, so a node read twice is an error.
    #[test]
    fn a_node_reached_twice_is_damage() {
        let mut file = node(1, &[64, 64]);
        file.resize(64, 0);
        file.extend(node(0, &[1000]));
        assert!(symbol_nodes(&file, F, 0).is_err());
        let mut file = node(1, &[64]);
        file.resize(64, 0);
        file.extend(node(0, &[1000]));
        assert_eq!(symbol_nodes(&file, F, 0).expect("one path"), [1000]);
    }
}
