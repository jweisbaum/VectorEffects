//! A captured field, and the `.vecap` container it travels in (spec.md 8.5).
//!
//! A capture is what `Cmd`-`C` over a region takes: the visible composite
//! inside that region, on the project's own lattice, at one step or over a run
//! of them. `Cmd`-`V` puts it back as a **patch** — an object like any other,
//! whose field is these samples instead of a formula.
//!
//! # Why this is in the project at all
//!
//! Invariants 1 and 2 forbid a *rendered* raster in a project, and rightly: a
//! render is a cache product, reproducible from the objects, and storing one
//! would mean a project that could disagree with itself. A **captured** raster
//! is the other thing entirely. It is user content — what the field looked
//! like at the moment they took it — and it stops being reproducible the
//! instant its sources change. Keeping it by reference, as a GRIB layer keeps
//! a path, would make every pasted patch in every project go blank the day the
//! stroke it came from was edited (D52).
//!
//! So it travels **with** the project, as its own compressed archive entry
//! keyed by content hash, and never as JSON: the document keeps the hash and
//! nothing else.
//!
//! # Zero and undefined are different things
//!
//! A cell no object and no raster wrote is **undefined**, and so is one a mask
//! removed — the mask exists to let what is beneath show through, and a
//! capture that turned that into a real zero would overwrite whatever the
//! patch is later pasted over (D58). Undefined is `NaN` in the container and
//! writes nothing when composited; a real zero is calm, and overwrites.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::document::Geometry;
use crate::error::{CoreError, Result};
use crate::project::FieldKind;

/// File magic. Six bytes so the header's `u16` lands on an even offset.
const MAGIC: &[u8; 6] = b"VECAP\0";

/// Container version. Bumped when the header grows a field that an older
/// reader would misread; a reader refuses a version it does not know rather
/// than guessing at the bytes after it.
/// Version 2 added each frame's displacement, for a capture that recorded a
/// moving region (spec.md 8.7). Version 1 is still read: its frames never
/// moved.
const VERSION: u16 = 2;

/// One time slice of a captured field.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureFrame {
    /// Hours after the capture's first frame.
    pub offset_hours: f64,
    /// Where this frame's region sat relative to the first frame's, in
    /// degrees of the map, eastward (spec.md 8.7).
    ///
    /// Zero for a still capture and for one taken **static**, where the whole
    /// point is that a region dragged to follow a moving system yields a macro
    /// of that system standing still.
    pub dx_deg: f64,
    /// And northward.
    pub dy_deg: f64,
    /// `[u, v]` in m/s at `j * ni + i`, `NaN` where the source was undefined.
    pub uv: Vec<[f32; 2]>,
}

impl CaptureFrame {
    /// A frame whose region did not move.
    pub fn still(offset_hours: f64, uv: Vec<[f32; 2]>) -> Self {
        Self {
            offset_hours,
            dx_deg: 0.0,
            dy_deg: 0.0,
            uv,
        }
    }
}

/// A field captured from a region of the map.
///
/// The lattice is in the **region's own space**, which is map space: a region
/// is drawn on the map, so what is captured from one is measured in degrees of
/// the map and not in ground metres (D28, D55). `x0_deg` and `y0_deg` are the
/// offset of node `(0, 0)` from the object's anchor, so the whole lattice
/// travels, turns and scales with the object's frame.
#[derive(Debug, Clone)]
pub struct Capture {
    /// Wind or current, from the project it was taken in.
    pub kind: FieldKind,
    /// Columns.
    pub ni: u32,
    /// Rows.
    pub nj: u32,
    /// Degrees of the map between nodes, both axes. The project's resolution.
    pub spacing_deg: f64,
    /// Offset of node `(0, 0)` from the anchor, in degrees, eastward.
    pub x0_deg: f64,
    /// And northward. Rows run **north to south**, as every lattice here does.
    pub y0_deg: f64,
    /// Hours between frames; zero for a still capture.
    pub seconds_per_frame: f64,
    /// The region's own shape, so a capture means something without an object
    /// around it — which is what a macro is (M16).
    pub shape: Geometry,
    /// One or more time slices, in order, the first at offset zero.
    pub frames: Vec<CaptureFrame>,
    /// BLAKE3 of the encoded bytes: the archive entry's name, and what makes
    /// two identical captures one entry.
    pub hash: String,
}

