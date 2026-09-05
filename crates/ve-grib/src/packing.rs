//! Simple packing, GRIB2 data representation template 5.0.
//!
//! Values are stored as scaled integers around a reference:
//!
//! ```text
//! Y * 10^D = R + X * 2^E
//! ```
//!
//! where `Y` is the original value, `X` the packed integer, `R` the reference
//! (an IEEE 32-bit float), `E` the binary scale factor and `D` the decimal
//! scale factor (spec.md 12.3).
//!
//! Sixteen bits over a +/-60 m/s wind field gives a step of 0.00195 m/s: the
//! span is 120/65535 = 0.00183, and the binary scale factor is a power of two
//! so it rounds up to 2^-9. That is about 0.004 knots — far finer than anything
//! the tools can express, so at the default width the packing is never what
//! limits the output. The width is a choice (spec.md 12.3, M19): 8 bits halves
//! the file at steps of about half a knot, 24 is for someone who asked.

use crate::error::{GribError, Result};

/// Bits per packed value, by default.
pub const BITS_PER_VALUE: u8 = 16;

/// The widths the export offers (spec.md 12.3, M19).
///
/// Whole octets and the two halves between them: 8 is the smallest that is
/// not visibly steppy on a wind field, 24 is finer than any reader will
/// distinguish from 32, and 12 and 16 are the two most decoders were built
/// around. Template 5.0 allows any width to 32; offering the ones with a
/// reason is what keeps the choice a choice.
pub const BIT_WIDTHS: [u8; 4] = [8, 12, 16, 24];

/// The step one level is worth at a width, over a nominal ±60 m/s field.
///
/// What the export dialog shows beside the width: the resolution the packing
/// *implies*, before the field's own range is known. The binary scale factor
/// is a power of two, so the step rounds up to one, as [`pack`] rounds it.
pub fn nominal_step_mps(bits: u8) -> f64 {
    let levels = ((1u64 << bits.clamp(1, 32)) - 1) as f64;
    2f64.powf((120.0 / levels).log2().ceil())
}

/// Decimal scale factor. Zero: the binary factor alone carries the range.
pub const DECIMAL_SCALE: i16 = 0;

/// Smallest binary scale factor allowed.
///
/// A nearly-constant field would otherwise drive the exponent far negative and
/// scale differences of a few ULPs up into the packed range, which is noise
/// rather than signal.
const MIN_BINARY_SCALE: i16 = -30;

/// A packed field, ready to become sections 5 and 7.
#[derive(Debug, Clone, PartialEq)]
pub struct Packed {
    /// Reference value `R`, the field minimum.
    pub reference: f32,
    /// Binary scale factor `E`.
    pub binary_scale: i16,
    /// Decimal scale factor `D`.
    pub decimal_scale: i16,
    /// Bits per value. Zero for a constant field.
    pub bits: u8,
    /// The packed bit stream, MSB first.
    pub data: Vec<u8>,
    /// How many values are packed.
    pub count: usize,
}

/// Packs a field with simple packing, at `bits` per value.
///
/// Any width from 1 to 32 packs — the bit stream is generic — but the export
/// offers [`BIT_WIDTHS`], and a width outside 1..=32 is refused rather than
/// silently clamped: a caller asking for 0 or 64 has a bug, not a preference.
pub fn pack(values: &[f32], bits: u8) -> Result<Packed> {
    if !(1..=32).contains(&bits) {
        return Err(GribError::UnsupportedGrid(format!(
            "{bits} bits per value; simple packing takes 1 to 32"
        )));
    }
    if let Some(index) = values.iter().position(|v| !v.is_finite()) {
        return Err(GribError::NonFiniteValue(index));
    }

    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &value in values {
        min = min.min(value);
        max = max.max(value);
    }
    if values.is_empty() {
        min = 0.0;
        max = 0.0;
    }

    // A constant field needs no data section at all. This is legal, compact,
    // and the common case for an untouched project.
    if min == max {
        return Ok(Packed {
            reference: min,
            binary_scale: 0,
            decimal_scale: DECIMAL_SCALE,
            bits: 0,
            data: Vec::new(),
            count: values.len(),
        });
    }

    let reference = min;
    let span = f64::from(max) - f64::from(reference);
    let levels = ((1u64 << bits) - 1) as f64;

    // Choose E so the whole span fits in the available levels.
    let scale = (span / levels).log2().ceil();
    let binary_scale = (scale as i16).max(MIN_BINARY_SCALE);
    let factor = 2f64.powi(i32::from(binary_scale));

    let mut writer = BitWriter::new(values.len(), bits);
    for &value in values {
        let scaled = ((f64::from(value) - f64::from(reference)) / factor).round();
        // Clamping guards the top of the range against rounding pushing the
        // maximum one level past what the bit width can hold.
        let packed = scaled.clamp(0.0, levels) as u64;
        writer.push(packed);
    }

    Ok(Packed {
        reference,
        binary_scale,
        decimal_scale: DECIMAL_SCALE,
        bits,
        data: writer.finish(),
        count: values.len(),
    })
}

