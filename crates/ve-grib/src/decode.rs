//! A GRIB2 decoder for import.
//!
//! Reads what real forecast files carry: any number of concatenated messages,
//! regular lat/lon grids (template 3.0) in any scanning order, the common
//! product templates, every packing the forecast centres ship — simple (5.0),
//! complex with and without spatial differencing (5.2, 5.3), JPEG 2000 (5.40)
//! and CCSDS adaptive entropy coding (5.42) — and a bitmap.
//!
//! The section walking and the three integer packings are hand-written. The
//! two compressed packings are not: `hayro-jpeg2000` and `rust-aec` decode
//! them. Both are pure Rust with no C library behind them, so the shipped
//! binary still depends on no external decoder, invariant 5 holds, and the
//! three-platform build needs nothing installed. Every one of those crates
//! gives back the same packed integers template 5.0 stores directly, so the
//! scaling formula below is shared by all five.
//!
//! Not handled, and reported by name rather than misread: PNG packing (5.41),
//! Gaussian, thinned and rotated grids, GRIB edition 1.
//!
//! Octet numbers in comments are the 1-based ones the WMO tables use; the code
//! indexes from 0 within each section, so octet `n` is `section[n - 1]`.
//! Signed fields are sign-magnitude, not two's complement (see writer.rs).

use crate::error::{GribError, Result};
use crate::projection::{Earth, Projection, RotatedPole};
use crate::writer::{ReferenceTime, from_i16_sm, from_i32_sm};

/// Marks a grid node the bitmap left out.
///
/// The same sentinel `ve_core::raster` uses, restated here so this crate's
/// output can be consumed without knowing about rasters.
pub const MISSING: f32 = ve_core::raster::MISSING;

/// Scanning mode flags: octet 72 of template 3.0, and its equivalent in
/// every other template.
///
/// The same eight bits order the nodes of every grid this decoder reads, so
/// one type walks all of them. Four of them matter, and each is a
/// permutation of the same nodes rather than a different set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scan(pub u8);

impl Scan {
    /// Whether consecutive points run in `+i`: west to east on a lat/lon
    /// grid, up the plane's `x` axis on a projected one.
    pub fn i_positive(self) -> bool {
        self.0 & 0x80 == 0
    }
    /// Whether rows run in `+j`: south to north, or up the plane's `y` axis.
    pub fn j_positive(self) -> bool {
        self.0 & 0x40 != 0
    }
    /// Whether adjacent points in the file step in `j` rather than `i`.
    pub fn j_consecutive(self) -> bool {
        self.0 & 0x20 != 0
    }
    /// Whether alternate rows reverse direction.
    pub fn boustrophedon(self) -> bool {
        self.0 & 0x10 != 0
    }

    /// Where the file keeps the node at canonical `(i, j)`.
    ///
    /// Canonical is row-major, `i` increasing along the grid's first axis and
    /// `j` running the *opposite* way to its second: north first on a lat/lon
    /// grid, the top of the plane first on a projected one. That is the
    /// layout `ve_core::raster` wants, and walking the canonical grid asking
    /// the file where each node sits is what turns any of the sixteen
    /// scanning modes into it.
    pub fn file_index(self, i: usize, j: usize, ni: usize, nj: usize) -> usize {
        if self.j_consecutive() {
            // Columns are consecutive: pick the column, then the row within
            // it. Alternate columns run the other way if the boustrophedon
            // flag is set.
            let fi = if self.i_positive() { i } else { ni - 1 - i };
            let reversed = self.j_positive() != (self.boustrophedon() && fi % 2 == 1);
            let fj = if reversed { nj - 1 - j } else { j };
            fi * nj + fj
        } else {
            let fj = if self.j_positive() { nj - 1 - j } else { j };
            let reversed = !self.i_positive() != (self.boustrophedon() && fj % 2 == 1);
            let fi = if reversed { ni - 1 - i } else { i };
            fj * ni + fi
        }
    }
}

/// A regular lat/lon grid, as the file describes it (template 3.0).
///
/// Values are in degrees, signs as written: `la1`/`lo1` is the first point in
/// scanning order and `scan` says which way the scan runs from there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLonGrid {
    /// Points along a parallel.
    pub ni: u32,
    /// Points along a meridian.
    pub nj: u32,
    /// Latitude of the first point.
    pub la1: f64,
    /// Longitude of the first point.
    pub lo1: f64,
    /// Latitude of the last point.
    pub la2: f64,
    /// Longitude of the last point.
    pub lo2: f64,
    /// Longitude increment, always positive; direction is in `scan`.
    pub di: f64,
    /// Latitude increment, always positive; direction is in `scan`.
    pub dj: f64,
    /// Scanning mode flags, octet 72 of template 3.0.
    pub scan: u8,
}

impl LatLonGrid {
    /// The scanning mode, which orders the values.
    pub fn scan(&self) -> Scan {
        Scan(self.scan)
    }
    /// Whether consecutive points run west to east.
    pub fn i_eastward(&self) -> bool {
        self.scan().i_positive()
    }
    /// Whether rows run south to north.
    pub fn j_northward(&self) -> bool {
        self.scan().j_positive()
    }
    /// Whether adjacent points in the file step in `j` rather than `i`.
    pub fn j_consecutive(&self) -> bool {
        self.scan().j_consecutive()
    }
    /// Whether alternate rows reverse direction.
    pub fn boustrophedon(&self) -> bool {
        self.scan().boustrophedon()
    }
    /// Total nodes.
    pub fn point_count(&self) -> usize {
        self.ni as usize * self.nj as usize
    }
}

/// An unstructured grid (template 3.101), as ICON and other icosahedral
/// models use.
///
/// The message says how many values it carries and *which* grid they belong
/// to, and nothing whatever about where any of them is. The cell centres live
/// in a separate file — DWD ships them as `CLAT`/`CLON` messages on this same
/// grid — and the UUID is what ties the two together. Two files with the same
/// UUID are on the same grid; a file whose UUID nothing knows cannot be
/// placed on the earth at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnstructuredGrid {
    /// Number of cells, from the section's common part.
    pub count: u32,
    /// The grid's identity, octets 20-35.
    pub uuid: [u8; 16],
    /// Number of grid used, octets 16-18. ICON global R03B07 is 26.
    pub number_used: u32,
    /// Number of grid in reference, octet 19.
    pub number_in_reference: u8,
}

impl UnstructuredGrid {
    /// The UUID as the lower-case hex DWD and ecCodes print.
    pub fn uuid_hex(&self) -> String {
        self.uuid.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A regular lattice on a map projection, rather than on lat/lon.
///
/// Templates 3.10, 3.20 and 3.30 lay a grid out on the Mercator, polar
/// stereographic and Lambert conformal planes; templates 3.1 and NCEP's
/// 3.32769 lay one out on a sphere whose pole has been moved. All five are
/// the same shape of thing — evenly spaced rows and columns in *some* plane —
/// and this is that thing, with the map back to the earth attached.
///
/// The nodes are given in canonical order: `(i, j)` sits at
/// `(x0 + i·dx, y0 - j·dy)`, with `j` running down the plane, whatever
/// scanning mode the file used. That is the same convention
/// [`Scan::file_index`] converts into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectedGrid {
    /// Columns.
    pub nx: u32,
    /// Rows.
    pub ny: u32,
    /// Plane `x` of column 0.
    pub x0: f64,
    /// Plane `y` of row 0, which is the highest row in the plane.
    pub y0: f64,
    /// Column spacing in the plane, positive.
    pub dx: f64,
    /// Row spacing in the plane, positive.
    pub dy: f64,
    /// Scanning mode flags, which order the file's values.
    pub scan: u8,
    /// The earth the projection is defined on (code table 3.2).
    pub earth: Earth,
    /// The map between the plane and the earth.
    pub projection: Projection,
    /// Whether `u` and `v` are resolved along the grid's own axes rather
    /// than along east and north (flag table 3.5, bit 5).
    ///
    /// Most regional models set this, and reading such a message as if it
    /// were earth-resolved is a silent error of tens of degrees.
    pub grid_relative: bool,
}

impl ProjectedGrid {
    /// Total nodes.
    pub fn point_count(&self) -> usize {
        self.nx as usize * self.ny as usize
    }