/// Two captures are the same capture when their contents hash alike.
///
/// Derived equality cannot serve: an undefined sample is `NaN`, and `NaN`
/// equals nothing including itself, so a capture with one hole would not equal
/// *itself*. The hash is over the encoded bytes, where a `NaN` is a bit
/// pattern like any other — the same reason `RasterGrid` compares by hash.
impl PartialEq for Capture {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

/// Whether a sample is the container's "nothing was here".
pub fn is_undefined(sample: [f32; 2]) -> bool {
    sample[0].is_nan() || sample[1].is_nan()
}

/// The sample an undefined cell carries.
pub const UNDEFINED: [f32; 2] = [f32::NAN, f32::NAN];

impl Capture {
    /// Nodes per frame.
    pub fn node_count(&self) -> usize {
        self.ni as usize * self.nj as usize
    }
}

/// Where a capture's lattice sits, in the object's own frame.
///
/// Grouped rather than passed loose: five numbers in a row, four of them
/// `f64` degrees, is a call nobody can read and two of them can be swapped in
/// without the compiler noticing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureLattice {
    /// Columns.
    pub ni: u32,
    /// Rows.
    pub nj: u32,
    /// Degrees of the map between nodes, both axes.
    pub spacing_deg: f64,
    /// Offset of node `(0, 0)` from the anchor, eastward.
    pub x0_deg: f64,
    /// And northward. Rows run north to south.
    pub y0_deg: f64,
}

impl Capture {
    /// Builds a capture and computes its hash, checking the frames fill it.
    pub fn new(
        kind: FieldKind,
        lattice: CaptureLattice,
        seconds_per_frame: f64,
        shape: Geometry,
        frames: Vec<CaptureFrame>,
    ) -> Result<Self> {
        let CaptureLattice {
            ni,
            nj,
            spacing_deg,
            x0_deg,
            y0_deg,
        } = lattice;
        if ni == 0 || nj == 0 {
            return Err(CoreError::Capture(
                "a capture needs at least one row and column".to_owned(),
            ));
        }
        if frames.is_empty() {
            return Err(CoreError::Capture(
                "a capture needs at least one frame".to_owned(),
            ));
        }
        let expected = ni as usize * nj as usize;
        for (at, frame) in frames.iter().enumerate() {
            if frame.uv.len() != expected {
                return Err(CoreError::Capture(format!(
                    "capture frame {at} holds {} samples for a {ni} x {nj} lattice",
                    frame.uv.len()
                )));
            }
        }
        let mut capture = Self {
            kind,
            ni,
            nj,
            spacing_deg,
            x0_deg,
            y0_deg,
            seconds_per_frame,
            shape,
            frames,
            hash: String::new(),
        };
        // The hash is of the encoded bytes, so two captures that encode alike
        // are one archive entry — and the entry's name proves its contents.
        let bytes = capture.encode()?;
        capture.hash = hex(blake3::hash(&bytes).as_bytes());
        Ok(capture)
    }

