//! GRIB2 message assembly.
//!
//! Each message is built from fixed section bytes with only the varying fields
//! patched in (spec.md 12.2). Sections 0-8, templates 3.0 (regular lat/lon),
//! 4.0 (analysis or forecast at a level) and 5.0 (simple packing).
//!
//! # Two traps this code exists to get right
//!
//! **Signed integers are sign-magnitude, not two's complement.** A latitude of
//! -90 degrees is `0x80000000 | 90_000_000`, not `-90_000_000` as a `i32`. Get
//! this wrong and decoders report a grid somewhere near the north pole with a
//! nonsensical extent.
//!
//! **The grid starts at longitude 0, not -180.** Scanning mode 0 runs west to
//! east from `Lo1` and north to south from `La1`, so the first value is at
//! (0 degrees east, 90 degrees north) and the row wraps through the
//! antimeridian. Everything inside the application works in [-180, 180); this
//! is the one place that converts.

use std::io::Write;

use crate::error::{GribError, Result};
use crate::packing::{self, Packed};

/// Missing-value marker for a one-octet field.
const MISSING_U8: u8 = 0xff;
/// Missing-value marker for a four-octet field.
const MISSING_U32: u32 = 0xffff_ffff;

/// Which field a message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parameter {
    /// Eastward wind at 10 m. Discipline 0, UGRD.
    WindU,
    /// Northward wind at 10 m. Discipline 0, VGRD.
    WindV,
    /// Eastward surface current. Discipline 10, UOGRD.
    CurrentU,
    /// Northward surface current. Discipline 10, VOGRD.
    CurrentV,
}

impl Parameter {
    /// GRIB2 discipline.
    pub fn discipline(self) -> u8 {
        match self {
            Self::WindU | Self::WindV => 0,
            Self::CurrentU | Self::CurrentV => 10,
        }
    }

    /// Parameter category within the discipline.
    pub fn category(self) -> u8 {
        match self {
            // Momentum.
            Self::WindU | Self::WindV => 2,
            // Currents.
            Self::CurrentU | Self::CurrentV => 1,
        }
    }

    /// Parameter number within the category.
    pub fn number(self) -> u8 {
        match self {
            Self::WindU | Self::CurrentU => 2,
            Self::WindV | Self::CurrentV => 3,
        }
    }

    /// `(type of fixed surface, scale factor, scaled value)`.
    pub fn surface(self) -> (u8, u8, u32) {
        match self {
            // 103: specified height above ground, 10 m.
            Self::WindU | Self::WindV => (103, 0, 10),
            // 160: depth below sea surface, 0 m.
            Self::CurrentU | Self::CurrentV => (160, 0, 0),
        }
    }

    /// Short name, for logs and test output.
    pub fn short_name(self) -> &'static str {
        match self {
            Self::WindU => "UGRD",
            Self::WindV => "VGRD",
            Self::CurrentU => "UOGRD",
            Self::CurrentV => "VOGRD",
        }
    }
}

/// A global regular lat/lon grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSpec {
    /// Points along a parallel.
    pub ni: u32,
    /// Points along a meridian.
    pub nj: u32,
    /// Grid spacing in micro-degrees.
    pub micro_degrees: u32,
}

impl GridSpec {
    /// Total grid points.
    pub fn point_count(self) -> u64 {
        u64::from(self.ni) * u64::from(self.nj)
    }

    /// Grid positions in GRIB scanning order.
    ///
    /// West to east from longitude 0, north to south from latitude 90.
    /// Longitudes come back normalised to [-180, 180) for sampling, but the
    /// *order* is the file's, starting at the prime meridian.
    pub fn points(self) -> impl Iterator<Item = (f64, f64)> {
        let delta = f64::from(self.micro_degrees) / 1e6;
        (0..self.nj).flat_map(move |j| {
            let lat = 90.0 - f64::from(j) * delta;
            (0..self.ni).map(move |i| {
                let lon = f64::from(i) * delta;
                let normalised = if lon >= 180.0 { lon - 360.0 } else { lon };
                (normalised, lat.clamp(-90.0, 90.0))
            })
        })
    }
}

/// Reference time, as GRIB2 stores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceTime {
    /// Four-digit year.
    pub year: u16,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
    /// Minute, 0-59.
    pub minute: u8,
    /// Second, 0-59.
    pub second: u8,
}

