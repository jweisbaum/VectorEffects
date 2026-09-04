# Packing fixtures

Five small GRIB2 files carrying the *same* field in five packings, so the
decoder can be held to one answer across all of them (`packings.rs`).

The field is the analytic one the import tests use — `u = lon/10 + hour`,
`v = lat/10 - hour`, on a 10° global grid of 36 × 19 points, at forecast
hours 0 and 3, four messages per file. Longitudes come from
`GridSpec::points`, so they run `0, 10 … 170, -180 … -10`: a transposed axis
or a swapped component is a number here and not a plausible-looking picture.

`simple.grib2` is the app's **own writer's** output, which is why the values
are known without trusting any decoder. The other four were made from it with
ecCodes 2.x, which is a development-machine tool and not a dependency of the
build or the tests:

```
grib_set -s packingType=grid_ccsds  simple.grib2 ccsds.grib2
grib_set -s packingType=grid_jpeg   simple.grib2 jpeg2000.grib2
```

The two `_bitmap` files additionally set `bitmapPresent=1` and mark 61 nodes
missing — a 60-node band in the middle rows and the first node, so both an
interior run and an edge are covered. They were made with the `eccodes`
Python bindings, since a bitmap needs the values set missing in the same pass
as the flag:

```python
codes_set(gid, "missingValue", 9999.0)
codes_set(gid, "bitmapPresent", 1)
values[200:260] = 9999.0
values[0] = 9999.0
codes_set(gid, "packingType", "grid_ccsds")   # or grid_jpeg
codes_set_values(gid, values)
```

Regenerating them is a deliberate act, not a routine one: they are the
independent reference, and re-recording them from our own decoder's output
would make the test assert only that the decoder still does what it does.

## `reference_spots.txt`

Ten values from each message of each file in the reference set, decoded by
**ecCodes**, one per line: file, message index, short name, node index, value.
The ten are both poles, both ends of the middle row, and six spread through
the interior — the places a transposed axis or a mishandled scanning mode
shows up first.

`reference_set.rs` checks the decoder against them. The GRIB files themselves
are 17 MB and are not committed; the test is `#[ignore]`d and looks for them
in `$VE_TEST_GRIBS`, defaulting to `~/temp_test_gribs`. Run it with:

```
VE_TEST_GRIBS=~/temp_test_gribs cargo test -p ve-grib --release \
    --test reference_set -- --ignored --nocapture
```

Generated with the `eccodes` Python bindings; regenerate only when the sample
set itself changes.