    /// The scanning mode, which orders the values.
    pub fn scan(&self) -> Scan {
        Scan(self.scan)
    }

    /// The projection with its constants worked out, for evaluating in bulk.
    ///
    /// A resample walks millions of nodes; rebuilding a Lambert cone at each
    /// one is most of the run. [`Self::locate`] and [`Self::position`] are
    /// the convenient forms, for a point or two.
    pub fn prepared(&self) -> crate::projection::Prepared {
        self.projection.prepared(&self.earth)
    }

    /// The plane position of the node at canonical `(i, j)`.
    pub fn node(&self, i: u32, j: u32) -> (f64, f64) {
        (
            self.x0 + f64::from(i) * self.dx,
            self.y0 - f64::from(j) * self.dy,
        )
    }

    /// Where a position on the earth falls, in fractional node indices.
    pub fn locate(&self, lat: f64, lon: f64) -> (f64, f64) {
        self.locate_with(&self.prepared(), lat, lon)
    }

    /// [`Self::locate`], with the projection already prepared.
    pub fn locate_with(
        &self,
        prepared: &crate::projection::Prepared,
        lat: f64,
        lon: f64,
    ) -> (f64, f64) {
        let (x, y) = prepared.forward(lat, lon);
        ((x - self.x0) / self.dx, (self.y0 - y) / self.dy)
    }

    /// Where the node at canonical `(i, j)` is on the earth.
    pub fn position(&self, i: u32, j: u32) -> (f64, f64) {
        let (x, y) = self.node(i, j);
        self.projection.inverse(&self.earth, x, y)
    }

    /// The grid's spacing in degrees of latitude, which is what choosing a
    /// project resolution compares against (spec §4.8).
    ///
    /// The rotated grids already state theirs in degrees. The three map
    /// projections state metres, and a degree of latitude is a degree of
    /// latitude wherever the grid happens to be.
    pub fn nominal_spacing_deg(&self) -> f64 {
        match self.projection {
            Projection::Rotated { .. } => self.dx.min(self.dy),
            _ => {
                let per_degree = std::f64::consts::PI * self.earth.a / 180.0;
                self.dx.min(self.dy) / per_degree
            }
        }
    }
}

/// The grid a message's values sit on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grid {
    /// A regular lat/lon lattice: the values carry their own geometry.
    LatLon(LatLonGrid),
    /// An unstructured grid: the values are a bare list, and where each one
    /// sits has to come from the grid's own definition (§4.8).
    Unstructured(UnstructuredGrid),
    /// A lattice on a map projection: the values carry their geometry too,
    /// but only through a projection (§4.8).
    Projected(ProjectedGrid),
}

impl Grid {
    /// Total nodes the message covers.
    pub fn point_count(&self) -> usize {
        match self {
            Self::LatLon(grid) => grid.point_count(),
            Self::Unstructured(grid) => grid.count as usize,
            Self::Projected(grid) => grid.point_count(),
        }
    }

    /// The lat/lon lattice, if this is one.
    pub fn lat_lon(&self) -> Option<&LatLonGrid> {
        match self {
            Self::LatLon(grid) => Some(grid),
            _ => None,
        }
    }

    /// The unstructured grid, if this is one.
    pub fn unstructured(&self) -> Option<&UnstructuredGrid> {
        match self {
            Self::Unstructured(grid) => Some(grid),
            _ => None,
        }
    }

    /// The projected lattice, if this is one.
    pub fn projected(&self) -> Option<&ProjectedGrid> {
        match self {
            Self::Projected(grid) => Some(grid),
            _ => None,
        }
    }
}

/// What a message is, before its values are decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Zero-based position in the file.
    pub index: usize,
    /// Product discipline: 0 meteorological, 10 oceanographic.
    pub discipline: u8,
    /// Originating centre.
    pub centre: u16,
    /// Reference time from section 1.
    pub reference_time: ReferenceTime,
    /// Product definition template number.
    pub product_template: u16,
    /// Parameter category within the discipline.
    pub category: u8,
    /// Parameter number within the category.
    pub number: u8,
    /// Forecast offset from the reference time, in hours.
    pub forecast_hours: f64,
    /// Type of the first fixed surface.
    pub surface_type: u8,
    /// Value of the first fixed surface, scale applied.
    pub surface_value: f64,
    /// The grid.
    pub grid: Grid,
    /// Data representation template number.
    pub packing_template: u16,
}

impl Header {
    /// Seconds since the Unix epoch at which the field is valid.
    pub fn valid_unix_s(&self) -> i64 {
        let r = self.reference_time;
        let days = days_from_civil(i64::from(r.year), u32::from(r.month), u32::from(r.day));
        let seconds = days * 86_400
            + i64::from(r.hour) * 3600
            + i64::from(r.minute) * 60
            + i64::from(r.second);
        seconds + (self.forecast_hours * 3600.0).round() as i64
    }
}

/// A decoded message: its header and its values in the file's own order.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// Everything but the data.
    pub header: Header,
    /// One value per grid node in scanning order, [`MISSING`] where the bitmap
    /// left one out.
    pub values: Vec<f32>,
}

/// A message the decoder could not use, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Zero-based position in the file.
    pub index: usize,
    /// What was wrong with it.
    pub reason: String,
}

/// Everything read from one file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decoded {
    /// Messages that decoded, in file order.
    pub messages: Vec<Message>,
    /// Messages that did not, with reasons for the user.
    pub skipped: Vec<Skipped>,
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// Howard Hinnant's `days_from_civil`, which is exact for every date GRIB can
/// express, so no calendar crate is needed.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// --- Byte helpers -----------------------------------------------------------

fn malformed(what: impl Into<String>) -> GribError {
    GribError::Malformed(what.into())
}

fn unsupported(what: impl Into<String>) -> GribError {
    GribError::Unsupported(what.into())
}

fn byte(s: &[u8], at: usize) -> Result<u8> {
    s.get(at)
        .copied()
        .ok_or_else(|| malformed(format!("section too short for octet {}", at + 1)))
}

fn u16_at(s: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes([byte(s, at)?, byte(s, at + 1)?]))
}

fn u32_at(s: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes([
        byte(s, at)?,
        byte(s, at + 1)?,
        byte(s, at + 2)?,
        byte(s, at + 3)?,
    ]))
}

fn i32_sm_at(s: &[u8], at: usize) -> Result<i32> {
    Ok(from_i32_sm(u32_at(s, at)?))
}

fn i16_sm_at(s: &[u8], at: usize) -> Result<i16> {
    Ok(from_i16_sm(u16_at(s, at)?))
}

/// Reads fixed-width unsigned values from a bit stream, MSB first.
struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    /// Reads `width` bits, up to 32.
    fn read(&mut self, width: u32) -> Result<u32> {
        if width == 0 {
            return Ok(0);
        }
        if width > 32 {
            return Err(unsupported(format!("{width}-bit packed values")));
        }
        let end = self.bit + width as usize;
        if end > self.data.len() * 8 {
            return Err(malformed("packed data ends before the last value"));
        }
        let mut value = 0u64;
        let mut at = self.bit;
        let mut remaining = width as usize;
        while remaining > 0 {
            let byte = u64::from(self.data[at / 8]);
            let offset = at % 8;
            let take = (8 - offset).min(remaining);
            let bits = (byte >> (8 - offset - take)) & ((1 << take) - 1);
            value = (value << take) | bits;
            at += take;
            remaining -= take;
        }
        self.bit = end;
        Ok(value as u32)
    }

    /// Skips to the next octet boundary.
    fn align(&mut self) {
        self.bit = self.bit.div_ceil(8) * 8;
    }
}

// --- Message scanning -------------------------------------------------------

/// The sections of one message, by number.
struct Sections<'a> {
    s1: &'a [u8],
    s3: &'a [u8],
    s4: &'a [u8],
    s5: &'a [u8],
    s6: &'a [u8],
    s7: &'a [u8],
}

