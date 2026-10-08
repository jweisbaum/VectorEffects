# Patched dependencies

Copies of crates.io crates with a local change, wired in through
`[patch.crates-io]` in the workspace `Cargo.toml` and excluded from the
workspace so they are neither linted nor formatted as ours. Each is the
published release unchanged except where listed. Drop a copy when upstream
makes the change unnecessary.

## `zarrs` 0.23.14

Hindsight prefetch uses `subchunk_byte_range` to merge requests. Upstream
adds a shard index entry's offset and length without checking the Zarr
missing-subchunk sentinel `(u64::MAX, u64::MAX)`. A partly populated shard
therefore panics on overflow in development builds and yields an invalid
range in release builds. All three Hindsight mirrors encounter these gaps.

The change, in `src/array/codec/array_to_bytes/sharding.rs`: return `None`
for the sentinel, matching the decoder's fill-value handling, and return a
codec error if any other entry overflows. Normal populated ranges and
decoding are unchanged. This range-discovery API is used only by Hindsight
in VectorEffects; Open Data and near-real-time paths do not call it.

In `src/array/array_sync_sharded_readable_ext.rs`, the exclusively sharded
index cache releases its global mutex while constructing a missing decoder
(which reads the remote index). It then inserts or reuses the first completed
entry under the lock. Otherwise all index requests serialize even when the
caller supplies independent shard jobs. The non-sharded branch is unchanged.

Held by `global_read_keeps_missing_chunks_in_a_populated_shard_empty` in
`crates/ve-zarr/src/hindsight.rs`. The fixture includes populated and missing
chunks in one shard plus wholly absent shards, and checks actual vector
values and NaNs. Drop this patch when upstream includes both checks; run
the ve-zarr and history-import tests when updating it. The index-lock change
is held by `independent_shard_indexes_overlap_instead_of_holding_the_cache_lock`
beside that test; retain it until upstream also permits concurrent index I/O.

## `hayro-jpeg2000` 0.4.0

GRIB template 5.40 (JPEG 2000) is decoded by this crate. Upstream refuses any
image wider or taller than 60,000 pixels. A GRIB writer packs a bitmapped
field as one row of every value the bitmap keeps, so Météo-France's AROME
files carry codestreams millions of pixels wide (4,160,515 × 1 on the 0.01°
France grid) and were refused as `ImageTooLarge`.

The change, in `src/j2c/codestream.rs`: the cap applies to the image's
**area** (60,000 × 60,000, the most the old check allowed) instead of to each
side, so what a hostile header can make the decoder allocate is unchanged.

Held by `a_jpeg_2000_codestream_wider_than_60000_decodes` in
`crates/ve-grib/tests/packings.rs`. At AROME's size it was checked once
against ecCodes: a 4,203,301 × 1 codestream decoded to the same values,
all 4.2 million, in 1.9 s in release.

To update: copy the new release over this directory, reapply the change if
it is still needed, and run `cargo test -p ve-grib`. Check `cargo tree` for
a `-sys` crate after any bump (invariant 5).
