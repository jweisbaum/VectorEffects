# Patched dependencies

Copies of crates.io crates with a local change, wired in through
`[patch.crates-io]` in the workspace `Cargo.toml` and excluded from the
workspace so they are neither linted nor formatted as ours. Each is the
published release unchanged except where listed. Drop a copy when upstream
makes the change unnecessary.

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
