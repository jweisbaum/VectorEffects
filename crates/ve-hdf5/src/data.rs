//! A dataset's elements: the chunks found, unfiltered and put in place, and
//! the stored numbers converted.

use std::io::Read;

use crate::btree1;
use crate::cursor::{Cursor, Format};
use crate::header::to_usize;
use crate::messages::{Datatype, Filter, Layout, element_count};
use crate::{Error, Result};

/// The largest dataset this reader will put in memory. A 0.05° global
/// field of doubles is 207 MB; a shape past this is a damaged header.
const MAX_BYTES: usize = 1 << 32;

/// The dataset's elements as stored, in row-major order, with the fill
/// value wherever nothing was written.
pub(crate) fn raw(
    file: &[u8],
    f: Format,
    shape: &[u64],
    element: usize,
    layout: &Layout,
    filters: &[Filter],
    fill: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let count = element_count(shape)?;
    let bytes = count
        .checked_mul(element)
        .filter(|&n| n <= MAX_BYTES)
        .ok_or_else(|| Error::Unsupported(format!("a dataset of shape {shape:?}")))?;
    let filled = |n: usize| match fill {
        Some(value) if value.len() == element && value.iter().any(|&b| b != 0) => {
            value.repeat(n / element.max(1))
        }
        _ => vec![0; n],
    };
    match layout {
        Layout::Compact(data) => {
            let mut out = data
                .get(..bytes)
                .ok_or(Error::Truncated { at: 0 })?
                .to_vec();
            out.truncate(bytes);
            Ok(out)
        }
        Layout::Contiguous { address: None, .. } => Ok(filled(bytes)),
        Layout::Contiguous {
            address: Some(at),
            size,
        } => {
            if (*size as u128) < bytes as u128 {
                return Err(Error::Corrupt(
                    "contiguous storage shorter than its dataset".into(),
                ));
            }
            Ok(Cursor::at(file, *at)?.bytes(bytes)?.to_vec())
        }
        Layout::Chunked { btree, chunk } => {
            if chunk.len() != shape.len() {
                return Err(Error::Corrupt(
                    "a chunk of another rank than its dataset".into(),
                ));
            }
            let mut out = filled(bytes);
            let Some(btree) = btree else {
                return Ok(out);
            };
            let chunk_bytes = element_count(chunk)?
                .checked_mul(element)
                .filter(|&n| n <= MAX_BYTES)
                .ok_or_else(|| Error::Corrupt(format!("a chunk of shape {chunk:?}")))?;
            for stored in btree1::chunks(file, f, *btree, shape.len())? {
                let data = Cursor::at(file, stored.address)?.bytes(stored.size as usize)?;
                let data = unfilter(data, filters, stored.mask, element, chunk_bytes)?;
                place(&mut out, &data, shape, chunk, &stored.offset, element)?;
            }
            Ok(out)
        }
    }
}

/// Undoes the pipeline, last filter first, skipping those the chunk's
/// mask says were not applied.
fn unfilter(
    data: &[u8],
    filters: &[Filter],
    mask: u32,
    element: usize,
    expected: usize,
) -> Result<Vec<u8>> {
    let mut data = data.to_vec();
    for (k, filter) in filters.iter().enumerate().rev() {
        if k < 32 && mask & (1 << k) != 0 {
            continue;
        }
        data = match filter.id {
            1 => {
                // Room for a checksum filter applied before deflate.
                let limit = expected as u64 + 64;
                let mut out = Vec::with_capacity(expected);
                flate2::read::ZlibDecoder::new(&data[..])
                    .take(limit)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Corrupt(format!("a deflated chunk: {e}")))?;
                out
            }
            2 => {
                let size = filter.params.first().map_or(element, |&s| s as usize);
                unshuffle(&data, size)
            }
            3 => {
                // Fletcher-32: the checksum is the last four bytes.
                let keep = data.len().saturating_sub(4);
                data.truncate(keep);
                data
            }
            id => {
                return Err(Error::Unsupported(format!(
                    "the {} filter",
                    filter_name(id)
                )));
            }
        };
    }
    if data.len() < expected {
        return Err(Error::Corrupt(format!(
            "a chunk of {} bytes where {expected} were expected",
            data.len()
        )));
    }
    data.truncate(expected);
    Ok(data)
}

/// The shuffle filter stores every element's first byte, then every
/// second byte, and so on; bytes past the last whole element stay put.
fn unshuffle(data: &[u8], size: usize) -> Vec<u8> {
    let n = data.len() / size.max(1);
    if size <= 1 || n == 0 {
        return data.to_vec();
    }
    let mut out = data.to_vec();
    for (b, plane) in data[..n * size].chunks_exact(n).enumerate() {
        for (i, &byte) in plane.iter().enumerate() {
            out[i * size + b] = byte;
        }
    }
    out
}