    /// The frame at an offset in hours, or the only one for a still capture.
    pub fn frame_at(&self, offset_hours: f64) -> Option<&CaptureFrame> {
        if self.frames.len() == 1 {
            return self.frames.first();
        }
        self.frames
            .iter()
            .min_by(|a, b| {
                let da = (a.offset_hours - offset_hours).abs();
                let db = (b.offset_hours - offset_hours).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .filter(|frame| (frame.offset_hours - offset_hours).abs() < 1e-6)
    }

    /// Encodes the container: a plain header, then the samples compressed.
    ///
    /// The header is uncompressed so a reader can reject a version, or read a
    /// capture's shape, without inflating megabytes of samples first.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let shape = serde_json::to_vec(&self.shape)
            .map_err(|e| CoreError::Capture(format!("its shape: {e}")))?;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.push(match self.kind {
            FieldKind::Wind => 0,
            FieldKind::Current => 1,
        });
        // Room for a flag byte the format does not need yet, so the first one
        // it does need is not a version bump.
        out.push(0);
        out.extend_from_slice(&self.ni.to_le_bytes());
        out.extend_from_slice(&self.nj.to_le_bytes());
        out.extend_from_slice(&self.spacing_deg.to_le_bytes());
        out.extend_from_slice(&self.x0_deg.to_le_bytes());
        out.extend_from_slice(&self.y0_deg.to_le_bytes());
        out.extend_from_slice(&self.seconds_per_frame.to_le_bytes());
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        out.extend_from_slice(&(shape.len() as u32).to_le_bytes());
        out.extend_from_slice(&shape);
        for frame in &self.frames {
            out.extend_from_slice(&frame.offset_hours.to_le_bytes());
            out.extend_from_slice(&frame.dx_deg.to_le_bytes());
            out.extend_from_slice(&frame.dy_deg.to_le_bytes());
        }

        let mut raw = Vec::with_capacity(self.frames.len() * self.node_count() * 8);
        for frame in &self.frames {
            for sample in &frame.uv {
                raw.extend_from_slice(&sample[0].to_le_bytes());
                raw.extend_from_slice(&sample[1].to_le_bytes());
            }
        }
        let packed = lz4_flex::compress_prepend_size(&raw);
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        Ok(out)
    }

    /// Decodes a container, checking its magic, version and lengths.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let bad = |why: &str| CoreError::Capture(why.to_owned());
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8]> {
            let end = at + n;
            let slice = bytes.get(at..end).ok_or_else(|| bad("ends early"))?;
            at = end;
            Ok(slice)
        };
        if take(6)? != MAGIC {
            return Err(bad("is not a .vecap container"));
        }
        let version = u16::from_le_bytes(take(2)?.try_into().map_err(|_| bad("version"))?);
        if version == 0 || version > VERSION {
            return Err(CoreError::Capture(format!(
                "written by a newer version ({version})"
            )));
        }
        let kind = match take(1)?[0] {
            0 => FieldKind::Wind,
            1 => FieldKind::Current,
            other => return Err(bad(&format!("field kind {other}"))),
        };
        let _flags = take(1)?[0];
        let u32_of = |slice: &[u8]| -> Result<u32> {
            Ok(u32::from_le_bytes(
                slice.try_into().map_err(|_| bad("a count"))?,
            ))
        };
        let f64_of = |slice: &[u8]| -> Result<f64> {
            Ok(f64::from_le_bytes(
                slice.try_into().map_err(|_| bad("a number"))?,
            ))
        };
        let ni = u32_of(take(4)?)?;
        let nj = u32_of(take(4)?)?;
        let spacing_deg = f64_of(take(8)?)?;
        let x0_deg = f64_of(take(8)?)?;
        let y0_deg = f64_of(take(8)?)?;
        let seconds_per_frame = f64_of(take(8)?)?;
        let frame_count = u32_of(take(4)?)? as usize;
        let shape_len = u32_of(take(4)?)? as usize;
        let shape: Geometry = serde_json::from_slice(take(shape_len)?)
            .map_err(|e| CoreError::Capture(format!("its shape: {e}")))?;
        let mut offsets = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            let offset = f64_of(take(8)?)?;
            // Version 1 knew no displacement; its frames never moved.
            let (dx, dy) = if version >= 2 {
                (f64_of(take(8)?)?, f64_of(take(8)?)?)
            } else {
                (0.0, 0.0)
            };
            offsets.push((offset, dx, dy));
        }
        let packed_len = u32_of(take(4)?)? as usize;
        let packed = take(packed_len)?;
        let raw = lz4_flex::decompress_size_prepended(packed)
            .map_err(|e| CoreError::Capture(format!("its samples: {e}")))?;

        let nodes = ni as usize * nj as usize;
        let expected = frame_count * nodes * 8;
        if raw.len() != expected {
            return Err(CoreError::Capture(format!(
                "a capture holds {} sample bytes where {ni} x {nj} x {frame_count} frames need {expected}",
                raw.len()
            )));
        }
        let mut frames = Vec::with_capacity(frame_count);
        let mut cursor = 0usize;
        for (offset_hours, dx_deg, dy_deg) in offsets {
            let mut uv = Vec::with_capacity(nodes);
            for _ in 0..nodes {
                let u = f32::from_le_bytes(
                    raw[cursor..cursor + 4]
                        .try_into()
                        .map_err(|_| bad("a sample"))?,
                );
                let v = f32::from_le_bytes(
                    raw[cursor + 4..cursor + 8]
                        .try_into()
                        .map_err(|_| bad("a sample"))?,
                );
                uv.push([u, v]);
                cursor += 8;
            }
            frames.push(CaptureFrame {
                offset_hours,
                dx_deg,
                dy_deg,
                uv,
            });
        }
        Self::new(
            kind,
            CaptureLattice {
                ni,
                nj,
                spacing_deg,
                x0_deg,
                y0_deg,
            },
            seconds_per_frame,
            shape,
            frames,
        )
    }

    /// Reads a container from an archive entry.
    pub fn read(mut source: impl Read) -> Result<Self> {
        let mut bytes = Vec::new();
        source
            .read_to_end(&mut bytes)
            .map_err(|e| CoreError::Capture(e.to_string()))?;
        Self::decode(&bytes)
    }

    /// Writes the container to an archive entry.
    pub fn write(&self, mut sink: impl Write) -> Result<()> {
        let bytes = self.encode()?;
        sink.write_all(&bytes)
            .map_err(|e| CoreError::Capture(e.to_string()))
    }

    /// Bilinear sample at a position in the capture's own lattice space,
    /// measured in degrees from the object's anchor.
    ///
    /// The same rule `RasterGrid::sample` follows and the WGSL kernel ports:
    /// an undefined corner is left out of the blend and the remaining weights
    /// renormalised, and all four undefined is no coverage at all. That is
    /// what makes a paste transparent exactly where the source was (D58).
    pub fn sample(&self, frame: &CaptureFrame, x_deg: f64, y_deg: f64) -> Option<[f32; 2]> {
        let (last_i, last_j) = (f64::from(self.ni - 1), f64::from(self.nj - 1));
        let fi = (x_deg - self.x0_deg) / self.spacing_deg;
        // Rows run north to south, so a larger `y` is a smaller row index.
        let fj = (self.y0_deg - y_deg) / self.spacing_deg;
        const SLACK: f64 = 1e-6;
        if !(-SLACK..=last_i + SLACK).contains(&fi) || !(-SLACK..=last_j + SLACK).contains(&fj) {
            return None;
        }
        let fi = fi.clamp(0.0, last_i);
        let fj = fj.clamp(0.0, last_j);
        let i0 = (fi.floor() as u32).min(self.ni - 1);
        let j0 = (fj.floor() as u32).min(self.nj - 1);
        let i1 = (i0 + 1).min(self.ni - 1);
        let j1 = (j0 + 1).min(self.nj - 1);
        let tx = (fi - f64::from(i0)) as f32;
        let ty = (fj - f64::from(j0)) as f32;
        let at = |i: u32, j: u32| frame.uv[j as usize * self.ni as usize + i as usize];
        let corners = [
            (at(i0, j0), (1.0 - tx) * (1.0 - ty)),
            (at(i1, j0), tx * (1.0 - ty)),
            (at(i0, j1), (1.0 - tx) * ty),
            (at(i1, j1), tx * ty),
        ];
        let (mut total, mut u, mut v) = (0.0f32, 0.0f32, 0.0f32);
        for (sample, weight) in corners {
            if !is_undefined(sample) {
                total += weight;
                u += sample[0] * weight;
                v += sample[1] * weight;
            }
        }
        (total > 1e-6).then(|| [u / total, v / total])
    }
}

