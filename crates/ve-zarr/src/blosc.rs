//! A pure-Rust decoder for the blosc1 container, limited to LZ4.
//!
//! The ARCO-ERA5 arrays are written with `blosc(cname="lz4",
//! clevel=5, shuffle=1)`. Decoding that needs three things: the container
//! layout, raw LZ4 block decompression, and the byte unshuffle filter. All
//! three are short, so this crate does them itself rather than linking
//! c-blosc, which drags in a C++ toolchain for a Snappy path that this store
//! never uses.
//!
//! # Container layout
//!
//! ```text
//! 0   version         1 octet
//! 1   version of lz    1 octet
//! 2   flags           1 octet
//! 3   typesize        1 octet
//! 4   nbytes          4 octets, little endian, uncompressed size
//! 8   blocksize       4 octets, little endian
//! 12  cbytes          4 octets, little endian, size of the whole container
//! 16  bstarts         4 octets each, one per block, little endian
//! ```
//!
//! Each block holds one or more *streams*. A block is split into `typesize`
//! streams unless the `DONT_SPLIT` flag is set or the block is the short
//! final one. Every stream is prefixed by its own compressed length; when
//! that length equals the stream's uncompressed size the stream was stored
//! verbatim, which is how blosc represents incompressible data.

use crate::error::{Result, ZarrError};

/// Byte shuffle filter was applied.
const FLAG_SHUFFLE: u8 = 0x01;
/// The payload is stored uncompressed.
const FLAG_MEMCPYED: u8 = 0x02;
/// Bit shuffle filter was applied. Not supported here.
const FLAG_BITSHUFFLE: u8 = 0x04;
/// Blocks were not split into per-byte streams.
const FLAG_DONT_SPLIT: u8 = 0x10;

/// The compressor identifier for LZ4, in the top three bits of the flags.
const COMPRESSOR_LZ4: u8 = 1;

/// The fixed container header size.
const HEADER_LEN: usize = 16;

fn u32le(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| ZarrError::Blosc(format!("truncated at offset {at}")))
}

/// Reverses blosc's byte shuffle.
///
/// The shuffled form stores every element's first byte, then every element's
/// second byte, and so on. Bytes past the last whole element are stored
/// verbatim, which matters for the short final block.
fn unshuffle(src: &[u8], typesize: usize) -> Vec<u8> {
    let n = src.len();
    if typesize <= 1 {
        return src.to_vec();
    }
    let mut out = vec![0u8; n];
    let nelem = n / typesize;
    for j in 0..typesize {
        let plane = j * nelem;
        for i in 0..nelem {
            out[i * typesize + j] = src[plane + i];
        }
    }
    let tail = nelem * typesize;
    out[tail..].copy_from_slice(&src[tail..]);
    out
}

