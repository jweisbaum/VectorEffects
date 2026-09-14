//! The GPU field evaluator.
//!
//! An approximation of `cpu.rs`, for preview only. The CPU path is
//! authoritative and is what every exported GRIB is produced by (spec.md 7.8),
//! so this is free to work in `f32` and to decline scenes it cannot handle.
//!
//! It declines exactly one thing: the clone stamp, which reads the composite
//! beneath itself and therefore needs recursion. A scene containing one is
//! reported unsupported and the caller falls back to the CPU — which is the
//! same fallback that covers a machine with no usable GPU at all.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use ve_core::project::FieldKind;
use ve_core::vector::Uv;

use crate::aeqd::Space;
use crate::error::{RenderError, Result};
use crate::evaluator::{FieldEvaluator, Sample, SamplePoint};
use crate::scene::{DirectionMode, EdgeMode, Modifier, Scene, SpeedMode};
use crate::sdf::Shape;

/// Words per packed object. Must match the WGSL `Object` struct.
const OBJECT_WORDS: usize = 40;
/// Words per packed raster header. Must match the WGSL `Raster` struct.
const RASTER_WORDS: usize = 12;
/// Threads per workgroup. Must match the `@workgroup_size` in the shader.
const WORKGROUP: u32 = 64;

/// Bytes of imported field a scene may bind at once.
///
/// The device is created with `Limits::downlevel_defaults()`, whose storage
/// binding limit is 128 MiB, and every raster in a scene shares one binding.
/// A 0.1° global grid is 52 MB, so two of them fit and a third does not; the
/// decision has to be made without a device in hand (`supports` is static),
/// so the limit is restated here rather than read back.
const MAX_RASTER_BYTES: usize = 128 << 20;

/// How many concatenated raster uploads to keep on the device.
///
/// A scene's rasters change only when the time slice does, and a slice is
/// megabytes, so re-uploading per tile would dominate the tile budget. A
/// few entries cover the playhead and its neighbours.
const RASTER_BUFFERS_KEPT: usize = 4;

/// One cached raster upload: the content hashes it holds, in order, and the
/// buffer.
type RasterUpload = (Vec<[u8; 32]>, Arc<wgpu::Buffer>);

/// Bytes one raster contributes to the shared data binding.
fn raster_bytes(scene: &Scene) -> usize {
    scene.rasters.iter().map(|r| r.grid.len() * 8).sum()
}

/// Whether the GPU kernel can render this scene.
///
/// Two structural exclusions, both for the same reason: a compute shader has no
/// recursion. The clone stamp reads the composite beneath it at another
/// position, and so does a warp — the one modifier that reads somewhere other
/// than the cell it is writing (spec.md 6.3). The three modifiers that
/// transform the vector in place need nothing the shader lacks and stay on the
/// GPU. The remaining exclusion is a limit — imported fields beyond what one
/// storage binding can hold — and a scene past it goes to the CPU like any
/// other unsupported one.
pub fn supports(scene: &Scene) -> bool {
    scene.objects.iter().all(|object| {
        object.clone_source.is_none()
            // A liquify re-reads the scene like a warp, and is declined with
            // it (spec.md 6.3, M17).
            && !matches!(object.modifier, Some(Modifier::Warp(_) | Modifier::Smear))
            // A patch replays a captured lattice in the object's *own* frame,
            // which is a second sampler and a second storage buffer beyond the
            // one the imported rasters use (spec.md 8.5, M14). Declined for
            // now rather than approximated: the CPU is the authority, and a
            // scene the GPU never sees cannot disagree with it. The fallback
            // is the clone stamp's, for the same reason and through the same
            // path.
            && object.capture.is_none()
            // The eraser's stamps are variable in number and shape and are
            // evaluated on the CPU alone for now (M29): a scene with one
            // takes the same fallback the patch takes, and cannot disagree.
            && object.erased.is_empty()
    }) && scene.rasters.iter().all(|raster| raster.erased.is_empty())
        && raster_bytes(scene) <= MAX_RASTER_BYTES
}

/// A scene packed for the shader.
struct Packed {
    objects: Vec<u8>,
    points: Vec<u8>,
    /// Raster headers, one per raster plus a placeholder when there are none.
    rasters: Vec<u8>,
    /// Content hashes of the rasters, in order: the key of their data upload.
    raster_key: Vec<[u8; 32]>,
}