impl ReferenceTime {
    /// Checks the fields are in range.
    pub fn validate(self) -> Result<()> {
        let ok = (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60;
        if ok {
            Ok(())
        } else {
            Err(GribError::UnsupportedGrid(format!(
                "invalid reference time {self:?}"
            )))
        }
    }
}

/// Everything a single message needs beyond its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSpec {
    /// Which field.
    pub parameter: Parameter,
    /// The grid.
    pub grid: GridSpec,
    /// Forecast reference time.
    pub reference_time: ReferenceTime,
    /// Hours from the reference time.
    pub forecast_hour: u32,
    /// Originating centre. 255 means missing.
    pub centre: u16,
    /// Bits per packed value (spec.md 12.3, M19). [`packing::BITS_PER_VALUE`]
    /// unless the user chose otherwise.
    pub bits: u8,
}

// --- Byte helpers -----------------------------------------------------------

fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// Writes a signed 16-bit value in GRIB2's sign-magnitude form.
fn put_i16_sm(out: &mut Vec<u8>, value: i16) {
    let magnitude = value.unsigned_abs();
    let sign = if value < 0 { 0x8000 } else { 0 };
    put_u16(out, sign | magnitude);
}

/// Writes a signed 32-bit value in GRIB2's sign-magnitude form.
fn put_i32_sm(out: &mut Vec<u8>, value: i32) {
    let magnitude = value.unsigned_abs();
    let sign = if value < 0 { 0x8000_0000 } else { 0 };
    put_u32(out, sign | magnitude);
}

/// Reads GRIB2 sign-magnitude back. Used by the in-test reader.
pub fn from_i32_sm(raw: u32) -> i32 {
    let magnitude = (raw & 0x7fff_ffff) as i32;
    if raw & 0x8000_0000 != 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// Reads a 16-bit sign-magnitude value.
pub fn from_i16_sm(raw: u16) -> i16 {
    let magnitude = (raw & 0x7fff) as i16;
    if raw & 0x8000 != 0 {
        -magnitude
    } else {
        magnitude
    }
}

// --- Sections ---------------------------------------------------------------

fn section1(spec: &MessageSpec) -> Vec<u8> {
    let mut out = Vec::with_capacity(21);
    put_u32(&mut out, 21);
    put_u8(&mut out, 1);
    put_u16(&mut out, spec.centre);
    put_u16(&mut out, 0); // sub-centre
    put_u8(&mut out, 2); // master tables version
    put_u8(&mut out, 0); // local tables version
    put_u8(&mut out, 1); // significance of reference time: start of forecast
    put_u16(&mut out, spec.reference_time.year);
    put_u8(&mut out, spec.reference_time.month);
    put_u8(&mut out, spec.reference_time.day);
    put_u8(&mut out, spec.reference_time.hour);
    put_u8(&mut out, spec.reference_time.minute);
    put_u8(&mut out, spec.reference_time.second);
    put_u8(&mut out, 2); // production status: research
    put_u8(&mut out, 1); // type of data: forecast
    out
}

fn section3(grid: GridSpec) -> Vec<u8> {
    let mut out = Vec::with_capacity(72);
    put_u32(&mut out, 72);
    put_u8(&mut out, 3);
    put_u8(&mut out, 0); // grid definition from template
    put_u32(&mut out, grid.point_count() as u32);
    put_u8(&mut out, 0); // no optional point list
    put_u8(&mut out, 0); // no list interpretation
    put_u16(&mut out, 0); // template 3.0: regular lat/lon

    // Shape of earth 6 implies a radius of 6,371,229 m, which is the same
    // sphere the application draws on (spec.md 3.1), so the radius fields are
    // left missing rather than restated.
    put_u8(&mut out, 6);
    put_u8(&mut out, MISSING_U8);
    put_u32(&mut out, MISSING_U32);
    put_u8(&mut out, MISSING_U8);
    put_u32(&mut out, MISSING_U32);
    put_u8(&mut out, MISSING_U8);
    put_u32(&mut out, MISSING_U32);

    put_u32(&mut out, grid.ni);
    put_u32(&mut out, grid.nj);
    put_u32(&mut out, 0); // basic angle: 0 means units of 1e-6 degrees
    put_u32(&mut out, MISSING_U32); // subdivisions of the basic angle

    put_i32_sm(&mut out, 90_000_000); // La1
    put_i32_sm(&mut out, 0); // Lo1: the prime meridian
    // 0x30: i and j increments are given, and u/v are earth-relative rather
    // than grid-relative. Without the latter, consumers rotate the vectors.
    put_u8(&mut out, 0x30);
    put_i32_sm(&mut out, -90_000_000); // La2
    put_i32_sm(&mut out, 360_000_000i32 - grid.micro_degrees as i32); // Lo2
    put_u32(&mut out, grid.micro_degrees); // Di
    put_u32(&mut out, grid.micro_degrees); // Dj
    put_u8(&mut out, 0x00); // scanning mode: +i, -j, i consecutive
    out
}

fn section4(spec: &MessageSpec) -> Vec<u8> {
    let (surface_type, surface_scale, surface_value) = spec.parameter.surface();
    let mut out = Vec::with_capacity(34);
    put_u32(&mut out, 34);
    put_u8(&mut out, 4);
    put_u16(&mut out, 0); // no optional coordinate values
    put_u16(&mut out, 0); // template 4.0

    put_u8(&mut out, spec.parameter.category());
    put_u8(&mut out, spec.parameter.number());
    put_u8(&mut out, 2); // generating process: forecast
    put_u8(&mut out, 0); // background process
    put_u8(&mut out, 0); // generating process identifier
    put_u16(&mut out, 0); // hours of observational cutoff
    put_u8(&mut out, 0); // minutes of cutoff
    put_u8(&mut out, 1); // unit of time range: hour
    put_u32(&mut out, spec.forecast_hour);

    put_u8(&mut out, surface_type);
    put_u8(&mut out, surface_scale);
    put_u32(&mut out, surface_value);
    put_u8(&mut out, MISSING_U8); // no second surface
    put_u8(&mut out, MISSING_U8);
    put_u32(&mut out, MISSING_U32);
    out
}

fn section5(packed: &Packed) -> Vec<u8> {
    let mut out = Vec::with_capacity(21);
    put_u32(&mut out, 21);
    put_u8(&mut out, 5);
    put_u32(&mut out, packed.count as u32);
    put_u16(&mut out, 0); // template 5.0: simple packing
    out.extend_from_slice(&packed.reference.to_be_bytes()); // IEEE 32-bit
    put_i16_sm(&mut out, packed.binary_scale);
    put_i16_sm(&mut out, packed.decimal_scale);
    put_u8(&mut out, packed.bits);
    put_u8(&mut out, 0); // original values were floating point
    out
}

fn section6() -> Vec<u8> {
    let mut out = Vec::with_capacity(6);
    put_u32(&mut out, 6);
    put_u8(&mut out, 6);
    put_u8(&mut out, 255); // no bitmap: every point has a value
    out
}

fn section7(packed: &Packed) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + packed.data.len());
    put_u32(&mut out, (5 + packed.data.len()) as u32);
    put_u8(&mut out, 7);
    out.extend_from_slice(&packed.data);
    out
}

