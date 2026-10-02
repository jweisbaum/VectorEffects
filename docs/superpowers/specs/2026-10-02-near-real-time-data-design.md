# Near-real-time data design

**Date:** 2026-10-02. **Status:** decisions taken in discussion, awaiting review.
**Companion to** `spec.md` §4.8 (imported layers), §4.10 (history layers),
§4.11 (backdrops), §9.1 (the timeline's start) and invariant 5.

## 1. What this is

A button in the layer panel, beside the calendar button of the history
import, that fetches **the last N days up to now** of observed sea-surface
temperature, surface current and surface wind, and adds one layer per product
ticked. Ten products are offered, each a checkbox, in three groups:

| Group | Products |
|---|---|
| SST | NOAA OISST · Met Office OSTIA (Copernicus) · NOAA Geo-Polar Blended · CMC |
| Currents | Copernicus DUACS · Copernicus MULTIOBS |
| Wind | ASCAT L3 Metop-B/C (Copernicus) · Copernicus L4 hourly · NOAA Blended Seawinds · CCMP NRT |

It is the history import's sibling, and reuses its machinery wherever the two
agree: what is fetched is written to a file in the application's data
directory and read back like any import, so a project opens the same way
offline as on; the fetch runs when the button is pressed and at no other
moment; the layers and the timeline's new start arrive as one history entry.

## 2. Decisions

Four were put to the user on 2026-10-02 and are fixed here.

1. **SST is display only.** A coloured temperature layer with its own legend
   and readout. It makes no field, cannot be painted or edited, and never
   reaches an export. (Export of SST can follow as a separate change.)
2. **One login, for one product.** CMC SST has no anonymous source and takes
   a NASA Earthdata token, kept in Settings. Copernicus Marine stays
   anonymous, as the history import already is, with its credit line shown.
3. **NOAA Blended Seawinds gets a Rust library.** Its current data exists
   only as NetCDF-4 files, which are HDF5 inside. A C library is forbidden
   (invariant 5, the three-platform build), so a pure-Rust HDF5 reader is
   written for it — a new crate.
4. **A product holds for its own period.** A daily field shows on every step
   of its day and a six-hourly one for its six hours, where an imported
   forecast shows only on its own step. This is an exception to D48, taken
   because the product itself says what span it is valid for.

Six smaller things are **assumed**, and are the first to change if they are
wrong:

- **Geo-Polar Blended** means the day-and-night analysis, not night-only.
- **ASCAT** is one layer: each day's Metop-B and Metop-C passes, ascending
  and descending, merged, the most recent measurement winning where passes
  overlap, and no value where no satellite passed.
- **"CCMP NRT"** is version 2.1 NRT. Version 3.1 is the delayed product and
  trails by a month or more.
- **Vector products land on the 0.25° grid** the history import uses. DUACS
  and the L4 wind are published at 0.125° and are point-sampled down. At
  0.125° a ten-day hourly wind is four gigabytes on disk; at 0.25° it is one.
- **SST is kept at 0.1° at finest.** OSTIA and Geo-Polar are 0.05° and are
  read at every second node, which is a request parameter and not a download.
- **The period starts at 00:00 UTC, N days before today**, and runs to the
  current hour. A daily product then has whole days, and the timeline starts
  on a midnight.

## 3. The dialog

Opened by a new button, `layers:import-nrt`, beside `layers:import-history`.

- **Days**, a `NumberField`. The largest value is what 240 steps of the
  project's own step come to — ten days hourly, thirty three-hourly — since
  240 is the most a project holds (§4.10).
- **The timeline's start.** A checkbox, ticked by default: "Set the
  timeline's start to *period start* UTC" on a project with no date, "Move
  …" on one that has a date, with the current one beside it. Unticked, the
  first field still lands on step 0 (§4.8) and the labels are left alone.
- **The timeline's length.** When the period needs more steps than the
  project has, a second checkbox, ticked: "Lengthen the timeline to N steps".
  Unticked, the fetch stops at the timeline's end, as a history import does.
- **The ten checkboxes**, grouped as above. Each row says what the product
  is, its step and resolution, and its credit. A product that cannot be
  fetched says why in place of its checkbox's hint: CMC with no token set
  ("needs a NASA Earthdata token — Settings"), or a source found stale.
- **What it will cost**, before the button: the number of downloads and an
  estimate of the megabytes written to disk. The count is per *product time*,
  not per step — three days of a daily product is three downloads on any
  timeline.

Progress is the history import's: a real fraction in the status bar, named
by product, counted as fields arrive. **Recent hours that do not exist yet
are not an error.** Every product trails the present by about a day; the
import fetches what is there and each layer says the last time it holds.

## 4. The products

Identifiers are as observed on 2026-10-02. Latency is what the servers
showed that day, not a published commitment.

| Product | Source | Read as | Step | Lands as |
|---|---|---|---|---|
| NOAA OISST v2.1 | NOAA ERDDAP `ncdcOisst21NrtAgg_LonPM180` (preliminary), `ncdcOisst21Agg_LonPM180` (final) | NetCDF-3 subset | daily, 0.25° | SST layer |
| OSTIA | Copernicus `SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001` | ARCO Zarr | daily, 0.05° read at 0.1° | SST layer |
| Geo-Polar Blended | NOAA CoastWatch ERDDAP `noaacwBLENDEDsstDNDaily` | NetCDF-3 subset | daily, 0.05° read at 0.1° | SST layer |
| CMC | NASA PO.DAAC `CMC0.1deg-CMC-L4-GLOB-v3.0` | NetCDF-4 file, bearer token | daily, 0.1° | SST layer |
| DUACS | Copernicus `SEALEVEL_GLO_PHY_L4_NRT_008_046`, the 0.125° dataset, `ugos`/`vgos` | ARCO Zarr | daily | current layer |
| MULTIOBS | Copernicus `MULTIOBS_GLO_PHY_MYNRT_015_003`, hourly NRT, `uo`/`vo` at the surface | ARCO Zarr — **already read** as GlobCurrent | hourly | current layer |
| ASCAT L3 | Copernicus `WIND_GLO_PHY_L3_NRT_012_002`, Metop-B and -C, both passes, 0.25° | ARCO Zarr | daily swaths | wind layer |
| L4 hourly wind | Copernicus `WIND_GLO_PHY_L4_NRT_012_004` | ARCO Zarr | hourly, 0.125° read at 0.25° | wind layer |
| Blended Seawinds | NOAA CoastWatch NRT files, `NBSv02_wind_6hourly_…_nrt.nc` | NetCDF-4 file | 6-hourly, 0.25° | wind layer |
| CCMP V2.1 NRT | NOAA ERDDAP `pifscCcmpDailyV21NRT_LonPM180` | NetCDF-3 subset | 6-hourly, 0.25° | wind layer |

Three readers cover all ten, and one of them exists.

## 5. The readers

### 5.1 Copernicus Marine (five products)

The stores are Zarr v2 with Blosc-LZ4 chunks, read anonymously over HTTPS —
what `ve-zarr` already does for GlobCurrent, with its own Blosc decoder.
What is new:

- **Discovery through the public STAC catalogue**, product → dataset →
  the `timeChunked` asset. The bucket numbers and the dataset version
  suffixes change, and GlobCurrent's two store addresses are constants
  today; they move to discovery with the rest. One store per time chunk is
  what "the last N days" wants.
- **One generic source** in place of `globcurrent.rs`'s particular one: the
  variable names, the packed integers' scale and offset, the fill value, the
  time axis's epoch (it differs per dataset) and the depth level come from
  the store's own metadata and a small per-product description.
- **Regridding** through `regrid::CellGrid`, generalised from GlobCurrent's
  one grid to a cell-centred grid of any spacing.

### 5.2 NOAA ERDDAP (three products)

A `griddap` request names the variable, the times and the stride, and the
server answers with exactly that subset as a NetCDF-3 classic file — a
header and big-endian arrays, a few hundred lines to read and no HDF5.

- **Staleness is checked before fetching.** These dataset identifiers are
  kept by regional nodes and go quiet without notice: the Blended Seawinds
  NRT datasets on the same server stopped in 2023 and are still listed. The
  time axis's last value is read first, and a dataset more than a few days
  behind is reported in the dialog rather than fetched as nothing.
- OISST reads the final dataset where it has the day and the preliminary one
  where it does not — the preference `GlobCurrentStore` already gives final
  current data.

### 5.3 A pure-Rust HDF5 reader (two products)

A new crate, **`ve-hdf5`**: reads, never writes, reaches no network, links
no C. It exists for Blended Seawinds and CMC, whose current data is published
only as NetCDF-4.

It begins with a **survey of the real files** — which superblock version,
which object-header version, how groups and chunks are indexed, which
filters — because the HDF5 format is large and a NetCDF-4 file uses a small,
regular part of it. The reader covers what the survey finds and refuses the
rest by name. Expected, and to be confirmed by the survey:

- superblock, object headers, the root group's links;
- dataspace, fixed-point and IEEE floating datatypes, either byte order;
- contiguous and chunked layouts, the chunk index, the fill value;
- the shuffle and deflate filters (`flate2` is already in the workspace, on
  its Rust backend), and the Fletcher-32 checksum;
- numeric and string attributes: `scale_factor`, `add_offset`, `_FillValue`,
  `units`.

Not attempted: writing, compound and variable-length data, SZIP, virtual
and external datasets.

**Its reference is not itself.** Fixtures are small files written by the
reference library, with every value also dumped by that library's own tools,
and the first megabyte of real product files with values read from the
publisher's own listing — the rule `ve-grib`'s decoder fixtures follow.

### 5.4 The Earthdata token

One field in Settings, "NASA Earthdata token", a plain string in the
settings file the person already owns — the precedent is the MCP service's
token. It is sent as a bearer credential to PO.DAAC and to nowhere else, is
never written to a log, and never enters a project file. The granule list
comes from NASA's catalogue anonymously; only the file download carries the
token. The download redirects through NASA's login host, and the HTTP client
drops a credential on a cross-host redirect by design, so the redirect is
followed by hand.

## 6. How a product becomes a layer

### 6.1 Wind and current

Each implements `ve_zarr::FieldSource` and goes through the history import's
path unchanged: the fields are packed into a GRIB2 file in the data
directory, sixteen bits with a bitmap where the product has no value — land
for a current, the gaps between swaths for ASCAT — and the layer reads that
file as an imported forecast does. Speed filter, eraser, edit objects,
macros and export all work, because the layer *is* an imported layer. Its
provenance records the product and the times, as a history layer's does.

### 6.2 Holding (the exception to D48)

The layer's provenance carries the product's **period** — 24, 6 or 1 hours —
and a new lookup, `RasterSequence::frame_within`, answers with the newest
frame whose period contains the step. `frame_at` keeps its exact match, and
`Layer::file_frame` is the one place that chooses between them.

- A daily product's frame is stamped 00:00 UTC of its day when it is
  written, whatever hour the publisher stamps it, so its period is the
  calendar day.
- **Past the last frame's period there is nothing.** Holding stops where the
  product's own validity does; the failure D48 was written against — a short
  file standing in for the rest of a timeline — stays impossible.
- A project *coarser* than the product still strides: a daily project reads
  one hour in twenty-four of an hourly product and fetches no other.
- Steps that share a frame share its hash, so they share tiles and the
  render cache is right by construction (D59's reasoning).
- **The export carries the held field.** A daily current exported on an
  hourly timeline is the same field in twenty-four messages. That is what
  holding means, and the spec will say so.

Imported GRIB files and history layers keep D48 exactly: the period is a
property of a near-real-time layer and nothing else has one.

### 6.3 SST: a display-only scalar layer

A new `LayerSource`, beside the image and GIS ones and under the same rule:
it reaches no scene, no `FlatScene` hash, no render-cache key and no export.

- **Stored** as a GRIB2 file in the data directory, one water-temperature
  message per day, sixteen bits with a bitmap over land and ice. One
  container for everything fetched, and one a person can open in any GRIB
  viewer. The writer gains a scalar message. The decoder already hands back
  any message with its discipline, category and number; only the import
  step, which recognises wind and current and nothing else, needs to learn
  to find a temperature.
- **Drawn** as RGBA tiles painted in Rust and served through the backdrop
  path (§4.11), addressed by the file's hash, the frame and the colour
  range, so an address is immutable. Sampled bilinearly; no value is
  transparent. It sits in the layer stack where an image layer does, obeys
  the eye, and has no speed filter (the September contract).
- **Held** by the same period rule as §6.2 — it is daily.
- **Legend and readout.** A second legend for temperature when an SST layer
  is visible, and the cursor readout gains the temperature under the
  pointer, sampled in Rust like the field. A **temperature unit** joins the
  global units, °C or °F.
- **Colour range**: −2 to 32 °C, or fitted to the view when Auto scale is on.

## 7. Invariant 5

This is a third exception, on the user's instruction, and is written into
the invariant in the commit that first fetches.

- A new crate, **`ve-nrt`**, holds the product catalogue, the STAC lookup,
  the ERDDAP client and the Earthdata download. It uses `ve-zarr`'s HTTP
  store and decoders; `ve-hdf5` reaches nothing. `ve-app` → `ve-nrt` →
  `ve-zarr`, `ve-hdf5`, `ve-grib`, `ve-core`.
- `check-offline.sh` names the crate and its hosts and allows them nowhere
  else: the Copernicus catalogue and its object store, the two NOAA
  CoastWatch servers, and NASA's catalogue, archive and login host.
- Nothing is fetched on a timer, at startup or to refresh. "Up to now" is
  read from the clock when the button is pressed.
- **Credits are shown**, in the dialog and on the layer: Copernicus Marine's
  required line, GHRSST and the Met Office for OSTIA, NOAA CoastWatch,
  Remote Sensing Systems for CCMP.

## 8. Order of work

Each is a milestone that ships on its own: tests, clippy and the offline
check pass, the text is in nine languages, and `spec.md` is updated in the
same commit. Ordered so that what needs nothing new comes first and the
largest risk comes after everything that does not depend on it.

1. **Holding.** The period on a layer's provenance and in `frame_at`; the
   stride, the hold, the end of the last period and the export, each tested;
   D48's exception recorded in the decisions log.
2. **The button, the dialog and the Copernicus gridded products.** `ve-nrt`
   with STAC discovery and the generic source; MULTIOBS, DUACS and the L4
   hourly wind; days, start time, timeline length, cost and progress;
   invariant 5 and the offline check. *Done when* three days of each lands
   on a new project with the right dates, offline reopening works, and a
   known value at a known place and hour matches the publisher's own.
3. **ASCAT L3.** The merge of four passes and the gaps as a bitmap.
4. **The ERDDAP reader, and CCMP.** NetCDF-3 and the staleness check.
5. **SST.** The scalar layer, its tiles, legend, readout and unit, with
   OISST and Geo-Polar through ERDDAP and OSTIA through Copernicus.
6. **`ve-hdf5`.** The survey, the reader, the fixtures.
7. **NOAA Blended Seawinds**, through it.
8. **The Earthdata token, and CMC SST**, through it.
9. **The MCP tools**: the products with what each holds now, and the import.
   (Every new command is in `mcp/invoke.rs`'s table from the milestone that
   adds it; this is the tools an agent reaches for.)

After milestone 5, eight of the ten products work. The last two wait on the
HDF5 reader.

## 9. Testing

- **Each reader against an independent reference**: a value the publisher's
  own tool or listing gives for a named place and time, not a round trip
  through our own code.
- **Recorded responses, not live servers, in CI.** Each client is tested
  against captured headers and small captured bodies. One `#[ignore]`d live
  test per source fetches on purpose, as `ve-osm`'s does.
- **Geography, both hard cases**: a product across the antimeridian (the
  −180…180 and 0…360 conventions both occur) and at the poles (the node rows
  a cell-centred grid does not have).
- **The dialog's arithmetic**: days to steps to downloads, the 240-step
  bound, a daily product on an hourly timeline.
- **Document rules**: the new sources round-trip through a save, and the
  import is one undo, start time and timeline length included.
- **A size and time budget**, measured: three days of every product on an
  hourly project, reported in megabytes and seconds.

## 10. What this does not do, and what may go wrong

- **No region.** Every product is fetched globally. A bounding box would cut
  the downloads by an order of magnitude and both source types support one,
  but it was not asked for.
- **No refresh.** A layer is the period it was fetched for. Fetching again
  makes new layers.
- **No finer grid for vectors** than 0.25°, and none for SST than 0.1°.
- **The fetched files stay on the machine that fetched them**, as history
  files do: a project moved elsewhere opens with those layers empty.
- **Risks.** The ERDDAP identifiers can go stale, which the staleness check
  reports but cannot fix. Latency is undocumented, so "up to now" will
  usually end about a day short. The HDF5 survey may find a feature that
  makes the reader much larger than expected; it comes after everything
  else for that reason, and the survey's result is reported before the
  reader is built. PO.DAAC's login redirect has not been exercised with a
  real token. And the disk: ten days of ten products is several gigabytes,
  which is why the dialog states the size before the button is pressed.