/// How a macro's frames map onto a project's steps (spec.md 8.7, M16).
///
/// A macro is a thing the user *placed*, so it holds like a keyframe rather
/// than vanishing like a message: §4.8's rule against holding a measurement
/// forward is about a forecast, and this is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resample {
    /// The frame at or before the step's time.
    Hold,
    /// The two nearest frames blended, `u` and `v` linearly, undefined where
    /// either is.
    Interpolate,
}

/// Which frame, or pair of frames, a macro shows at an elapsed time.
///
/// `None` past the last frame, unless the caller loops. Separated from the
/// sampling so the arithmetic can be checked by hand rather than through a
/// field (spec.md 8.7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramePick {
    /// The frame to show, or the earlier of the pair.
    pub frame: usize,
    /// The later frame, when two are being blended.
    pub next: usize,
    /// How far between them, 0 at `frame` and 1 at `next`.
    pub blend: f32,
}

impl Capture {
    /// Which frame a macro shows `elapsed_hours` after it starts.
    ///
    /// `looping` wraps past the end by the capture's own duration, so a macro
    /// set to loop repeats rather than stopping.
    pub fn pick(&self, elapsed_hours: f64, resample: Resample, looping: bool) -> Option<FramePick> {
        let last = self.frames.last()?;
        let span = last.offset_hours;
        let mut t = elapsed_hours;
        if t < -1e-9 {
            return None;
        }
        if t > span + 1e-9 {
            if !looping || span <= 0.0 {
                return None;
            }
            // One frame's worth past the last frame is where the first comes
            // round again, so the loop's period is the span plus one step.
            let step = span / (self.frames.len().saturating_sub(1).max(1)) as f64;
            t = t.rem_euclid(span + step);
            if t > span {
                // Inside the wrap-around gap: blend the last frame back to the
                // first, or hold the last.
                return Some(match resample {
                    Resample::Hold => FramePick {
                        frame: self.frames.len() - 1,
                        next: self.frames.len() - 1,
                        blend: 0.0,
                    },
                    Resample::Interpolate => FramePick {
                        frame: self.frames.len() - 1,
                        next: 0,
                        blend: (((t - span) / step) as f32).clamp(0.0, 1.0),
                    },
                });
            }
        }
        // The last frame at or before `t`.
        let at = self
            .frames
            .iter()
            .rposition(|frame| frame.offset_hours <= t + 1e-9)
            .unwrap_or(0);
        Some(match resample {
            Resample::Hold => FramePick {
                frame: at,
                next: at,
                blend: 0.0,
            },
            Resample::Interpolate => {
                let next = (at + 1).min(self.frames.len() - 1);
                let (a, b) = (self.frames[at].offset_hours, self.frames[next].offset_hours);
                let blend = if (b - a).abs() < 1e-9 {
                    0.0
                } else {
                    (((t - a) / (b - a)) as f32).clamp(0.0, 1.0)
                };
                FramePick {
                    frame: at,
                    next,
                    blend,
                }
            }
        })
    }