/// Splits a file into its messages, without decoding them.
///
/// Anything between messages that is not `GRIB` — padding, a stray newline —
/// is skipped, as decoders conventionally do.
fn split_messages(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 16 <= bytes.len() {
        if &bytes[at..at + 4] != b"GRIB" {
            at += 1;
            continue;
        }
        let edition = bytes[at + 7];
        if edition != 2 {
            return Err(unsupported(format!(
                "GRIB edition {edition}; only GRIB2 is supported"
            )));
        }
        let total = u64::from_be_bytes(
            bytes[at + 8..at + 16]
                .try_into()
                .map_err(|_| malformed("section 0"))?,
        ) as usize;
        if total < 16 + 4 || at + total > bytes.len() {
            return Err(malformed(format!(
                "message at byte {at} declares {total} bytes, past the end of the file"
            )));
        }
        let message = &bytes[at..at + total];
        if &message[total - 4..] != b"7777" {
            return Err(malformed(format!(
                "message at byte {at} lacks its end marker"
            )));
        }
        out.push(message);
        at += total;
    }
    if out.is_empty() {
        return Err(malformed("no GRIB2 message found in the file"));
    }
    Ok(out)
}

fn sections(message: &[u8]) -> Result<Sections<'_>> {
    let (mut s1, mut s3, mut s4, mut s5, mut s6, mut s7) = (None, None, None, None, None, None);
    let mut at = 16;
    let end = message.len() - 4;
    while at < end {
        let length = u32_at(message, at)? as usize;
        let number = byte(message, at + 4)?;
        if length < 5 || at + length > end {
            return Err(malformed(format!("section {number} length {length}")));
        }
        let section = &message[at..at + length];
        match number {
            1 => s1 = Some(section),
            2 => {}
            3 => s3 = Some(section),
            4 => s4 = Some(section),
            5 => s5 = Some(section),
            6 => s6 = Some(section),
            7 => s7 = Some(section),
            other => return Err(malformed(format!("unknown section number {other}"))),
        }
        at += length;
    }
    fn need(s: Option<&[u8]>, n: u8) -> Result<&[u8]> {
        s.ok_or_else(|| malformed(format!("section {n} is missing")))
    }
    Ok(Sections {
        s1: need(s1, 1)?,
        s3: need(s3, 3)?,
        s4: need(s4, 4)?,
        s5: need(s5, 5)?,
        s6: need(s6, 6)?,
        s7: need(s7, 7)?,
    })
}

/// Section 3: which template, and then that template.
///
/// Public because a grid definition is the one part of a message worth
/// reading on its own — it is a few dozen octets, it is what tells a caller
/// whether a file can be placed on the earth at all, and it is what the
/// grid tests assert against.
pub fn grid_of(s3: &[u8]) -> Result<Grid> {
    // Octet 6: source of grid definition. Anything but 0 means the grid is not
    // described by a template at all.
    if byte(s3, 5)? != 0 {
        return Err(unsupported("a grid not defined by a template"));
    }
    // Octet 11: length of the optional list of numbers per row. Non-zero is a
    // quasi-regular (thinned) grid.
    if byte(s3, 10)? != 0 {
        return Err(unsupported("a thinned (quasi-regular) grid"));
    }
    match u16_at(s3, 12)? {
        0 => lat_lon_of(s3).map(Grid::LatLon),
        101 => unstructured_of(s3).map(Grid::Unstructured),
        template @ (1 | 10 | 20 | 30 | 32769) => projected_of(s3, template).map(Grid::Projected),
        40 => Err(unsupported("a Gaussian grid (template 3.40)")),
        other => Err(unsupported(format!("grid definition template 3.{other}"))),
    }
}

/// The angle unit templates 3.0, 3.1 and 3.32769 state their coordinates in.
///
/// Zero or missing in either field means micro-degrees, which is what every
/// file this decoder has met uses.
fn angle_unit(s3: &[u8], basic_at: usize) -> Result<f64> {
    let basic = u32_at(s3, basic_at)?;
    let subdivisions = u32_at(s3, basic_at + 4)?;
    Ok(
        if basic == 0 || basic == u32::MAX || subdivisions == 0 || subdivisions == u32::MAX {
            1e-6
        } else {
            f64::from(basic) / f64::from(subdivisions)
        },
    )
}

/// Code table 3.2, octets 15-30: the earth every template but 3.101 names the
/// same way.
fn earth_of(s3: &[u8]) -> Result<Earth> {
    Earth::of_code(
        byte(s3, 14)?,
        byte(s3, 15)?,
        u32_at(s3, 16)?,
        byte(s3, 20)?,
        u32_at(s3, 21)?,
        byte(s3, 25)?,
        u32_at(s3, 26)?,
    )
}

/// Octets 31-38: the two counts, checked against what section 3 declared.
fn extent_of(s3: &[u8]) -> Result<(u32, u32)> {
    let ni = u32_at(s3, 30)?;
    let nj = u32_at(s3, 34)?;
    if ni == 0 || nj == 0 || ni == u32::MAX || nj == u32::MAX {
        return Err(unsupported("a grid with an unspecified number of points"));
    }
    let declared = u32_at(s3, 6)?;
    if u64::from(declared) != u64::from(ni) * u64::from(nj) {
        return Err(malformed(format!(
            "grid declares {declared} points but is {ni} x {nj}"
        )));
    }
    Ok((ni, nj))
}

/// Section 3, template 3.0.
fn lat_lon_of(s3: &[u8]) -> Result<LatLonGrid> {
    // Octets 39-46: basic angle and its subdivisions.
    let unit = angle_unit(s3, 38)?;
    let (ni, nj) = extent_of(s3)?;

    let la1 = f64::from(i32_sm_at(s3, 46)?) * unit;
    let lo1 = f64::from(i32_sm_at(s3, 50)?) * unit;
    let flags = byte(s3, 54)?;
    let la2 = f64::from(i32_sm_at(s3, 55)?) * unit;
    let lo2 = f64::from(i32_sm_at(s3, 59)?) * unit;
    let scan = byte(s3, 71)?;

    // Octet 55, bits 3 and 4 (0x20, 0x10): whether the increments are given.
    // When they are not, they follow from the corners.
    let i_given = flags & 0x20 != 0;
    let j_given = flags & 0x10 != 0;
    let di = if i_given {
        f64::from(u32_at(s3, 63)?) * unit
    } else if ni > 1 {
        let span = (lo2 - lo1).rem_euclid(360.0);
        let span = if scan & 0x80 == 0 { span } else { 360.0 - span };
        span / f64::from(ni - 1)
    } else {
        0.0
    };
    let dj = if j_given {
        f64::from(u32_at(s3, 67)?) * unit
    } else if nj > 1 {
        (la2 - la1).abs() / f64::from(nj - 1)
    } else {
        0.0
    };
    if !(di > 0.0 && dj > 0.0) && (ni > 1 || nj > 1) {
        return Err(malformed(format!("grid increments {di} x {dj}")));
    }
    if !(-90.0..=90.0).contains(&la1) || !(-90.0..=90.0).contains(&la2) {
        return Err(malformed(format!("grid latitudes {la1} to {la2}")));
    }

    Ok(LatLonGrid {
        ni,
        nj,
        la1,
        lo1,
        la2,
        lo2,
        di,
        dj,
        scan,
    })
}