/// The concatenated samples of every raster in a scene, `[u, v]` as `f32`.
///
/// Built only when no cached upload matches `Packed::raster_key`.
fn raster_data(scene: &Scene) -> Vec<u8> {
    let mut data = Vec::with_capacity(raster_bytes(scene).max(8));
    for raster in &scene.rasters {
        for sample in &raster.grid.uv {
            push_f32(&mut data, sample[0]);
            push_f32(&mut data, sample[1]);
        }
    }
    // A storage buffer may not be empty.
    if data.is_empty() {
        push_f32(&mut data, 0.0);
        push_f32(&mut data, 0.0);
    }
    data
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// The kind as the kernel reads it: 1 for wind, 0 for a current — the same
/// bit the tile carries.
fn kind_word(kind: FieldKind) -> u32 {
    match kind {
        FieldKind::Wind => 1,
        FieldKind::Current => 0,
    }
}

fn pack(scene: &Scene) -> Packed {
    let mut objects = Vec::with_capacity(scene.objects.len() * OBJECT_WORDS * 4);
    let mut points: Vec<u8> = Vec::new();
    let mut point_count = 0u32;

    let push_point = |points: &mut Vec<u8>, count: &mut u32, p: [f64; 2]| {
        push_f32(points, p[0] as f32);
        push_f32(points, p[1] as f32);
        *count += 1;
    };

    // A swept shape is uploaded as *segment pairs* rather than polylines. The
    // shader then needs no chain boundaries: it walks pairs and takes the
    // nearest, and a one-point chain becomes a degenerate pair that the segment
    // distance already handles.
    let push_chains = |points: &mut Vec<u8>, count: &mut u32, chains: &[Vec<[f64; 2]>]| {
        for chain in chains {
            match chain.as_slice() {
                [] => {}
                [only] => {
                    push_point(points, count, *only);
                    push_point(points, count, *only);
                }
                _ => {
                    for pair in chain.windows(2) {
                        push_point(points, count, pair[0]);
                        push_point(points, count, pair[1]);
                    }
                }
            }
        }
    };

    for object in &scene.objects {
        let shape_offset = point_count;
        let (kind, mut a, mut b) = match &object.shape {
            Shape::Capsule { chains, radius_m } => {
                push_chains(&mut points, &mut point_count, chains);
                (0u32, *radius_m as f32, 0.0)
            }
            // The same pairs as a capsule, in the same layout: only the metric
            // the shader measures them in changes.
            Shape::SweptSquare {
                chains,
                half_size_m,
            } => {
                push_chains(&mut points, &mut point_count, chains);
                (5, *half_size_m as f32, 0.0)
            }
            Shape::Disc { radius_m } => (1, *radius_m as f32, 0.0),
            Shape::Annulus {
                radius_m,
                half_width_m,
            } => (2, *radius_m as f32, *half_width_m as f32),
            Shape::Rect {
                half_width_m,
                half_height_m,
            } => (3, *half_width_m as f32, *half_height_m as f32),
            Shape::Contours { rings, .. } => {
                for ring in rings.iter().filter(|r| r.len() >= 3) {
                    for (a, b) in ring
                        .iter()
                        .zip(ring.iter().cycle().skip(1))
                        .take(ring.len())
                    {
                        push_point(&mut points, &mut point_count, *a);
                        push_point(&mut points, &mut point_count, *b);
                    }
                }
                (6, 0.0, 0.0)
            }
            Shape::Polygon { ring } => {
                for point in ring {
                    push_point(&mut points, &mut point_count, *point);
                }
                (4, 0.0, 0.0)
            }
        };
        let shape_count = point_count - shape_offset;

        // A contour's footprint, source skeleton and direction path have
        // separate offsets. A radial edit must not read the direction path
        // as segment pairs when both are present.
        if let Shape::Contours { source, .. } = &object.shape {
            match source.as_ref() {
                Shape::Capsule { chains, .. } | Shape::SweptSquare { chains, .. } => {
                    b = point_count as f32;
                    let offset = point_count;
                    push_chains(&mut points, &mut point_count, chains);
                    a = (point_count - offset) as f32;
                }
                _ => {}
            }
        }

        // The path is uploaded separately: a curve's shape is a corridor and
        // its direction follows the ordered path, so one region cannot serve
        // both.
        let path_offset = point_count;
        if matches!(object.direction, DirectionMode::AlongPath { .. }) {
            for point in &object.path {
                push_point(&mut points, &mut point_count, *point);
            }
        }
        let path_count = point_count - path_offset;

        let (speed_kind, speed_a, speed_b, speed_extent) = match object.speed {
            SpeedMode::Constant(speed) => (0u32, speed as f32, 0.0, 0.0),
            SpeedMode::Radial {
                centre,
                edge,
                extent,
            } => (1, centre as f32, edge as f32, extent as f32),
            SpeedMode::Axis { start, end } => (2, start as f32, end as f32, 0.0),
        };

        let (dir_kind, dir_a, dir_b) = match &object.direction {
            DirectionMode::Constant(bearing) => (0u32, bearing.degrees() as f32, 0.0),
            DirectionMode::Target { target, rhumb, .. } => (
                if *rhumb { 6 } else { 1 },
                target.lon as f32,
                target.lat as f32,
            ),
            DirectionMode::Axis { start, end } => (2, start.degrees() as f32, end.degrees() as f32),
            DirectionMode::AlongPath { offset } => (3, offset.degrees() as f32, 0.0),
            // Tangential is a signed quarter turn off the outward bearing.
            DirectionMode::Tangential { clockwise, angle } => (
                4,
                (if *clockwise {
                    90.0 - angle
                } else {
                    angle - 90.0
                }) as f32,
                0.0,
            ),
        };

        push_f32(&mut objects, object.frame.anchor.lon as f32);
        push_f32(&mut objects, object.frame.anchor.lat as f32);
        push_f32(&mut objects, object.frame.rotation_deg as f32);
        push_f32(&mut objects, object.frame.scale as f32);

        push_f32(&mut objects, object.cap_radius_m as f32);
        push_u32(&mut objects, kind);
        push_f32(&mut objects, a);
        push_f32(&mut objects, b);

        push_u32(&mut objects, shape_offset);
        push_u32(&mut objects, shape_count);
        push_u32(&mut objects, speed_kind);
        push_f32(&mut objects, speed_a);

        push_f32(&mut objects, speed_b);
        push_f32(&mut objects, speed_extent);
        push_u32(&mut objects, dir_kind);
        push_f32(&mut objects, dir_a);

        // What a modifier does to the field beneath it, in the two words where
        // `divergence` and `curl` used to sit (spec.md 7.5) — which is fitting,
        // since a modifier is where that shaping went. A warp never reaches the
        // GPU (`supports`), so it needs no encoding.
        let (mod_kind, mod_a) = match object.modifier {
            None => (0u32, 0.0f32),
            Some(Modifier::Gain(gain)) => (1, gain as f32),
            Some(Modifier::Radial(fraction)) => (2, fraction as f32),
            Some(Modifier::Turn(degrees)) => (3, degrees as f32),
            Some(Modifier::Warp(_) | Modifier::Smear) => (0, 0.0),
        };

        push_f32(&mut objects, dir_b);
        push_f32(&mut objects, object.feather as f32);
        push_u32(&mut objects, mod_kind);
        push_f32(&mut objects, mod_a);

        push_u32(
            &mut objects,
            match object.edge_mode {
                EdgeMode::Blend => 0,
                EdgeMode::Replace => 1,
            },
        );
        push_f32(&mut objects, object.gradient_axis.degrees() as f32);
        push_f32(&mut objects, object.gradient_extent as f32);
        push_f32(&mut objects, object.shape.feather_reference_m() as f32);

        push_u32(&mut objects, path_offset);
        push_u32(&mut objects, path_count);
        push_u32(
            &mut objects,
            match object.frame.space {
                Space::Geodesic => 0,
                Space::Projected => 1,
                Space::Mercator => 2,
                Space::Miller => 3,
            },
        );
        push_u32(&mut objects, u32::from(object.invert));

        // The object's own movement (spec.md 9.3). Four words, so the struct
        // stays a whole number of 16-byte rows.
        push_f32(&mut objects, object.motion.omega[0] as f32);
        push_f32(&mut objects, object.motion.omega[1] as f32);
        push_f32(&mut objects, object.motion.omega[2] as f32);
        push_f32(&mut objects, object.motion.scale_rate as f32);

        // Its layer, its kind and whether it removes rather than writes
        // (M31): what the kernel stacks the layers by, draws the cell as,
        // and takes the coverage down with. A fourth word keeps the row.
        push_u32(&mut objects, object.layer);
        push_u32(&mut objects, kind_word(object.kind));
        push_u32(&mut objects, u32::from(object.erases));
        let target_offset = match object.direction {
            DirectionMode::Target { offset, .. } => offset as f32,
            _ => 0.0,
        };
        push_f32(&mut objects, target_offset);
        let (low, high) = scene
            .speed_band(object.layer)
            .map_or((0.0, f32::INFINITY), |band| (band.min_mps, band.max_mps));
        push_f32(&mut objects, low);
        push_f32(&mut objects, high);
        push_u32(&mut objects, 0);
        push_u32(&mut objects, 0);

        debug_assert_eq!(objects.len() % (OBJECT_WORDS * 4), 0);
    }

    // A storage buffer may not be empty, and `arrayLength` derives from its
    // size, so a scene with no polyline still needs one placeholder point.
    if points.is_empty() {
        push_f32(&mut points, 0.0);
        push_f32(&mut points, 0.0);
    }

    // Raster headers. Offsets count `vec2<f32>` samples into the shared data
    // binding, in the order the rasters are concatenated.
    let mut rasters = Vec::with_capacity(scene.rasters.len().max(1) * RASTER_WORDS * 4);
    let mut raster_key = Vec::with_capacity(scene.rasters.len());
    let mut offset = 0u32;
    for raster in &scene.rasters {
        let grid = &raster.grid;
        push_u32(&mut rasters, grid.ni);
        push_u32(&mut rasters, grid.nj);
        push_u32(&mut rasters, offset);
        push_u32(&mut rasters, raster.z as u32);
        push_f32(&mut rasters, grid.lon0 as f32);
        push_f32(&mut rasters, grid.lat0 as f32);
        push_f32(&mut rasters, grid.dlon as f32);
        push_f32(&mut rasters, grid.dlat as f32);
        push_u32(&mut rasters, u32::from(grid.wraps));
        // The layer's speed band, as a pair the shader can compare against
        // without a branch on "is there one": no band is the widest possible
        // one (spec.md 4.8).
        let (low, high) = scene
            .speed_band(raster.layer)
            .map_or((0.0, f32::INFINITY), |band| (band.min_mps, band.max_mps));
        push_f32(&mut rasters, low);
        push_f32(&mut rasters, high);
        // Its layer and kind (M31), packed in one word: the layer in the low
        // sixteen bits, the kind above.
        push_u32(&mut rasters, raster.layer | (kind_word(raster.kind) << 16));
        offset += grid.len() as u32;
        raster_key.push(grid.hash);
        debug_assert_eq!(rasters.len() % (RASTER_WORDS * 4), 0);
    }
    // The placeholder sits above every object, so the kernel never reaches
    // it: `z` is compared against the object count, which is far smaller.
    if rasters.is_empty() {
        for word in [0u32, 0, 0, u32::MAX, 0, 0, 0, 0, 0, 0, 0, 0] {
            push_u32(&mut rasters, word);
        }
    }

    Packed {
        objects,
        points,
        rasters,
        raster_key,
    }
}

/// A wgpu-backed evaluator.
#[derive(Debug)]
pub struct GpuEvaluator {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// Recent raster uploads, most recently used first, keyed by the content
    /// hashes of the rasters they hold in order.
    raster_buffers: Mutex<Vec<RasterUpload>>,
    /// Adapter description, surfaced in About and in the log.
    pub adapter: String,
}

impl GpuEvaluator {
    /// Creates an evaluator, or reports why the GPU cannot be used.
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|err| RenderError::NoAdapter(err.to_string()))?;

        let info = adapter.get_info();
        let description = format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend);

        // GitHub's Intel Mac VM reports its paravirtual adapter as DiscreteGpu,
        // but it fails the deterministic field-fidelity sweep (case 32: a
        // 21.456 m/s vector where the CPU reports calm). The same sweep passes
        // on physical Metal hardware. Decline the adapter for the application
        // too, so a virtualized Mac gets the correct CPU preview.
        if info.name.starts_with("Apple Paravirtual") {
            return Err(RenderError::NoAdapter(
                "Apple virtual GPUs are not supported; using CPU rendering".to_owned(),
            ));
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ve-render"),
            required_features: wgpu::Features::empty(),
            // The downlevel limits, except that the kernel binds six storage
            // buffers — the four it always had plus the imported fields'
            // headers and samples — where downlevel allows four. Every
            // desktop backend this crate is built for offers far more (Metal
            // 31, and `Limits::default()` already assumes 8); an adapter that
            // does not fails device creation and the CPU path takes over,
            // which is the documented fallback for a machine without a
            // usable GPU (spec.md 7.8).
            required_limits: wgpu::Limits {
                max_storage_buffers_per_shader_stage: 6,
                ..wgpu::Limits::downlevel_defaults()
            },
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            ..Default::default()
        }))
        .map_err(|err| RenderError::NoAdapter(format!("device creation failed: {err}")))?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("evaluate"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shaders/evaluate.wgsl"))),
        });

        let storage = |read_only: bool| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let entry = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: storage(read_only),
            count: None,
        };

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("evaluate"),
            entries: &[
                entry(0, true),
                entry(1, true),
                entry(2, true),
                entry(3, false),
                entry(4, true),
                entry(5, true),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("evaluate"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("evaluate"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            layout,
            raster_buffers: Mutex::new(Vec::new()),
            adapter: description,
        })
    }

    /// The data upload for a scene's rasters, reused across tiles.
    fn raster_buffer(&self, scene: &Scene, key: &[[u8; 32]]) -> Result<Arc<wgpu::Buffer>> {
        use wgpu::util::DeviceExt;
        let mut kept = self
            .raster_buffers
            .lock()
            .map_err(|_| RenderError::Flatten("raster upload cache poisoned".to_owned()))?;
        if let Some(index) = kept.iter().position(|(k, _)| k.as_slice() == key) {
            let entry = kept.remove(index);
            let buffer = Arc::clone(&entry.1);
            kept.insert(0, entry);
            return Ok(buffer);
        }
        let buffer = Arc::new(
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("rasters"),
                    contents: &raster_data(scene),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
        );
        kept.insert(0, (key.to_vec(), Arc::clone(&buffer)));
        kept.truncate(RASTER_BUFFERS_KEPT);
        Ok(buffer)
    }
}

