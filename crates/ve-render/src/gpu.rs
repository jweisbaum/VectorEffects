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

use ve_core::vector::Uv;

use crate::aeqd::Space;
use crate::error::{RenderError, Result};
use crate::evaluator::{FieldEvaluator, SamplePoint};
use crate::scene::{DirectionMode, EdgeMode, Scene, SpeedMode};
use crate::sdf::Shape;

/// Words per packed object. Must match the WGSL `Object` struct.
const OBJECT_WORDS: usize = 28;
/// Threads per workgroup. Must match the `@workgroup_size` in the shader.
const WORKGROUP: u32 = 64;

/// Whether the GPU kernel can render this scene.
///
/// The clone stamp is the only exclusion, and it is a structural one rather
/// than a limit that could be raised: a compute shader has no recursion.
pub fn supports(scene: &Scene) -> bool {
    scene
        .objects
        .iter()
        .all(|object| object.clone_source.is_none())
}

/// A scene packed for the shader.
struct Packed {
    objects: Vec<u8>,
    points: Vec<u8>,
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
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
        let (kind, a, b) = match &object.shape {
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
            Shape::Polygon { ring } => {
                for point in ring {
                    push_point(&mut points, &mut point_count, *point);
                }
                (4, 0.0, 0.0)
            }
        };
        let shape_count = point_count - shape_offset;

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
            DirectionMode::Toward(target) => (1, target.lon as f32, target.lat as f32),
            DirectionMode::Away(target) => (5, target.lon as f32, target.lat as f32),
            DirectionMode::Axis { start, end } => (2, start.degrees() as f32, end.degrees() as f32),
            DirectionMode::AlongPath { offset } => (3, offset.degrees() as f32, 0.0),
            // Tangential is a signed quarter turn off the outward bearing.
            DirectionMode::Tangential { clockwise } => {
                (4, if *clockwise { 90.0 } else { -90.0 }, 0.0)
            }
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

        push_f32(&mut objects, dir_b);
        push_f32(&mut objects, object.feather as f32);
        // Where `divergence` and `curl` were: no tool has them (spec.md 7.5).
        // Left as padding rather than closing the gap, because the WGSL struct
        // has to stay a multiple of 16 bytes and 26 words is not one.
        push_f32(&mut objects, 0.0);
        push_f32(&mut objects, 0.0);

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
            },
        );
        push_f32(&mut objects, 0.0);

        debug_assert_eq!(objects.len() % (OBJECT_WORDS * 4), 0);
    }

    // A storage buffer may not be empty, and `arrayLength` derives from its
    // size, so a scene with no polyline still needs one placeholder point.
    if points.is_empty() {
        push_f32(&mut points, 0.0);
        push_f32(&mut points, 0.0);
    }

    Packed { objects, points }
}

/// A wgpu-backed evaluator.
#[derive(Debug)]
pub struct GpuEvaluator {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
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

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ve-render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
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
            adapter: description,
        })
    }
}

impl FieldEvaluator for GpuEvaluator {
    fn backend_name(&self) -> &'static str {
        "wgpu"
    }

    fn evaluate(&self, scene: &Scene, points: &[SamplePoint]) -> Result<Vec<Uv>> {
        if points.is_empty() {
            return Ok(Vec::new());
        }
        if !supports(scene) {
            return Err(RenderError::Unsupported(
                "scene contains a clone stamp, which needs recursion".to_owned(),
            ));
        }
        // Nothing to composite: every sample is calm, and a zero-length storage
        // buffer is not permitted anyway.
        if scene.objects.is_empty() {
            return Ok(vec![Uv::default(); points.len()]);
        }

        use wgpu::util::DeviceExt;
        let packed = pack(scene);

        let mut sample_bytes = Vec::with_capacity(points.len() * 8);
        for point in points {
            sample_bytes.extend_from_slice(&(point.lon as f32).to_le_bytes());
            sample_bytes.extend_from_slice(&(point.lat as f32).to_le_bytes());
        }
        let output_size = (points.len() * 8) as u64;

        let make = |label, contents: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let objects = make("objects", &packed.objects);
        let point_buffer = make("points", &packed.points);
        let samples = make("samples", &sample_bytes);

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
        let out = data
            .chunks_exact(8)
            .map(|chunk| Uv {
                u: f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                v: f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            })
            .collect();
        drop(data);
        staging.unmap();

        Ok(out)
    }
}