/// Section 3, templates 3.1, 3.10, 3.20, 3.30 and NCEP's 3.32769.
///
/// Every one of them says the same five things — how many rows and columns,
/// where the first node is, how far apart the nodes are, which way the scan
/// runs, and whether `u`/`v` are resolved along the grid — and then names a
/// projection. What differs is the octets, and whether "where the first node
/// is" is stated on the earth or already in the projection's own plane.
///
/// Octet numbers below are the 1-based WMO ones; `byte` and friends index
/// from 0, so octet `n` is read at `n - 1`.
fn projected_of(s3: &[u8], template: u16) -> Result<ProjectedGrid> {
    let (nx, ny) = extent_of(s3)?;
    let earth = earth_of(s3)?;

    // Where the three map projections keep the fields the two rotated ones
    // keep eight octets later, because those two carry a basic angle and its
    // subdivisions and the map projections do not.
    let (flags, scan) = match template {
        1 | 32769 => (byte(s3, 54)?, byte(s3, 71)?),
        10 => (byte(s3, 46)?, byte(s3, 59)?),
        _ => (byte(s3, 46)?, byte(s3, 64)?),
    };
    // Flag table 3.5, bit 5: components along the grid's axes, not east and
    // north.
    let grid_relative = flags & 0x08 != 0;

    // Each arm produces the projection, the plane position of the file's
    // *first* node, and the plane increments.
    let (projection, x1, y1, dx, dy) = match template {
        1 | 32769 => rotated_of(s3, template, nx, ny, Scan(scan), &earth)?,
        10 => {
            // Octets 39-46: the first point, in micro-degrees; 48-51 the
            // latitude the grid length is true at; 65-72 the increments, in
            // millimetres.
            let la1 = f64::from(i32_sm_at(s3, 38)?) * 1e-6;
            let lo1 = f64::from(i32_sm_at(s3, 42)?) * 1e-6;
            let lad = f64::from(i32_sm_at(s3, 47)?) * 1e-6;
            let la2 = f64::from(i32_sm_at(s3, 51)?) * 1e-6;
            let lo2 = f64::from(i32_sm_at(s3, 55)?) * 1e-6;
            // Octets 61-64 are the angle between the i axis and the equator.
            // Every file that sets it — NCEP's Hawaii and Puerto Rico blend
            // grids write 200° and 295° — is a plain Mercator all the same:
            // their stated corners agree with an unrotated grid to the width
            // of one node and disagree wildly with a rotated one. So it is
            // read and ignored, as ecCodes and wgrib2 both do, rather than
            // trusted into placing the field somewhere it is not.
            let projection = Projection::Mercator { lad, lon_ref: lo1 }.checked(&earth)?;
            let (x1, y1) = projection.forward(&earth, la1, lo1);
            // Octets 65-72 hold the increments in millimetres, but octet 47's
            // bits 3 and 4 say whether they are *given*, and half of NCEP's
            // Mercator grids say they are not. Those files mean their two
            // corners, and their stated increment misses the far corner by a
            // couple of cells; the ones that do set the bits mean the
            // increment, and their corner is the approximate one. Believing
            // whichever the file says it gave is what gets both right.
            let (x2, y2) = projection.forward(&earth, la2, lo2);
            let dx = span_or_given(flags & 0x20 != 0, u32_at(s3, 64)?, x1, x2, nx)?;
            let dy = span_or_given(flags & 0x10 != 0, u32_at(s3, 68)?, y1, y2, ny)?;
            (projection, x1, y1, dx, dy)
        }
        20 | 30 => {
            // Octets 39-46 the first point, 48-51 LaD, 52-55 LoV, 56-63 the
            // increments in millimetres, 64 the projection centre.
            let la1 = f64::from(i32_sm_at(s3, 38)?) * 1e-6;
            let lo1 = f64::from(i32_sm_at(s3, 42)?) * 1e-6;
            let lad = f64::from(i32_sm_at(s3, 47)?) * 1e-6;
            let lov = f64::from(i32_sm_at(s3, 51)?) * 1e-6;
            let dx = f64::from(u32_at(s3, 55)?) * 1e-3;
            let dy = f64::from(u32_at(s3, 59)?) * 1e-3;
            let centre = byte(s3, 63)?;
            // Flag table 3.5 again: bit 1 picks the pole the plane is
            // tangent at, bit 2 asks for both at once.
            if centre & 0x40 != 0 {
                return Err(unsupported("a bi-polar projection"));
            }
            let north = centre & 0x80 == 0;
            let projection = if template == 20 {
                Projection::PolarStereographic { lov, lad, north }
            } else {
                // Octets 66-73: the two standard parallels, which are what
                // fix the cone. LaD, the latitude the grid length is true
                // at, plays no part in the projection — the increments are
                // plane distances — and the two disagree in real files.
                Projection::LambertConformal {
                    lov,
                    latin1: f64::from(i32_sm_at(s3, 65)?) * 1e-6,
                    latin2: f64::from(i32_sm_at(s3, 69)?) * 1e-6,
                    north,
                }
            }
            .checked(&earth)?;
            let (x1, y1) = projection.forward(&earth, la1, lo1);
            (projection, x1, y1, dx, dy)
        }
        _ => unreachable!("projected_of is only called for the templates above"),
    };

    if !(dx > 0.0 && dy > 0.0) || !dx.is_finite() || !dy.is_finite() {
        return Err(malformed(format!("grid increments {dx} x {dy}")));
    }
    if !x1.is_finite() || !y1.is_finite() {
        return Err(malformed(
            "a grid whose first point does not land on its own projection".to_owned(),
        ));
    }

    // The file's first node is a corner; which corner is what the scan says.
    // Canonical order starts at the low `x`, high `y` one.
    let scan = Scan(scan);
    let x0 = if scan.i_positive() {
        x1
    } else {
        x1 - f64::from(nx - 1) * dx
    };
    let y0 = if scan.j_positive() {
        y1 + f64::from(ny - 1) * dy
    } else {
        y1
    };

    Ok(ProjectedGrid {
        nx,
        ny,
        x0,
        y0,
        dx,
        dy,
        scan: scan.0,
        earth,
        projection,
        grid_relative,
    })
}

/// An increment: the one the file stated, or the one its two corners imply.
///
/// The stated one is in millimetres, which is the unit the three map
/// projections use for a plane distance.
fn span_or_given(given: bool, stated: u32, from: f64, to: f64, count: u32) -> Result<f64> {
    if given || count < 2 {
        return Ok(f64::from(stated) * 1e-3);
    }
    let derived = (to - from).abs() / f64::from(count - 1);
    Ok(if derived > 0.0 && derived.is_finite() {
        derived
    } else {
        f64::from(stated) * 1e-3
    })
}

/// The two rotated lat/lon templates, which name the same rotation two ways.
///
/// Template 3.1 is the WMO one: it states the **southern pole of the
/// projection**, and its first and last points are already in rotated
/// coordinates. NCEP's 3.32769 states the **centre of the grid** instead —
/// the point rotated `(0, 0)` — and gives its first and last points as true
/// latitudes and longitudes, which have to be rotated before they mean an
/// index.
///
/// The plane here is the rotated sphere itself, measured in its own degrees,
/// so `x` is periodic and is taken from the grid's own first meridian: a grid
/// that straddles the rotated antimeridian then has no seam in it.
fn rotated_of(
    s3: &[u8],
    template: u16,
    nx: u32,
    ny: u32,
    scan: Scan,
    earth: &Earth,
) -> Result<(Projection, f64, f64, f64, f64)> {
    let unit = angle_unit(s3, 38)?;
    let la1 = f64::from(i32_sm_at(s3, 46)?) * unit;
    let lo1 = f64::from(i32_sm_at(s3, 50)?) * unit;
    let la2 = f64::from(i32_sm_at(s3, 55)?) * unit;
    let lo2 = f64::from(i32_sm_at(s3, 59)?) * unit;

    if template == 32769 {
        // Octets 56-63 are the centre of the grid and 73-80 its last point,
        // both as true coordinates.
        let pole = RotatedPole::from_centre(la2, lo2);
        let last_lat = f64::from(i32_sm_at(s3, 72)?) * unit;
        let last_lon = f64::from(i32_sm_at(s3, 76)?) * unit;
        let (x1, y1) = pole.to_rotated(la1, lo1);
        let (x2, y2) = pole.to_rotated(last_lat, last_lon);
        if nx < 2 || ny < 2 {
            return Err(malformed("a rotated grid of one row or column"));
        }
        // The increments are *derived* from the two corners rather than read
        // from octets 64-71, which NCEP writes in no unit this decoder can
        // identify: for the one grid that uses this template they are 121 813
        // where the corners say 121 833, and the corners are unambiguous —
        // they are true coordinates, and they put the first and last nodes
        // symmetrically about the stated centre to nine digits.
        let dx = crate::projection::wrap180(x2 - x1).abs() / f64::from(nx - 1);
        let dy = (y2 - y1).abs() / f64::from(ny - 1);
        let projection = Projection::Rotated { pole, lon_ref: x1 }.checked(earth)?;
        return Ok((projection, 0.0, y1, dx, dy));
    }

    // Template 3.1. Octets 73-84: the southern pole, and the angle of
    // rotation about it.
    let lat_sp = f64::from(i32_sm_at(s3, 72)?) * unit;
    let lon_sp = f64::from(i32_sm_at(s3, 76)?) * unit;
    let angle = f64::from(i32_sm_at(s3, 80)?) * unit;
    if angle != 0.0 {
        return Err(unsupported(format!(
            "a rotated grid turned {angle}° about its own pole"
        )));
    }
    if !(-90.0..=90.0).contains(&lat_sp) {
        return Err(malformed(format!(
            "a rotated grid whose southern pole is at latitude {lat_sp}"
        )));
    }
    let pole = RotatedPole::from_south_pole(lat_sp, lon_sp);

    // Octet 55, bits 3 and 4: whether the increments are given. When they are
    // not they follow from the corners, exactly as on a plain lat/lon grid.
    let flags = byte(s3, 54)?;
    let dx = if flags & 0x20 != 0 {
        f64::from(u32_at(s3, 63)?) * unit
    } else if nx > 1 {
        let span = (lo2 - lo1).rem_euclid(360.0);
        let span = if scan.i_positive() {
            span
        } else {
            360.0 - span
        };
        span / f64::from(nx - 1)
    } else {
        0.0
    };
    let dy = if flags & 0x10 != 0 {
        f64::from(u32_at(s3, 67)?) * unit
    } else if ny > 1 {
        (la2 - la1).abs() / f64::from(ny - 1)
    } else {
        0.0
    };
    if !(-90.0..=90.0).contains(&la1) || !(-90.0..=90.0).contains(&la2) {
        return Err(malformed(format!(
            "a rotated grid spanning rotated latitudes {la1} to {la2}"
        )));
    }
    // The rotated plane is measured in its own degrees, so the shape of the
    // earth plays no part in it; it is carried along for the grid to keep.
    let projection = Projection::Rotated { pole, lon_ref: lo1 }.checked(earth)?;
    Ok((projection, 0.0, la1, dx, dy))
}