    /// The displacement a pick implies, in degrees of the map.
    pub fn displacement(&self, pick: FramePick) -> (f64, f64) {
        let a = &self.frames[pick.frame];
        let b = &self.frames[pick.next];
        let t = f64::from(pick.blend);
        (
            a.dx_deg + (b.dx_deg - a.dx_deg) * t,
            a.dy_deg + (b.dy_deg - a.dy_deg) * t,
        )
    }

    /// A blended sample at a position, for a pick.
    ///
    /// Undefined where **either** frame is: a cell that one frame never
    /// covered is a cell the blend has no honest value for, and inventing one
    /// would paint half a field over whatever is beneath (D58).
    pub fn sample_pick(&self, pick: FramePick, x_deg: f64, y_deg: f64) -> Option<[f32; 2]> {
        let a = self.sample(self.frames.get(pick.frame)?, x_deg, y_deg)?;
        if pick.frame == pick.next || pick.blend <= 0.0 {
            return Some(a);
        }
        let b = self.sample(self.frames.get(pick.next)?, x_deg, y_deg)?;
        let t = pick.blend;
        Some([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::LocalPoint;

    fn sample_capture(frames: usize) -> Capture {
        let (ni, nj) = (4u32, 3u32);
        let frames = (0..frames)
            .map(|f| CaptureFrame {
                offset_hours: f as f64 * 3.0,
                dx_deg: f as f64 * 0.5,
                dy_deg: 0.0,
                uv: (0..ni * nj)
                    .map(|k| {
                        if k == 5 {
                            UNDEFINED
                        } else {
                            [k as f32 + f as f32, -(k as f32)]
                        }
                    })
                    .collect(),
            })
            .collect();
        Capture::new(
            FieldKind::Wind,
            CaptureLattice {
                ni,
                nj,
                spacing_deg: 0.25,
                x0_deg: -0.5,
                y0_deg: 0.25,
            },
            10_800.0,
            Geometry::Rect {
                half_width_m: 500.0,
                half_height_m: 250.0,
            },
            frames,
        )
        .expect("a capture")
    }

    #[test]
    fn a_capture_round_trips_through_its_container() {
        for count in [1usize, 4] {
            let capture = sample_capture(count);
            let bytes = capture.encode().expect("encode");
            let back = Capture::decode(&bytes).expect("decode");
            assert_eq!(back, capture);
            // `==` is hash equality, so the samples are checked bit for bit
            // here rather than trusted to it — including the `NaN`, which is
            // the one value that would slip through a value comparison.
            assert_eq!(back.frames.len(), capture.frames.len());
            for (a, b) in back.frames.iter().zip(&capture.frames) {
                assert_eq!(a.offset_hours, b.offset_hours);
                assert_eq!(a.uv.len(), b.uv.len());
                for (x, y) in a.uv.iter().zip(&b.uv) {
                    assert_eq!(x[0].to_bits(), y[0].to_bits());
                    assert_eq!(x[1].to_bits(), y[1].to_bits());
                }
            }
            // And the encoding is stable, which is what lets the hash name the
            // archive entry.
            assert_eq!(back.encode().expect("re-encode"), bytes);
            assert_eq!(back.hash, capture.hash);
        }
    }

    /// Undefined must survive the container as undefined, not as calm: a cell
    /// a mask removed writes nothing when the patch is composited, and a real
    /// zero overwrites (D58).
    #[test]
    fn undefined_and_zero_stay_distinct() {
        let mut capture = sample_capture(1);
        capture.frames[0].uv[0] = [0.0, 0.0];
        capture.frames[0].uv[1] = UNDEFINED;
        let capture = Capture::new(
            capture.kind,
            CaptureLattice {
                ni: capture.ni,
                nj: capture.nj,
                spacing_deg: capture.spacing_deg,
                x0_deg: capture.x0_deg,
                y0_deg: capture.y0_deg,
            },
            capture.seconds_per_frame,
            capture.shape.clone(),
            capture.frames.clone(),
        )
        .expect("a capture");
        let back = Capture::decode(&capture.encode().expect("encode")).expect("decode");
        assert_eq!(back.frames[0].uv[0], [0.0, 0.0]);
        assert!(is_undefined(back.frames[0].uv[1]));
        assert!(!is_undefined(back.frames[0].uv[0]));
    }

    #[test]
    fn a_container_refuses_what_it_did_not_write() {
        assert!(Capture::decode(b"not a capture at all").is_err());
        let mut bytes = sample_capture(1).encode().expect("encode");
        bytes[6] = 99; // a version from the future
        assert!(Capture::decode(&bytes).is_err());
        let truncated = &sample_capture(1).encode().expect("encode")[..20];
        assert!(Capture::decode(truncated).is_err());
    }

    /// Two captures of the same field are one archive entry.
    #[test]
    fn identical_captures_hash_alike() {
        assert_eq!(sample_capture(2).hash, sample_capture(2).hash);
        assert_ne!(sample_capture(2).hash, sample_capture(3).hash);
    }

    #[test]
    fn a_polygon_shape_survives_the_container() {
        let capture = Capture::new(
            FieldKind::Current,
            CaptureLattice {
                ni: 2,
                nj: 2,
                spacing_deg: 1.0,
                x0_deg: 0.0,
                y0_deg: 0.0,
            },
            0.0,
            Geometry::Polygon {
                points: vec![
                    LocalPoint::new(0.0, 0.0),
                    LocalPoint::new(1000.0, 0.0),
                    LocalPoint::new(0.0, 1000.0),
                ],
            },
            vec![CaptureFrame::still(0.0, vec![[1.0, 2.0]; 4])],
        )
        .expect("a capture");
        let back = Capture::decode(&capture.encode().expect("encode")).expect("decode");
        assert_eq!(back.shape, capture.shape);
        assert_eq!(back.kind, FieldKind::Current);
    }

    /// The sampler's rule is `RasterGrid::sample`'s: an undefined corner is
    /// left out of the blend, and all four undefined is no coverage.
    #[test]
    fn sampling_leaves_undefined_corners_out_of_the_blend() {
        let capture = Capture::new(
            FieldKind::Wind,
            CaptureLattice {
                ni: 2,
                nj: 2,
                spacing_deg: 1.0,
                x0_deg: 0.0,
                y0_deg: 0.0,
            },
            0.0,
            Geometry::Rect {
                half_width_m: 1.0,
                half_height_m: 1.0,
            },
            vec![CaptureFrame::still(
                0.0,
                vec![[10.0, 0.0], [10.0, 0.0], UNDEFINED, UNDEFINED],
            )],
        )
        .expect("a capture");
        let frame = &capture.frames[0];
        // On the defined row, exactly its value.
        assert_eq!(capture.sample(frame, 0.5, 0.0), Some([10.0, 0.0]));
        // Halfway to the undefined row: the defined corners carry it alone,
        // rather than being dragged toward a zero that was never there.
        assert_eq!(capture.sample(frame, 0.5, -0.5), Some([10.0, 0.0]));
        // On the undefined row there is nothing at all.
        assert_eq!(capture.sample(frame, 0.5, -1.0), None);
        // And outside the lattice there is nothing either.
        assert_eq!(capture.sample(frame, 5.0, 0.0), None);
    }
}

#[cfg(test)]
mod resample_tests {
    use super::*;
    use crate::document::Geometry;

    /// A 6-hourly capture of three frames: 0, 6 and 12 hours.
    fn capture() -> Capture {
        Capture::new(
            FieldKind::Wind,
            CaptureLattice {
                ni: 1,
                nj: 1,
                spacing_deg: 1.0,
                x0_deg: 0.0,
                y0_deg: 0.0,
            },
            21_600.0,
            Geometry::Rect {
                half_width_m: 1.0,
                half_height_m: 1.0,
            },
            (0..3)
                .map(|f| CaptureFrame {
                    offset_hours: f as f64 * 6.0,
                    dx_deg: f as f64 * 2.0,
                    dy_deg: 0.0,
                    uv: vec![[f as f32 * 10.0, 0.0]],
                })
                .collect(),
        )
        .expect("a capture")
    }

    /// A 6-hourly capture in a 3-hourly project: hold shows the frame at or
    /// before the step, interpolate blends the two around it.
    #[test]
    fn a_coarse_capture_holds_or_interpolates() {
        let capture = capture();
        // Step 1 of a 3-hourly project is 3 hours in: half way between the
        // 0 h and 6 h frames.
        let held = capture.pick(3.0, Resample::Hold, false).expect("a frame");
        assert_eq!((held.frame, held.next), (0, 0));
        assert_eq!(
            capture.sample_pick(held, 0.0, 0.0),
            Some([0.0, 0.0]),
            "hold shows the 0 h frame unchanged"
        );

        let blended = capture
            .pick(3.0, Resample::Interpolate, false)
            .expect("a frame");
        assert_eq!((blended.frame, blended.next), (0, 1));
        assert!((blended.blend - 0.5).abs() < 1e-6);
        // Hand-computed: the 0 h frame is 0 and the 6 h frame is 10, so half
        // way is 5, and the displacement half way between 0 and 2 is 1.
        assert_eq!(capture.sample_pick(blended, 0.0, 0.0), Some([5.0, 0.0]));
        let (dx, dy) = capture.displacement(blended);
        assert!((dx - 1.0).abs() < 1e-9 && dy.abs() < 1e-9);
    }

    /// An hourly project reading a 6-hourly capture lands exactly on a frame
    /// every sixth step, whichever mode it is in.
    #[test]
    fn every_sixth_hour_lands_on_a_frame() {
        let capture = capture();
        for (hours, frame) in [(0.0, 0usize), (6.0, 1), (12.0, 2)] {
            for mode in [Resample::Hold, Resample::Interpolate] {
                let pick = capture.pick(hours, mode, false).expect("a frame");
                assert_eq!(pick.frame, frame, "{hours} h in {mode:?}");
                assert!(pick.blend.abs() < 1e-6 || pick.frame == pick.next);
            }
        }
    }

    /// Past the last frame there is nothing, unless the macro loops.
    #[test]
    fn a_macro_stops_at_its_end_unless_it_loops() {
        let capture = capture();
        assert!(capture.pick(18.0, Resample::Hold, false).is_none());
        let looped = capture.pick(18.0, Resample::Hold, true).expect("wrapped");
        // 18 h into a capture whose loop period is 12 + 6 = 18 h is back at
        // the start.
        assert_eq!(looped.frame, 0);
        assert!(capture.pick(-1.0, Resample::Hold, true).is_none());
    }

    /// A blend is undefined where either frame is: half a field painted over
    /// what is beneath would be worse than none (D58).
    #[test]
    fn a_blend_is_undefined_where_either_frame_is() {
        let mut frames = capture().frames;
        frames[1].uv[0] = UNDEFINED;
        let capture = Capture::new(
            FieldKind::Wind,
            CaptureLattice {
                ni: 1,
                nj: 1,
                spacing_deg: 1.0,
                x0_deg: 0.0,
                y0_deg: 0.0,
            },
            21_600.0,
            Geometry::Rect {
                half_width_m: 1.0,
                half_height_m: 1.0,
            },
            frames,
        )
        .expect("a capture");
        let pick = capture
            .pick(3.0, Resample::Interpolate, false)
            .expect("a frame");
        assert_eq!(capture.sample_pick(pick, 0.0, 0.0), None);
        // Holding shows the frame that *is* defined.
        let held = capture.pick(3.0, Resample::Hold, false).expect("a frame");
        assert_eq!(capture.sample_pick(held, 0.0, 0.0), Some([0.0, 0.0]));
    }
}