/// Decompresses one blosc1 container.
///
/// # Errors
/// Returns [`ZarrError::Blosc`] if the container is truncated, uses a
/// compressor or filter this decoder does not implement, or does not decode to
/// the size its header declares.
pub fn decompress(src: &[u8]) -> Result<Vec<u8>> {
    if src.len() < HEADER_LEN {
        return Err(ZarrError::Blosc(format!(
            "container is {} bytes, shorter than the {HEADER_LEN}-byte header",
            src.len()
        )));
    }

    let flags = src[2];
    let typesize = usize::from(src[3]);
    let nbytes = u32le(src, 4)? as usize;
    let blocksize = u32le(src, 8)? as usize;
    let cbytes = u32le(src, 12)? as usize;

    if flags & FLAG_BITSHUFFLE != 0 {
        return Err(ZarrError::Blosc("bit shuffle is not supported".into()));
    }
    let compressor = flags >> 5;
    if compressor != COMPRESSOR_LZ4 {
        return Err(ZarrError::Blosc(format!(
            "compressor id {compressor} is not supported; this decoder handles lz4 only"
        )));
    }
    if cbytes != src.len() {
        return Err(ZarrError::Blosc(format!(
            "header declares {cbytes} bytes but the chunk is {}",
            src.len()
        )));
    }

    if flags & FLAG_MEMCPYED != 0 {
        return src
            .get(HEADER_LEN..HEADER_LEN + nbytes)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| ZarrError::Blosc("truncated uncompressed payload".into()));
    }

    if blocksize == 0 {
        return Err(ZarrError::Blosc("header declares a zero blocksize".into()));
    }
    if typesize == 0 {
        return Err(ZarrError::Blosc("header declares a zero typesize".into()));
    }

    let shuffled = flags & FLAG_SHUFFLE != 0;
    let dont_split = flags & FLAG_DONT_SPLIT != 0;
    let nblocks = nbytes.div_ceil(blocksize);
    let mut out = Vec::with_capacity(nbytes);

    for b in 0..nblocks {
        let start = u32le(src, HEADER_LEN + b * 4)? as usize;
        let is_final_partial = b == nblocks - 1 && !nbytes.is_multiple_of(blocksize);
        let block_len = if is_final_partial {
            nbytes - b * blocksize
        } else {
            blocksize
        };

        // c-blosc splits a block into one stream per byte of the element, so
        // that each stream sees one shuffle plane. The short final block and
        // an explicit DONT_SPLIT are the exceptions.
        let nstreams = if !dont_split && !is_final_partial {
            typesize
        } else {
            1
        };
        if block_len % nstreams != 0 {
            return Err(ZarrError::Blosc(format!(
                "block {b} of {block_len} bytes does not divide into {nstreams} streams"
            )));
        }
        let stream_len = block_len / nstreams;

        let mut block = Vec::with_capacity(block_len);
        let mut at = start;
        for s in 0..nstreams {
            let compressed_len = u32le(src, at)? as usize;
            at += 4;
            let payload = src.get(at..at + compressed_len).ok_or_else(|| {
                ZarrError::Blosc(format!("block {b} stream {s} runs past the chunk"))
            })?;
            if compressed_len == stream_len {
                // blosc stores incompressible streams verbatim.
                block.extend_from_slice(payload);
            } else {
                let decoded = lz4_flex::block::decompress(payload, stream_len)
                    .map_err(|e| ZarrError::Blosc(format!("block {b} stream {s}: {e}")))?;
                if decoded.len() != stream_len {
                    return Err(ZarrError::Blosc(format!(
                        "block {b} stream {s} decoded to {} bytes, expected {stream_len}",
                        decoded.len()
                    )));
                }
                block.extend_from_slice(&decoded);
            }
            at += compressed_len;
        }

        if shuffled {
            out.extend_from_slice(&unshuffle(&block, typesize));
        } else {
            out.extend_from_slice(&block);
        }
    }

    if out.len() != nbytes {
        return Err(ZarrError::Blosc(format!(
            "decoded {} bytes, header declares {nbytes}",
            out.len()
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shuffle is its own documentation: element bytes are interleaved
    /// into planes, and unshuffling must put them back in element order.
    #[test]
    fn unshuffle_reverses_the_plane_layout() {
        // Three 4-byte elements: 00010203 04050607 08090a0b.
        let planes = [0u8, 4, 8, 1, 5, 9, 2, 6, 10, 3, 7, 11];
        assert_eq!(
            unshuffle(&planes, 4),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        );
    }

    #[test]
    fn a_typesize_of_one_is_a_copy() {
        let data = [9u8, 8, 7];
        assert_eq!(unshuffle(&data, 1), data.to_vec());
    }

    /// A buffer that is not a whole number of elements keeps its tail bytes in
    /// place. The final block of an ERA5 chunk is where this shows up.
    #[test]
    fn unshuffle_copies_a_partial_trailing_element() {
        // Two whole 4-byte elements plus two loose bytes.
        let mut planes = vec![0u8, 4, 1, 5, 2, 6, 3, 7];
        planes.extend_from_slice(&[0xaa, 0xbb]);
        let out = unshuffle(&planes, 4);
        assert_eq!(&out[..8], &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&out[8..], &[0xaa, 0xbb], "tail bytes are verbatim");
    }

    fn header(flags: u8, typesize: u8, nbytes: u32, blocksize: u32, cbytes: u32) -> Vec<u8> {
        let mut h = vec![2u8, 1, flags, typesize];
        h.extend_from_slice(&nbytes.to_le_bytes());
        h.extend_from_slice(&blocksize.to_le_bytes());
        h.extend_from_slice(&cbytes.to_le_bytes());
        h
    }

    #[test]
    fn a_memcpyed_container_returns_its_payload() {
        let payload = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let flags = FLAG_MEMCPYED | (COMPRESSOR_LZ4 << 5);
        let mut c = header(flags, 4, 8, 8, (HEADER_LEN + 8) as u32);
        c.extend_from_slice(&payload);
        assert_eq!(decompress(&c).expect("decodes"), payload.to_vec());
    }

    /// One block, one stream, stored verbatim because it did not compress.
    #[test]
    fn an_incompressible_stream_is_taken_verbatim() {
        let payload = [0xdeu8, 0xad, 0xbe, 0xef];
        let flags = FLAG_DONT_SPLIT | (COMPRESSOR_LZ4 << 5);
        let body_at = HEADER_LEN + 4;
        let mut c = header(flags, 4, 4, 4, (body_at + 4 + 4) as u32);
        c.extend_from_slice(&(body_at as u32).to_le_bytes()); // bstarts[0]
        c.extend_from_slice(&4u32.to_le_bytes()); // stream length == stream size
        c.extend_from_slice(&payload);
        assert_eq!(decompress(&c).expect("decodes"), payload.to_vec());
    }

    #[test]
    fn a_short_container_is_rejected() {
        assert!(matches!(decompress(&[0; 4]), Err(ZarrError::Blosc(_))));
    }

    #[test]
    fn an_unsupported_compressor_is_rejected() {
        // Compressor 4 is zstd, which this decoder does not implement.
        let c = header(4 << 5, 4, 8, 8, HEADER_LEN as u32);
        let err = decompress(&c).expect_err("must reject");
        assert!(format!("{err}").contains("not supported"), "{err}");
    }

    #[test]
    fn bit_shuffle_is_rejected_rather_than_decoded_wrongly() {
        let flags = FLAG_BITSHUFFLE | (COMPRESSOR_LZ4 << 5);
        let c = header(flags, 4, 8, 8, HEADER_LEN as u32);
        let err = decompress(&c).expect_err("must reject");
        assert!(format!("{err}").contains("bit shuffle"), "{err}");
    }

    #[test]
    fn a_length_that_disagrees_with_the_header_is_rejected() {
        let flags = COMPRESSOR_LZ4 << 5;
        let c = header(flags, 4, 8, 8, 999);
        let err = decompress(&c).expect_err("must reject");
        assert!(format!("{err}").contains("declares"), "{err}");
    }
}