/// Builds one complete GRIB2 message.
pub fn message(spec: &MessageSpec, values: &[f32]) -> Result<Vec<u8>> {
    spec.reference_time.validate()?;
    if values.len() as u64 != spec.grid.point_count() {
        return Err(GribError::UnsupportedGrid(format!(
            "{} values for a {}x{} grid",
            values.len(),
            spec.grid.ni,
            spec.grid.nj
        )));
    }

    let packed = packing::pack(values, spec.bits)?;
    let body = [
        section1(spec),
        section3(spec.grid),
        section4(spec),
        section5(&packed),
        section6(),
        section7(&packed),
    ]
    .concat();

    let total = 16 + body.len() + 4;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"GRIB");
    put_u16(&mut out, 0); // reserved
    put_u8(&mut out, spec.parameter.discipline());
    put_u8(&mut out, 2); // edition 2
    out.extend_from_slice(&(total as u64).to_be_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(b"7777");
    Ok(out)
}

/// Appends one message to a writer.
///
/// Messages are written as they are produced rather than assembled into one
/// buffer: a 0.1 degree export is several gigabytes, which must never be held
/// in memory at once (spec.md 12.4).
pub fn write_message<W: Write>(out: &mut W, spec: &MessageSpec, values: &[f32]) -> Result<usize> {
    let bytes = message(spec, values)?;
    out.write_all(&bytes)?;
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> GridSpec {
        GridSpec {
            ni: 360,
            nj: 181,
            micro_degrees: 1_000_000,
        }
    }

    fn spec() -> MessageSpec {
        MessageSpec {
            parameter: Parameter::WindU,
            grid: grid(),
            reference_time: ReferenceTime {
                year: 2026,
                month: 9,
                day: 2,
                hour: 12,
                minute: 0,
                second: 0,
            },
            forecast_hour: 6,
            centre: 255,
            bits: 16,
        }
    }

    #[test]
    fn sections_are_the_documented_lengths() {
        assert_eq!(section1(&spec()).len(), 21);
        assert_eq!(section3(grid()).len(), 72);
        assert_eq!(section4(&spec()).len(), 34);
        assert_eq!(section6().len(), 6);

        let packed = packing::pack(&[1.0, 2.0], packing::BITS_PER_VALUE).expect("packs");
        assert_eq!(section5(&packed).len(), 21);
        assert_eq!(section7(&packed).len(), 5 + packed.data.len());
    }

    /// Sign-magnitude, not two's complement. A latitude of -90 must not become
    /// 0xFAC9... or a decoder places the grid near the north pole.
    #[test]
    fn negative_values_use_sign_magnitude() {
        let mut out = Vec::new();
        put_i32_sm(&mut out, -90_000_000);
        assert_eq!(out, (0x8000_0000u32 | 90_000_000).to_be_bytes());
        assert_eq!(
            from_i32_sm(u32::from_be_bytes([out[0], out[1], out[2], out[3]])),
            -90_000_000
        );

        let mut out = Vec::new();
        put_i16_sm(&mut out, -9);
        assert_eq!(out, (0x8000u16 | 9).to_be_bytes());
        assert_eq!(from_i16_sm(u16::from_be_bytes([out[0], out[1]])), -9);
    }

    #[test]
    fn sign_magnitude_round_trips() {
        for value in [0i32, 1, -1, 90_000_000, -90_000_000, i32::MAX / 2] {
            let mut out = Vec::new();
            put_i32_sm(&mut out, value);
            let raw = u32::from_be_bytes([out[0], out[1], out[2], out[3]]);
            assert_eq!(from_i32_sm(raw), value);
        }
    }

    /// The grid runs from the prime meridian, not from -180.
    #[test]
    fn grid_points_start_at_the_prime_meridian() {
        let points: Vec<(f64, f64)> = grid().points().take(3).collect();
        assert_eq!(points[0], (0.0, 90.0));
        assert_eq!(points[1], (1.0, 90.0));
        assert_eq!(points[2], (2.0, 90.0));
    }

    /// The row wraps through the antimeridian into negative longitudes.
    #[test]
    fn grid_points_wrap_at_the_antimeridian() {
        let row: Vec<(f64, f64)> = grid().points().take(360).collect();
        assert_eq!(row[179], (179.0, 90.0));
        assert_eq!(
            row[180],
            (-180.0, 90.0),
            "180 east is -180 in the app's range"
        );
        assert_eq!(row[359], (-1.0, 90.0));
    }

    #[test]
    fn the_grid_runs_north_to_south_and_reaches_both_poles() {
        let points: Vec<(f64, f64)> = grid().points().collect();
        assert_eq!(points.len(), 360 * 181);
        assert_eq!(points[0].1, 90.0, "first row is the north pole");
        assert_eq!(points[360].1, 89.0, "the next row is one degree south");
        assert_eq!(points[points.len() - 1], (-1.0, -90.0), "last point");
    }

    #[test]
    fn a_message_has_the_right_envelope() {
        let values = vec![5.0f32; grid().point_count() as usize];
        let bytes = message(&spec(), &values).expect("builds");

        assert_eq!(&bytes[0..4], b"GRIB");
        assert_eq!(bytes[6], 0, "wind is discipline 0");
        assert_eq!(bytes[7], 2, "edition 2");
        let declared = u64::from_be_bytes(bytes[8..16].try_into().expect("8 bytes"));
        assert_eq!(declared as usize, bytes.len(), "declared length must match");
        assert_eq!(&bytes[bytes.len() - 4..], b"7777");
    }

    #[test]
    fn currents_use_the_oceanographic_discipline() {
        let mut spec = spec();
        spec.parameter = Parameter::CurrentU;
        let values = vec![0.5f32; grid().point_count() as usize];
        let bytes = message(&spec, &values).expect("builds");
        assert_eq!(bytes[6], 10, "currents are discipline 10");
        assert_eq!(
            Parameter::CurrentU.surface().0,
            160,
            "depth below sea surface"
        );
    }

    #[test]
    fn a_mismatched_value_count_is_rejected() {
        assert!(message(&spec(), &[1.0, 2.0]).is_err());
    }

    #[test]
    fn an_invalid_reference_time_is_rejected() {
        let mut spec = spec();
        spec.reference_time.month = 13;
        let values = vec![0.0f32; grid().point_count() as usize];
        assert!(message(&spec, &values).is_err());
    }

    #[test]
    fn parameter_numbers_follow_the_tables() {
        assert_eq!(
            (Parameter::WindU.category(), Parameter::WindU.number()),
            (2, 2)
        );
        assert_eq!(
            (Parameter::WindV.category(), Parameter::WindV.number()),
            (2, 3)
        );
        assert_eq!(
            (Parameter::CurrentU.category(), Parameter::CurrentU.number()),
            (1, 2)
        );
        assert_eq!(
            (Parameter::CurrentV.category(), Parameter::CurrentV.number()),
            (1, 3)
        );
    }
}