/// Template 3.101, the general unstructured grid.
///
/// Thirty-five octets in total and only one of them is geometry: the shape of
/// the earth. The rest names the grid. Where the cells are is not in the file
/// and is not derivable from it — ICON's icosahedral mesh is relaxed by a
/// spring solver, so no formula reproduces it — which is why the UUID matters
/// more here than any field in template 3.0.
fn unstructured_of(s3: &[u8]) -> Result<UnstructuredGrid> {
    // Octets 7-10 of the common part: how many cells the message covers.
    let count = u32_at(s3, 6)?;
    if count == 0 {
        return Err(malformed("an unstructured grid of no cells"));
    }
    // Octet 15. Code 6 is the sphere of radius 6 371 229 m, which is the one
    // this app works on throughout (spec 3.1) and the one ICON uses.
    let shape = byte(s3, 14)?;
    if shape != 6 {
        return Err(unsupported(format!(
            "an unstructured grid on shape-of-earth {shape}"
        )));
    }
    // Octets 16-18, then 19: which grid, and which of its references.
    let number_used = (u32::from(byte(s3, 15)?) << 16)
        | (u32::from(byte(s3, 16)?) << 8)
        | u32::from(byte(s3, 17)?);
    let number_in_reference = byte(s3, 18)?;
    // Octets 20-35.
    let uuid: [u8; 16] = s3
        .get(19..35)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| malformed("an unstructured grid without a UUID"))?;

    Ok(UnstructuredGrid {
        count,
        uuid,
        number_used,
        number_in_reference,
    })
}

/// Hours per unit of code table 4.4.
fn hours_per_time_unit(code: u8) -> Result<f64> {
    Ok(match code {
        0 => 1.0 / 60.0,
        1 => 1.0,
        2 => 24.0,
        10 => 3.0,
        11 => 6.0,
        12 => 12.0,
        13 => 1.0 / 3600.0,
        other => {
            return Err(unsupported(format!(
                "forecast time unit code {other} (months, years or longer)"
            )));
        }
    })
}

/// Reads the header of one message.
fn header_of(index: usize, message: &[u8], s: &Sections<'_>) -> Result<Header> {
    let discipline = byte(message, 6)?;

    let reference_time = ReferenceTime {
        year: u16_at(s.s1, 12)?,
        month: byte(s.s1, 14)?,
        day: byte(s.s1, 15)?,
        hour: byte(s.s1, 16)?,
        minute: byte(s.s1, 17)?,
        second: byte(s.s1, 18)?,
    };
    reference_time
        .validate()
        .map_err(|_| malformed(format!("reference time {reference_time:?}")))?;

    let grid = grid_of(s.s3)?;

    // Templates whose first 34 octets share template 4.0's layout: the
    // deterministic forecast, its ensemble and derived forms, and the
    // statistically processed ones. Others put the parameter and time
    // elsewhere and are not guessed at.
    let product_template = u16_at(s.s4, 7)?;
    if !matches!(product_template, 0 | 1 | 2 | 8 | 11 | 12 | 15) {
        return Err(unsupported(format!(
            "product definition template 4.{product_template}"
        )));
    }
    let category = byte(s.s4, 9)?;
    let number = byte(s.s4, 10)?;
    let unit_hours = hours_per_time_unit(byte(s.s4, 17)?)?;
    // Octets 19-22: forecast time. Signed in recent editions of the tables;
    // a negative offset is legal, if unusual.
    let forecast_units = i32_sm_at(s.s4, 18)?;
    let forecast_hours = f64::from(forecast_units) * unit_hours;
    let surface_type = byte(s.s4, 22)?;
    let surface_scale = byte(s.s4, 23)?;
    let surface_raw = u32_at(s.s4, 24)?;
    let surface_value = if surface_raw == u32::MAX || surface_scale == 0xff {
        0.0
    } else {
        // One-octet sign-magnitude, like the wider fields.
        let magnitude = i32::from(surface_scale & 0x7f);
        let scale = if surface_scale & 0x80 != 0 {
            -magnitude
        } else {
            magnitude
        };
        f64::from(from_i32_sm(surface_raw)) / 10f64.powi(scale)
    };

    let packing_template = u16_at(s.s5, 9)?;

    Ok(Header {
        index,
        discipline,
        centre: u16_at(s.s1, 5)?,
        reference_time,
        product_template,
        category,
        number,
        forecast_hours,
        surface_type,
        surface_value,
        grid,
        packing_template,
    })
}

// --- Unpacking --------------------------------------------------------------

/// Shared fields of templates 5.0, 5.2 and 5.3 (octets 12-21).
struct Scaling {
    reference: f64,
    binary: f64,
    decimal: f64,
    bits: u32,
}

impl Scaling {
    fn read(s5: &[u8]) -> Result<Self> {
        let reference =
            f32::from_be_bytes([byte(s5, 11)?, byte(s5, 12)?, byte(s5, 13)?, byte(s5, 14)?]);
        let binary_scale = i16_sm_at(s5, 15)?;
        let decimal_scale = i16_sm_at(s5, 17)?;
        Ok(Self {
            reference: f64::from(reference),
            binary: 2f64.powi(i32::from(binary_scale)),
            decimal: 10f64.powi(i32::from(decimal_scale)),
            bits: u32::from(byte(s5, 19)?),
        })
    }

    /// `Y = (R + X * 2^E) / 10^D`.
    fn value(&self, x: i64) -> f32 {
        ((self.reference + x as f64 * self.binary) / self.decimal) as f32
    }
}

/// Template 5.0.
fn unpack_simple(s5: &[u8], data: &[u8], count: usize) -> Result<Vec<f32>> {
    let scaling = Scaling::read(s5)?;
    if scaling.bits == 0 {
        return Ok(vec![scaling.value(0); count]);
    }
    let mut reader = BitReader::new(data);
    (0..count)
        .map(|_| Ok(scaling.value(i64::from(reader.read(scaling.bits)?))))
        .collect()
}

