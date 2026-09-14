#![allow(clippy::expect_used, clippy::unwrap_used, reason = "test fixtures")]
//! Empty paint layers must leave the canvas and imported fields beneath them
//! visible, including on tiles where every real object has been culled.

use std::sync::Arc;

use ve_core::document::{Geometry, Layer, LocalPoint, Object, SpeedRange};
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::raster::{MISSING, RasterFrame, RasterGrid, RasterSequence};
use ve_core::schema::{PropId, ToolKind};
use ve_core::vector::Uv;
use ve_core::{LonLat, PropValue};
use ve_render::cpu::CpuEvaluator;
use ve_render::cull::{digests_of, tile_scene};
use ve_render::evaluator::{FieldEvaluator, Sample};
use ve_render::gpu::GpuEvaluator;
use ve_render::scene::flatten;
use ve_render::tile::TileId;

fn check_empty_layers(evaluator: &dyn FieldEvaluator) {
    let points = [
        LonLat::new(0.5, 0.5).unwrap(),   // moving field
        LonLat::new(2.5, 0.5).unwrap(),   // explicitly calm
        LonLat::new(4.5, 0.5).unwrap(),   // missing data
        LonLat::new(10.0, 10.0).unwrap(), // beyond the regional grid
    ];
    let tile = TileId::new(3, 8, 3).unwrap();

    for kind in [FieldKind::Wind, FieldKind::Current] {
        let uv = match kind {
            FieldKind::Wind => Uv { u: 3.0, v: 4.0 },
            FieldKind::Current => Uv { u: 0.12, v: 0.16 },
        };
        let row = [
            [uv.u, uv.v],
            [uv.u, uv.v],
            [0.0; 2],
            [0.0; 2],
            [MISSING; 2],
            [MISSING; 2],
        ];
        let grid = Arc::new(RasterGrid::new(6, 2, 0.0, 1.0, 1.0, 1.0, row.repeat(2)).unwrap());
        let sequence = Arc::new(
            RasterSequence::new(
                kind,
                vec![RasterFrame {
                    offset_hours: 0.0,
                    valid_unix_s: 0,
                    grid,
                }],
            )
            .unwrap(),
        );
        for history in [false, true] {
            let imported = if history {
                Layer::from_history(
                    "History",
                    "history.grib2".into(),
                    Arc::clone(&sequence),
                    "test",
                    0,
                    0,
                )
            } else {
                Layer::from_grib("GRIB", "field.grib2".into(), Arc::clone(&sequence), true)
            };
            // Changing order or visibility of an empty layer must not change
            // the imported field, even if that changes its flattened index.
            for layout in 0..6 {
                let mut empty = Layer::new("Empty paint");
                // An empty layer's own filter must not filter another layer.
                empty.speed_range = Some(SpeedRange {
                    min_mps: 0.0,
                    max_mps: 0.0,
                });
                empty.visible = layout < 3;
                let mut project = Project::new(
                    "Empty layers",
                    ProjectSettings::new(kind, Resolution::Deg1, StepHours::H1, 2),
                );
                project.layers = match layout {
                    0 => vec![imported.clone()],
                    1 | 3 => vec![imported.clone(), empty],
                    2 | 4 => vec![empty, imported.clone()],
                    _ => {
                        // The whole scene has an object, but this tile does
                        // not: tile culling must behave like a raster-only scene.
                        empty.visible = true;
                        let mut object = Object::new(ToolKind::Brush, "Far away", 2);
                        object.geometry = Geometry::Stroke {
                            chains: vec![vec![LocalPoint::new(0.0, 0.0)]],
                        };
                        object
                            .props
                            .get_mut(PropId::Position)
                            .unwrap()
                            .set_base(PropValue::LonLat(LonLat::new(-90.0, -45.0).unwrap()));
                        object
                            .props
                            .get_mut(PropId::SizeKm)
                            .unwrap()
                            .set_base(PropValue::F32(100.0));
                        empty.objects.push(object);
                        vec![imported.clone(), empty]
                    }
                };
                for filtered in [false, true] {
                    let imported = project
                        .layers
                        .iter_mut()
                        .find(|layer| layer.raster.is_some())
                        .unwrap();
                    // This band excludes the calm sample, and keeps the moving
                    // sample. With no filter, deliberate calm remains covered.
                    imported.speed_range = filtered.then_some(SpeedRange {
                        min_mps: 0.1,
                        max_mps: 6.0,
                    });
                    let scene = flatten(&project, 0);
                    let culled = tile_scene(&scene, &digests_of(&scene), tile).scene;
                    assert!(culled.objects.is_empty());
                    assert_eq!(culled.rasters.len(), 1);
                    let expected = [
                        Sample {
                            uv,
                            coverage: 1.0,
                            kind,
                        },
                        if filtered {
                            Sample::default()
                        } else {
                            Sample {
                                uv: Uv::default(),
                                coverage: 1.0,
                                kind,
                            }
                        },
                        Sample::default(),
                        Sample::default(),
                    ];
                    for scene in [&scene, &culled] {
                        let actual = evaluator.evaluate_samples(scene, &points).unwrap();
                        assert_eq!(actual.len(), expected.len());
                        for (index, (got, want)) in actual.iter().zip(expected).enumerate() {
                            assert!(
                                got.coverage == want.coverage
                                    && got.kind == want.kind
                                    && (got.uv.u - want.uv.u).abs() < 1e-5
                                    && (got.uv.v - want.uv.v).abs() < 1e-5,
                                "{} {kind:?} history={history} layout={layout} filtered={filtered} point={index}: {got:?}, expected {want:?}",
                                evaluator.backend_name(),
                            );
                        }
                    }
                }
                // Past the imported sequence, only empty paint layers remain.
                let empty = evaluator
                    .evaluate_samples(&flatten(&project, 1), &points)
                    .unwrap();
                assert_eq!(empty, vec![Sample::default(); points.len()]);
            }
        }
    }
}

#[test]
fn cpu_empty_layers_leave_imported_fields_visible() {
    check_empty_layers(&CpuEvaluator);
}

#[test]
fn gpu_empty_layers_leave_imported_fields_visible() {
    let gpu = match GpuEvaluator::new() {
        Ok(gpu) => gpu,
        Err(error) => {
            println!("no GPU available ({error}); skipping GPU empty-layer comparison");
            return;
        }
    };
    println!("checking empty layers on {}", gpu.adapter);
    check_empty_layers(&gpu);
}