impl FieldEvaluator for GpuEvaluator {
    fn backend_name(&self) -> &'static str {
        "wgpu"
    }

    fn evaluate_samples(&self, scene: &Scene, points: &[SamplePoint]) -> Result<Vec<Sample>> {
        if points.is_empty() {
            return Ok(Vec::new());
        }
        if !supports(scene) {
            return Err(RenderError::Unsupported(
                "scene contains a clone stamp, which needs recursion, or more \
                 imported field than one binding holds"
                    .to_owned(),
            ));
        }
        // Nothing to composite: every sample is calm, and a zero-length storage
        // buffer is not permitted anyway.
        if scene.objects.is_empty() && scene.rasters.is_empty() {
            return Ok(vec![Sample::default(); points.len()]);
        }

        use wgpu::util::DeviceExt;
        let packed = pack(scene);

        let mut sample_bytes = Vec::with_capacity(points.len() * 8);
        for point in points {
            sample_bytes.extend_from_slice(&(point.lon as f32).to_le_bytes());
            sample_bytes.extend_from_slice(&(point.lat as f32).to_le_bytes());
        }
        // Four floats a sample: u, v, coverage, kind (M31).
        let output_size = (points.len() * 16) as u64;

        let make = |label, contents: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        // A scene of rasters alone still needs a non-empty object buffer. The
        // placeholder's cap radius is negative, so the cull rejects it at
        // every sample and it is never looked at further.
        let object_bytes = if packed.objects.is_empty() {
            let mut placeholder = Vec::with_capacity(OBJECT_WORDS * 4);
            for word in 0..OBJECT_WORDS {
                push_f32(&mut placeholder, if word == 4 { -1.0 } else { 0.0 });
            }
            placeholder
        } else {
            packed.objects
        };
        let objects = make("objects", &object_bytes);
        let point_buffer = make("points", &packed.points);
        let samples = make("samples", &sample_bytes);
        let raster_headers = make("raster headers", &packed.rasters);
        let raster_samples = self.raster_buffer(scene, &packed.raster_key)?;

        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output"),
            size: output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("evaluate"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: objects.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: point_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: samples.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: raster_headers.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: raster_samples.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("evaluate"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("evaluate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups((points.len() as u32).div_ceil(WORKGROUP), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, output_size);
        self.queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|err| RenderError::Flatten(format!("gpu poll failed: {err}")))?;
        receiver
            .recv()
            .map_err(|_| RenderError::Flatten("gpu readback was dropped".to_owned()))?
            .map_err(|err| RenderError::Flatten(format!("gpu readback failed: {err}")))?;

        let data = slice
            .get_mapped_range()
            .map_err(|err| RenderError::Flatten(format!("gpu buffer map failed: {err}")))?;
        let word = |chunk: &[u8], at: usize| {
            f32::from_le_bytes([chunk[at], chunk[at + 1], chunk[at + 2], chunk[at + 3]])
        };
        let out = data
            .chunks_exact(16)
            .map(|chunk| Sample {
                uv: Uv {
                    u: word(chunk, 0),
                    v: word(chunk, 4),
                },
                coverage: word(chunk, 8),
                kind: if word(chunk, 12) >= 0.5 {
                    FieldKind::Wind
                } else {
                    FieldKind::Current
                },
            })
            .collect();
        drop(data);
        staging.unmap();

        Ok(out)
    }
}