/// Reverses [`pack`], for tests and for the in-repo reader.
pub fn unpack(packed: &Packed) -> Vec<f32> {
    let reference = f64::from(packed.reference);
    let binary = 2f64.powi(i32::from(packed.binary_scale));
    let decimal = 10f64.powi(i32::from(packed.decimal_scale));

    if packed.bits == 0 {
        return vec![(reference / decimal) as f32; packed.count];
    }

    let mut reader = BitReader::new(&packed.data, packed.bits);
    (0..packed.count)
        .map(|_| {
            let raw = reader.next().unwrap_or(0);
            (((raw as f64) * binary + reference) / decimal) as f32
        })
        .collect()
}

/// Writes fixed-width values into a bit stream, most significant bit first.
struct BitWriter {
    out: Vec<u8>,
    accumulator: u64,
    filled: u32,
    width: u32,
}

impl BitWriter {
    fn new(count: usize, bits: u8) -> Self {
        let capacity = (count * usize::from(bits)).div_ceil(8);
        Self {
            out: Vec::with_capacity(capacity),
            accumulator: 0,
            filled: 0,
            width: u32::from(bits),
        }
    }

    fn push(&mut self, value: u64) {
        self.accumulator = (self.accumulator << self.width) | value;
        self.filled += self.width;
        while self.filled >= 8 {
            self.filled -= 8;
            self.out
                .push(((self.accumulator >> self.filled) & 0xff) as u8);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.filled > 0 {
            // Pad the final octet with zeroes, as the format requires.
            let shift = 8 - self.filled;
            self.out.push(((self.accumulator << shift) & 0xff) as u8);
        }
        self.out
    }
}

/// Reads fixed-width values back out of a bit stream.
struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
    width: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], bits: u8) -> Self {
        Self {
            data,
            bit: 0,
            width: usize::from(bits),
        }
    }

    fn next(&mut self) -> Option<u64> {
        if self.width == 0 || self.bit + self.width > self.data.len() * 8 {
            return None;
        }
        let mut value = 0u64;
        for _ in 0..self.width {
            let byte = self.data.get(self.bit / 8)?;
            let shift = 7 - (self.bit % 8);
            value = (value << 1) | u64::from((byte >> shift) & 1);
            self.bit += 1;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tolerance the packing itself guarantees over a wind-sized range,
    /// at a width: the step is the span over the levels, rounded up to a
    /// power of two, and a rounded value is within half a step.
    fn tolerance_at(values: &[f32], bits: u8) -> f32 {
        let min = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let levels = ((1u64 << bits) - 1) as f32;
        ((max - min) / levels).max(1e-6) * 1.5
    }

    fn tolerance(values: &[f32]) -> f32 {
        tolerance_at(values, BITS_PER_VALUE)
    }

    /// Spec 12.3, M19: every offered width round-trips within its own step,
    /// keeps both endpoints exactly, and says in the header which width it
    /// used. The 8-bit step over a ±60 m/s field is about half a knot, which
    /// is the number the dialog shows.
    #[test]
    fn every_offered_width_round_trips_within_its_own_step() {
        let values: Vec<f32> = (0..1000)
            .map(|i| -60.0 + 120.0 * (i as f32) / 999.0)
            .collect();
        for bits in BIT_WIDTHS {
            let packed = pack(&values, bits).expect("packs");
            assert_eq!(packed.bits, bits);
            let back = unpack(&packed);
            let tol = tolerance_at(&values, bits);
            for (i, (a, b)) in values.iter().zip(&back).enumerate() {
                assert!(
                    (a - b).abs() <= tol,
                    "{bits} bits, index {i}: {a} != {b} (tol {tol})"
                );
            }
            assert_eq!(
                back[0], values[0],
                "{bits} bits: the minimum is the reference"
            );
            assert!(
                (back[999] - values[999]).abs() <= tol,
                "{bits} bits: the maximum survives the top of the range"
            );
            // The bit stream is exactly as long as the width says.
            assert_eq!(packed.data.len(), (1000 * usize::from(bits)).div_ceil(8));
        }
        // And the nominal step is what the field actually got: 2^E.
        let eight = pack(&values, 8).expect("packs");
        assert_eq!(
            nominal_step_mps(8),
            2f64.powi(i32::from(eight.binary_scale))
        );
        assert!(
            (nominal_step_mps(8) - 0.5).abs() < 1e-9,
            "half a metre per second at 8 bits"
        );
        assert!((nominal_step_mps(16) - 2f64.powi(-9)).abs() < 1e-12);
    }

    #[test]
    fn a_width_outside_the_stream_is_refused() {
        assert!(pack(&[1.0, 2.0], 0).is_err());
        assert!(pack(&[1.0, 2.0], 33).is_err());
        assert!(pack(&[1.0, 2.0], 32).is_ok());
    }

    fn round_trips(values: &[f32]) {
        let packed = pack(values, BITS_PER_VALUE).expect("packs");
        let back = unpack(&packed);
        assert_eq!(back.len(), values.len());
        let tol = tolerance(values);
        for (i, (a, b)) in values.iter().zip(&back).enumerate() {
            assert!((a - b).abs() <= tol, "index {i}: {a} != {b} (tol {tol})");
        }
    }

    #[test]
    fn a_wind_field_round_trips() {
        let values: Vec<f32> = (0..1000).map(|i| (i as f32) * 0.06 - 30.0).collect();
        round_trips(&values);
    }

    #[test]
    fn negative_and_positive_components_round_trip() {
        round_trips(&[-42.5, -1.0, 0.0, 0.25, 17.75, 63.0]);
    }

    /// A brand-new project is calm everywhere; that must not need a data
    /// section at all.
    #[test]
    fn a_constant_field_packs_to_nothing() {
        let packed = pack(&[7.5; 500], BITS_PER_VALUE).expect("packs");
        assert_eq!(packed.bits, 0);
        assert!(packed.data.is_empty());
        assert_eq!(packed.reference, 7.5);
        assert_eq!(unpack(&packed), vec![7.5; 500]);
    }

    #[test]
    fn an_all_zero_field_packs_to_nothing() {
        let packed = pack(&[0.0; 64], BITS_PER_VALUE).expect("packs");
        assert_eq!(packed.bits, 0);
        assert_eq!(unpack(&packed), vec![0.0; 64]);
    }

    #[test]
    fn the_reference_is_the_minimum() {
        let packed = pack(&[3.0, -11.0, 8.0], BITS_PER_VALUE).expect("packs");
        assert_eq!(packed.reference, -11.0);
    }

    /// The extremes must survive exactly, or the field's range shifts.
    #[test]
    fn the_endpoints_are_preserved() {
        let values = [-50.0f32, 0.0, 50.0];
        let back = unpack(&pack(&values, BITS_PER_VALUE).expect("packs"));
        let tol = tolerance(&values);
        assert!((back[0] - -50.0).abs() <= tol, "{}", back[0]);
        assert!((back[2] - 50.0).abs() <= tol, "{}", back[2]);
    }

    #[test]
    fn a_tiny_range_still_round_trips() {
        round_trips(&[10.000_01, 10.000_02, 10.000_03]);
    }

    #[test]
    fn a_wide_range_still_round_trips() {
        round_trips(&[-1000.0, 0.0, 1000.0, 999.5]);
    }

    #[test]
    fn non_finite_values_are_rejected_by_index() {
        assert!(matches!(
            pack(&[1.0, f32::NAN, 3.0], BITS_PER_VALUE),
            Err(GribError::NonFiniteValue(1))
        ));
        assert!(matches!(
            pack(&[f32::INFINITY], BITS_PER_VALUE),
            Err(GribError::NonFiniteValue(0))
        ));
    }

    #[test]
    fn an_empty_field_is_handled() {
        let packed = pack(&[], BITS_PER_VALUE).expect("packs");
        assert_eq!(packed.count, 0);
        assert!(unpack(&packed).is_empty());
    }

    /// Sixteen bits over a realistic wind range must be far finer than anything
    /// the tools can express, or the packing would be the limiting factor.
    ///
    /// The exact figure is 2^-9 = 0.00195 m/s: the span is 120/65535 = 0.00183
    /// and the scale factor is a power of two, so it rounds up. About 0.004
    /// knots, against a UI that shows one decimal place.
    #[test]
    fn resolution_is_far_finer_than_the_display() {
        let values: Vec<f32> = (0..=120).map(|i| (i as f32) - 60.0).collect();
        let packed = pack(&values, BITS_PER_VALUE).expect("packs");
        let step = 2f64.powi(i32::from(packed.binary_scale));
        assert!(step < 0.01, "packing step is {step} m/s");
        assert!(
            (step - 0.001_953_125).abs() < 1e-9,
            "expected 2^-9, got {step}"
        );
    }

    #[test]
    fn the_bit_stream_is_the_expected_length() {
        let packed = pack(&[0.0, 1.0, 2.0, 3.0], BITS_PER_VALUE).expect("packs");
        assert_eq!(packed.data.len(), 4 * 2, "four 16-bit values");
    }
}