/// Templates 5.2 and 5.3.
///
/// Values are grouped; each group has a reference, a width in bits and a
/// length, and every value is its group's reference plus a deviation of that
/// width. Template 5.3 additionally stores the values as first- or
/// second-order differences with an overall minimum subtracted, which is
/// undone last. Missing values, when managed, are all-ones in their width.
fn unpack_complex(s5: &[u8], data: &[u8], count: usize, differenced: bool) -> Result<Vec<f32>> {
    let scaling = Scaling::read(s5)?;
    // Octet 22: group splitting method. Only the general one (1) is defined.
    // Octet 23: missing value management.
    let missing_mode = byte(s5, 22)?;
    if missing_mode > 2 {
        return Err(malformed(format!(
            "missing value management {missing_mode}"
        )));
    }
    let group_count = u32_at(s5, 31)? as usize;
    let width_reference = u32::from(byte(s5, 35)?);
    let width_bits = u32::from(byte(s5, 36)?);
    let length_reference = u32_at(s5, 37)?;
    let length_increment = u32::from(byte(s5, 41)?);
    let last_length = u32_at(s5, 42)?;
    let length_bits = u32::from(byte(s5, 46)?);

    let (order, extra_octets) = if differenced {
        (u32::from(byte(s5, 47)?), u32::from(byte(s5, 48)?))
    } else {
        (0, 0)
    };
    if differenced && !(order == 1 || order == 2) {
        return Err(unsupported(format!(
            "spatial differencing of order {order}"
        )));
    }

    let mut reader = BitReader::new(data);

    // Template 7.3, octets 6 onward: the first `order` values and the overall
    // minimum, each `extra_octets` wide with the top bit the sign.
    let mut first = [0i64; 2];
    let mut overall_min = 0i64;
    if differenced {
        if extra_octets == 0 || extra_octets > 4 {
            return Err(malformed(format!(
                "{extra_octets} octets for spatial differencing descriptors"
            )));
        }
        let read_signed = |reader: &mut BitReader<'_>| -> Result<i64> {
            let raw = u64::from(reader.read(extra_octets * 8)?);
            let sign_bit = 1u64 << (extra_octets * 8 - 1);
            let magnitude = (raw & (sign_bit - 1)) as i64;
            Ok(if raw & sign_bit != 0 {
                -magnitude
            } else {
                magnitude
            })
        };
        for slot in first.iter_mut().take(order as usize) {
            *slot = read_signed(&mut reader)?;
        }
        overall_min = read_signed(&mut reader)?;
    }

    // Group references, widths and lengths: three runs, each padded to an
    // octet boundary.
    let mut references = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        references.push(reader.read(scaling.bits)?);
    }
    reader.align();
    let mut widths = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        widths.push(width_reference + reader.read(width_bits)?);
    }
    reader.align();
    // Every group's length is in the stream, the last one's included — and
    // then the last one is overridden by the true length from section 5. The
    // read cannot be skipped for it: doing so leaves the reader `length_bits`
    // short, and the alignment that follows then lands on the wrong octet
    // whenever those bits cross a boundary. The group lengths still sum to
    // the value count, so nothing downstream notices; the data is simply read
    // from an octet earlier. That was live for every complex-packed file
    // whose group count did not happen to make the two alignments agree.
    let mut lengths = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        let length = length_reference + reader.read(length_bits)? * length_increment;
        lengths.push(length as usize);
    }
    if let Some(last) = lengths.last_mut() {
        *last = last_length as usize;
    }
    reader.align();

    let total: usize = lengths.iter().sum();
    if total != count {
        return Err(malformed(format!(
            "groups hold {total} values but {count} were declared"
        )));
    }

    // The packed deviations, a continuous bit stream across groups. `None`
    // is a managed missing value.
    let mut raw: Vec<Option<i64>> = Vec::with_capacity(count);
    let all_ones = |bits: u32| {
        if bits >= 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        }
    };
    for g in 0..group_count {
        let width = widths[g];
        if width > 32 {
            return Err(unsupported(format!("{width}-bit group width")));
        }
        let reference = references[g];
        if width == 0 {
            // Every value is the reference. With missing management, a
            // reference of all ones marks the whole group missing.
            let missing = missing_mode != 0
                && scaling.bits > 0
                && (reference == all_ones(scaling.bits)
                    || (missing_mode == 2 && reference == all_ones(scaling.bits) - 1));
            for _ in 0..lengths[g] {
                raw.push(if missing {
                    None
                } else {
                    Some(i64::from(reference))
                });
            }
            continue;
        }
        let ones = all_ones(width);
        for _ in 0..lengths[g] {
            let deviation = reader.read(width)?;
            let missing = missing_mode != 0
                && (deviation == ones || (missing_mode == 2 && deviation == ones - 1));
            raw.push(if missing {
                None
            } else {
                Some(i64::from(reference) + i64::from(deviation))
            });
        }
    }

    if differenced {
        undo_spatial_differencing(&mut raw, order, first, overall_min);
    }

    Ok(raw
        .into_iter()
        .map(|x| x.map_or(MISSING, |x| scaling.value(x)))
        .collect())
}

/// Reverses template 5.3's differencing in place, skipping missing values.
///
/// The first `order` non-missing values are replaced by the explicitly
/// stored ones; each later value is its stored difference plus the overall
/// minimum plus the reconstruction from the values before it.
fn undo_spatial_differencing(values: &mut [Option<i64>], order: u32, first: [i64; 2], min: i64) {
    let mut present = values.iter_mut().filter_map(|v| v.as_mut());
    if order == 1 {
        let Some(head) = present.next() else { return };
        *head = first[0];
        let mut last = first[0];
        for value in present {
            *value += last + min;
            last = *value;
        }
    } else {
        let (Some(a), Some(b)) = (present.next(), present.next()) else {
            return;
        };
        *a = first[0];
        *b = first[1];
        let mut penultimate = first[0];
        let mut last = first[1];
        for value in present {
            *value += min + 2 * last - penultimate;
            penultimate = last;
            last = *value;
        }
    }
}

/// Template 5.42: CCSDS adaptive entropy coding (CCSDS 121.0-B).
///
/// The entropy coder gives back exactly the integers template 5.0 stores in
/// the clear, so all this adds is the decode and the sample width the
/// decoder chose. That width is derived from the output rather than from the
/// bit count, because the 3-byte flag is advisory: `libaec` and the crate
/// that follows it both ignore it below 17 bits, and every ECMWF file in the
/// reference set sets it while packing 12 bits into two bytes.
fn unpack_ccsds(s5: &[u8], data: &[u8], count: usize) -> Result<Vec<f32>> {
    let scaling = Scaling::read(s5)?;
    if scaling.bits == 0 {
        return Ok(vec![scaling.value(0); count]);
    }
    if scaling.bits > 32 {
        return Err(malformed(format!("{} bits per CCSDS sample", scaling.bits)));
    }
    // Octets 22-25: the compression options mask, the block size, and the
    // reference sample interval. The coder cannot be run without all three.
    let flags = byte(s5, 21)?;
    let block_size = u32::from(byte(s5, 22)?);
    let reference_interval = u32::from(u16_at(s5, 23)?);

    // Bit 0 of the mask. A signed sample would make `X` negative, which the
    // 5.0 formula this shares has no meaning for — its reference is the
    // field's minimum. No centre writes one; if one does, say so rather than
    // reading the sign bit as magnitude.
    if flags & 0x01 != 0 {
        return Err(unsupported(
            "CCSDS packing with signed samples (template 5.42, options bit 1)",
        ));
    }
    // Bit 2 says the coder wrote each sample big-endian. It is the same bit
    // the decoder is given below, so the two cannot disagree about the order.
    let big_endian = flags & 0x04 != 0;

    let params = rust_aec::AecParams::new(
        // Checked above, so this cannot truncate.
        scaling.bits as u8,
        block_size,
        reference_interval,
        rust_aec::flags_from_grib2_ccsds_flags(flags),
    );
    let bytes = rust_aec::decode(data, params, count)
        .map_err(|e| malformed(format!("CCSDS packing (template 5.42): {e:?}")))?;

    if count == 0 {
        return Ok(Vec::new());
    }
    if !bytes.len().is_multiple_of(count) {
        return Err(malformed(format!(
            "CCSDS decode produced {} bytes for {count} samples",
            bytes.len()
        )));
    }
    let width = bytes.len() / count;
    if !(1..=4).contains(&width) {
        return Err(malformed(format!("{width} bytes per CCSDS sample")));
    }
    Ok(bytes
        .chunks_exact(width)
        .map(|sample| {
            let x = if big_endian {
                sample
                    .iter()
                    .fold(0i64, |acc, &b| (acc << 8) | i64::from(b))
            } else {
                sample
                    .iter()
                    .rev()
                    .fold(0i64, |acc, &b| (acc << 8) | i64::from(b))
            };
            scaling.value(x)
        })
        .collect())
}