fn filter_name(id: u16) -> String {
    match id {
        4 => "SZIP".into(),
        5 => "N-bit".into(),
        6 => "scale-offset".into(),
        307 => "bzip2".into(),
        32001 => "Blosc".into(),
        32004 => "LZ4".into(),
        32015 => "Zstandard".into(),
        id => format!("HDF5 filter {id}"),
    }
}

/// Copies the part of a chunk inside the dataset into place. Edge chunks
/// are stored whole and run past the dataset's end.
fn place(
    out: &mut [u8],
    chunk_data: &[u8],
    shape: &[u64],
    chunk: &[u64],
    offset: &[u64],
    element: usize,
) -> Result<()> {
    let rank = shape.len();
    if offset.len() != rank {
        return Err(Error::Corrupt("a chunk key of another rank".into()));
    }
    if rank == 0 {
        let n = element.min(out.len()).min(chunk_data.len());
        out[..n].copy_from_slice(&chunk_data[..n]);
        return Ok(());
    }
    let mut extent = Vec::with_capacity(rank);
    for d in 0..rank {
        if offset[d] >= shape[d] || !offset[d].is_multiple_of(chunk[d]) {
            return Err(Error::Corrupt(format!(
                "a chunk at {offset:?} in a dataset of {shape:?}"
            )));
        }
        extent.push(to_usize(chunk[d].min(shape[d] - offset[d]))?);
    }
    let strides = |dims: &[u64]| -> Result<Vec<usize>> {
        let mut s = vec![1usize; rank];
        for d in (0..rank - 1).rev() {
            s[d] = s[d + 1] * to_usize(dims[d + 1])?;
        }
        Ok(s)
    };
    let (src_stride, dst_stride) = (strides(chunk)?, strides(shape)?);
    let row = extent[rank - 1] * element;
    let mut index = vec![0usize; rank];
    loop {
        let src: usize = (0..rank).map(|d| index[d] * src_stride[d]).sum::<usize>() * element;
        let dst: usize = (0..rank)
            .map(|d| (index[d] + offset[d] as usize) * dst_stride[d])
            .sum::<usize>()
            * element;
        out[dst..dst + row].copy_from_slice(&chunk_data[src..src + row]);
        // The next row: an odometer over every dimension but the last.
        let mut d = rank - 1;
        loop {
            if d == 0 {
                return Ok(());
            }
            d -= 1;
            index[d] += 1;
            if index[d] < extent[d] {
                break;
            }
            index[d] = 0;
        }
    }
}

/// Each stored element as a double. Integers up to 2^53 are exact.
pub(crate) fn numbers(raw: &[u8], datatype: &Datatype) -> Result<Vec<f64>> {
    let (size, big_endian) = match *datatype {
        Datatype::Integer {
            size, big_endian, ..
        }
        | Datatype::Float { size, big_endian } => (size, big_endian),
        ref other => return Err(Error::Unsupported(format!("reading {other:?} as numbers"))),
    };
    Ok(raw
        .chunks_exact(size)
        .map(|bytes| {
            let mut b = [0u8; 8];
            b[..size].copy_from_slice(bytes);
            if big_endian {
                b[..size].reverse();
            }
            let bits = u64::from_le_bytes(b);
            match *datatype {
                Datatype::Float { size: 4, .. } => f64::from(f32::from_bits(bits as u32)),
                Datatype::Float { .. } => f64::from_bits(bits),
                Datatype::Integer { signed: true, .. } => {
                    // Sign-extend from the stored width.
                    let shift = 64 - 8 * size as u32;
                    ((bits << shift) as i64 >> shift) as f64
                }
                _ => bits as f64,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shuffled chunk shorter than one element — an empty deflate
    /// stream, a damaged size — is a short chunk, not a panic.
    #[test]
    fn a_chunk_shorter_than_an_element_unshuffles_to_itself() {
        assert_eq!(unshuffle(&[1, 2], 4), [1, 2]);
        assert!(
            unfilter(
                &[1, 2],
                &[Filter {
                    id: 2,
                    params: vec![4]
                }],
                0,
                4,
                8
            )
            .is_err()
        );
    }

    /// Four 2-byte elements stored low bytes first, then high bytes.
    #[test]
    fn unshuffle_interleaves_the_byte_planes() {
        assert_eq!(
            unshuffle(&[1, 3, 5, 7, 2, 4, 6, 8, 9], 2),
            [1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
    }
}