/// Template 5.40: JPEG 2000 packing.
///
/// Section 7 holds a bare J2K codestream — no JP2 wrapper — of one component
/// whose precision is the bit count. The samples come back as `f32` because
/// the decoder is written for images; every value a GRIB message carries is a
/// non-negative integer below `2^bits`, and `f32` represents those exactly up
/// to 24 bits, which is past anything a centre packs.
///
/// Lossy codestreams (octet 22 = 1) are decoded too. A lossy file's values
/// are approximate, but that is the file's own bargain and not the decoder's
/// to refuse.
fn unpack_jpeg2000(s5: &[u8], data: &[u8], count: usize) -> Result<Vec<f32>> {
    let scaling = Scaling::read(s5)?;
    if scaling.bits == 0 {
        return Ok(vec![scaling.value(0); count]);
    }
    let settings = hayro_jpeg2000::DecodeSettings {
        // A GRIB codestream carries no palette; resolving one would be a
        // lookup into a table that is not there.
        resolve_palette_indices: false,
        strict: false,
        target_resolution: None,
    };
    let image = hayro_jpeg2000::Image::new(data, &settings)
        .map_err(|e| malformed(format!("JPEG 2000 packing (template 5.40): {e:?}")))?;
    let mut context = hayro_jpeg2000::DecoderContext::default();
    let decoded = image
        .decode(&mut context)
        .map_err(|e| malformed(format!("JPEG 2000 packing (template 5.40): {e:?}")))?;

    let components = decoded.components();
    let [component] = components else {
        return Err(unsupported(format!(
            "a JPEG 2000 codestream of {} components",
            components.len()
        )));
    };
    let samples = component.samples();
    if samples.len() < count {
        return Err(malformed(format!(
            "JPEG 2000 codestream holds {} samples for {count} values",
            samples.len()
        )));
    }
    // The codestream is a rectangle and the message a run of values; when a
    // bitmap leaves nodes out the encoder pads the last row, so take the run.
    Ok(samples
        .iter()
        .take(count)
        .map(|&x| scaling.value(x.round() as i64))
        .collect())
}

/// Decodes the data of a message whose header has been read.
fn values_of(header: &Header, s: &Sections<'_>) -> Result<Vec<f32>> {
    let point_count = header.grid.point_count();
    // Section 5, octets 6-9: how many values section 7 actually holds, which
    // is fewer than the grid has when a bitmap leaves nodes out.
    let packed_count = u32_at(s.s5, 5)? as usize;
    let data = s.s7.get(5..).unwrap_or(&[]);

    let packed = match header.packing_template {
        0 => unpack_simple(s.s5, data, packed_count)?,
        2 => unpack_complex(s.s5, data, packed_count, false)?,
        3 => unpack_complex(s.s5, data, packed_count, true)?,
        40 => unpack_jpeg2000(s.s5, data, packed_count)?,
        41 => return Err(unsupported("PNG packing (template 5.41)")),
        42 => unpack_ccsds(s.s5, data, packed_count)?,
        other => {
            return Err(unsupported(format!(
                "data representation template 5.{other}"
            )));
        }
    };

    // Section 6, octet 6: 255 no bitmap, 0 a bitmap follows, 254 a bitmap
    // from an earlier message, 1-253 predefined by the centre.
    match byte(s.s6, 5)? {
        255 => {
            if packed.len() != point_count {
                return Err(malformed(format!(
                    "{} values for a grid of {point_count} points",
                    packed.len()
                )));
            }
            Ok(packed)
        }
        0 => {
            let bitmap = s.s6.get(6..).unwrap_or(&[]);
            if bitmap.len() * 8 < point_count {
                return Err(malformed("bitmap shorter than the grid"));
            }
            let mut out = Vec::with_capacity(point_count);
            let mut next = packed.iter();
            for k in 0..point_count {
                let set = bitmap[k / 8] & (0x80 >> (k % 8)) != 0;
                if set {
                    out.push(*next.next().ok_or_else(|| {
                        malformed("bitmap marks more values present than were packed")
                    })?);
                } else {
                    out.push(MISSING);
                }
            }
            Ok(out)
        }
        254 => Err(unsupported("a bitmap carried over from an earlier message")),
        other => Err(unsupported(format!("a centre-defined bitmap ({other})"))),
    }
}

// --- Public entry points ----------------------------------------------------

/// Decodes every message in `bytes` that `wanted` selects.
///
/// Headers are always parsed; values are unpacked only for messages the
/// caller asks for, which keeps a large multi-parameter file cheap to scan
/// for its wind. A message that cannot be read is reported in `skipped`
/// rather than failing the file — one unsupported field must not stop the
/// others from importing. A file that is not GRIB2 at all is an error.
pub fn read(bytes: &[u8], wanted: impl Fn(&Header) -> bool) -> Result<Decoded> {
    let mut decoded = Decoded::default();
    for (index, message) in split_messages(bytes)?.into_iter().enumerate() {
        let skip = |reason: GribError| Skipped {
            index,
            reason: reason.to_string(),
        };
        let sections = match sections(message) {
            Ok(s) => s,
            Err(err) => {
                decoded.skipped.push(skip(err));
                continue;
            }
        };
        let header = match header_of(index, message, &sections) {
            Ok(h) => h,
            Err(err) => {
                decoded.skipped.push(skip(err));
                continue;
            }
        };
        if !wanted(&header) {
            continue;
        }
        match values_of(&header, &sections) {
            Ok(values) => decoded.messages.push(Message { header, values }),
            Err(err) => decoded.skipped.push(skip(err)),
        }
    }
    Ok(decoded)
}

/// Decodes every message in `bytes`.
pub fn read_all(bytes: &[u8]) -> Result<Decoded> {
    read(bytes, |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bit_reader_crosses_byte_boundaries() {
        // 1010 1100 | 0101 0011 | 1111 0000
        let data = [0xAC, 0x53, 0xF0];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read(3).unwrap(), 0b101);
        assert_eq!(r.read(7).unwrap(), 0b0110001);
        assert_eq!(r.read(6).unwrap(), 0b010011);
        r.align();
        assert_eq!(r.read(4).unwrap(), 0b1111);
        assert!(r.read(8).is_err(), "past the end");
    }

    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 9, 3), 20_699);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }

    #[test]
    fn time_units_convert_to_hours() {
        assert_eq!(hours_per_time_unit(1).unwrap(), 1.0);
        assert_eq!(hours_per_time_unit(0).unwrap() * 60.0, 1.0);
        assert_eq!(hours_per_time_unit(10).unwrap(), 3.0);
        assert_eq!(hours_per_time_unit(2).unwrap(), 24.0);
        assert!(
            hours_per_time_unit(3).is_err(),
            "months are not a forecast step"
        );
    }

    /// First-order differencing, worked by hand: stored differences
    /// `[_, 2, -1, 3]` with min -1 and first value 10 reconstruct to
    /// 10, 11, 9, 11.
    #[test]
    fn first_order_differencing_is_undone() {
        let mut values = vec![Some(0), Some(2), Some(-1), Some(3)];
        undo_spatial_differencing(&mut values, 1, [10, 0], -1);
        assert_eq!(values, vec![Some(10), Some(11), Some(9), Some(11)]);
    }

    /// Second order: v[i] = d[i] + min + 2 v[i-1] - v[i-2].
    #[test]
    fn second_order_differencing_is_undone_and_skips_missing() {
        let mut values = vec![Some(0), None, Some(0), Some(1), Some(-2)];
        undo_spatial_differencing(&mut values, 2, [5, 7], 1);
        // 5, 7, then 1+1+14-5 = 11, then -2+1+22-7 = 14.
        assert_eq!(values, vec![Some(5), None, Some(7), Some(11), Some(14)]);
    }

    /// A hand-built template 5.2 / 7.2 section pair: two groups.
    ///
    /// R = 0, E = 0, D = 0, 8-bit references. Group 1: reference 10, width 2,
    /// length 3, deviations 0 1 2. Group 2: reference 100, width 0, length 2.
    #[test]
    fn complex_packing_is_unpacked() {
        let mut s5 = vec![0u8; 47];
        s5[5..9].copy_from_slice(&5u32.to_be_bytes());
        s5[9..11].copy_from_slice(&2u16.to_be_bytes());
        s5[11..15].copy_from_slice(&0f32.to_be_bytes());
        s5[19] = 8; // bits per reference
        s5[21] = 1; // general group splitting
        s5[22] = 0; // no missing management
        s5[31..35].copy_from_slice(&2u32.to_be_bytes()); // NG
        s5[35] = 0; // width reference
        s5[36] = 2; // bits per width
        s5[37..41].copy_from_slice(&3u32.to_be_bytes()); // length reference
        s5[41] = 1; // length increment
        s5[42..46].copy_from_slice(&2u32.to_be_bytes()); // last group length
        s5[46] = 1; // bits per length

        // References: 10, 100 (8 bits each) -> 0x0A 0x64.
        // Widths: 2, 0 (2 bits each) -> 10 00 -> 0x80 padded.
        // Lengths: *both* groups are in the stream at 1 bit each, the last
        // one's value then discarded for section 5's true length -> 0x00.
        // Values: 00 01 10 (6 bits) -> 0x18 padded.
        let data = [0x0A, 0x64, 0x80, 0x00, 0x18];
        let values = unpack_complex(&s5, &data, 5, false).unwrap();
        assert_eq!(values, vec![10.0, 11.0, 12.0, 100.0, 100.0]);
    }

    #[test]
    fn a_managed_missing_value_is_marked() {
        let mut s5 = vec![0u8; 47];
        s5[9..11].copy_from_slice(&2u16.to_be_bytes());
        s5[19] = 8;
        s5[21] = 1;
        s5[22] = 1; // primary missing management
        s5[31..35].copy_from_slice(&1u32.to_be_bytes());
        s5[36] = 2;
        s5[37..41].copy_from_slice(&0u32.to_be_bytes());
        s5[41] = 1;
        s5[42..46].copy_from_slice(&3u32.to_be_bytes());
        s5[46] = 1;
        // Reference 4, width 2 (10b). The one group is also the last, and
        // its length still occupies its bit in the stream before section 5's
        // true length replaces it -> 0x00. Values 00 11 01.
        let data = [0x04, 0x80, 0x00, 0b0011_0100];
        let values = unpack_complex(&s5, &data, 3, false).unwrap();
        assert_eq!(values[0], 4.0);
        assert!(ve_core::raster::is_missing(values[1]));
        assert_eq!(values[2], 5.0);
    }

    /// The last group's length is read from the stream like every other, and
    /// only *then* replaced by section 5's true length.
    ///
    /// Skipping the read leaves the reader short by `length_bits`, and the
    /// octet alignment that follows then lands one octet early whenever those
    /// bits cross a boundary — so the values are read from the wrong place
    /// while the group lengths still sum correctly and nothing complains.
    /// Here the two groups' 7-bit lengths span two octets where one group's
    /// would span one, which is exactly the case that breaks.
    #[test]
    fn the_last_group_length_still_occupies_the_stream() {
        let mut s5 = vec![0u8; 47];
        s5[5..9].copy_from_slice(&5u32.to_be_bytes());
        s5[9..11].copy_from_slice(&2u16.to_be_bytes());
        s5[11..15].copy_from_slice(&0f32.to_be_bytes());
        s5[19] = 8; // bits per reference
        s5[21] = 1; // general group splitting
        s5[22] = 0; // no missing management
        s5[31..35].copy_from_slice(&2u32.to_be_bytes()); // NG
        s5[35] = 0; // width reference
        s5[36] = 2; // bits per width
        s5[37..41].copy_from_slice(&3u32.to_be_bytes()); // length reference
        s5[41] = 1; // length increment
        s5[42..46].copy_from_slice(&2u32.to_be_bytes()); // last group length
        s5[46] = 7; // bits per length — two of them cross an octet

        // References 10, 100; widths 2 and 0; two 7-bit lengths, both zero,
        // filling two octets; then group 1's deviations 00 01 10.
        let data = [0x0A, 0x64, 0x80, 0x00, 0x00, 0x18];
        let values = unpack_complex(&s5, &data, 5, false).unwrap();
        assert_eq!(values, vec![10.0, 11.0, 12.0, 100.0, 100.0]);
    }

    /// A hand-built section 3 on template 3.101.
    ///
    /// Thirty-five octets: the common part, then the shape of the earth, the
    /// grid number and its reference, then the UUID. Nothing in it says where
    /// a single cell is, which is what the decoder has to represent honestly
    /// rather than inventing a lattice for.
    #[test]
    fn an_unstructured_grid_carries_only_its_identity() {
        let mut s3 = vec![0u8; 35];
        s3[0..4].copy_from_slice(&35u32.to_be_bytes());
        s3[4] = 3; // section number
        s3[5] = 0; // grid defined by a template
        s3[6..10].copy_from_slice(&2_949_120u32.to_be_bytes());
        s3[10] = 0; // no list of numbers per row
        s3[11] = 0;
        s3[12..14].copy_from_slice(&101u16.to_be_bytes());
        s3[14] = 6; // the 6 371 229 m sphere
        s3[15..18].copy_from_slice(&[0x00, 0x00, 0x1a]); // grid 26
        s3[18] = 1; // reference 1
        let uuid: [u8; 16] = [
            0xa2, 0x7b, 0x8d, 0xe6, 0x18, 0xc4, 0x11, 0xe4, 0x82, 0x0a, 0xb5, 0xb0, 0x98, 0xc6,
            0xa5, 0xc0,
        ];
        s3[19..35].copy_from_slice(&uuid);

        let Grid::Unstructured(grid) = grid_of(&s3).unwrap() else {
            panic!("expected an unstructured grid");
        };
        assert_eq!(grid.count, 2_949_120);
        assert_eq!(grid.number_used, 26);
        assert_eq!(grid.number_in_reference, 1);
        assert_eq!(grid.uuid, uuid);
        assert_eq!(grid.uuid_hex(), "a27b8de618c411e4820ab5b098c6a5c0");
        assert_eq!(grid_of(&s3).unwrap().point_count(), 2_949_120);
    }

    /// A mesh on some other earth is refused rather than placed on ours.
    ///
    /// Every position this app computes is on the 6 371 229 m sphere
    /// (spec §3.1); a mesh defined on a different one would be off by
    /// kilometres, which is far worse than not importing.
    #[test]
    fn an_unstructured_grid_on_another_earth_is_refused() {
        let mut s3 = vec![0u8; 35];
        s3[0..4].copy_from_slice(&35u32.to_be_bytes());
        s3[4] = 3;
        s3[6..10].copy_from_slice(&100u32.to_be_bytes());
        s3[12..14].copy_from_slice(&101u16.to_be_bytes());
        s3[14] = 1; // some other sphere
        assert!(matches!(grid_of(&s3), Err(GribError::Unsupported(_))));
    }

    #[test]
    fn a_file_without_grib_is_refused() {
        assert!(matches!(read_all(b"hello"), Err(GribError::Malformed(_))));
        let mut grib1 = b"GRIB\0\0\0\x01".to_vec();
        grib1.extend_from_slice(&[0; 16]);
        assert!(matches!(read_all(&grib1), Err(GribError::Unsupported(_))));
    }
}
