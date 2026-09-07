//! The part of a scene a tile can see (spec.md 7.10, M31).
//!
//! A tile is keyed by the content of the scene that produced it. Keyed by
//! the *whole* scene, every edit anywhere re-addressed every tile on the
//! map: a stroke in the Bay of Biscay re-rendered the Tasman Sea. So a tile
//! is keyed by, and rendered from, the objects that can reach it — the ones
//! whose spherical cap touches the tile's rectangle — and an edit changes
//! the keys of the tiles the edited object reaches and no others.
//!
//! The sub-scene renders exactly as the whole scene does at the tile's
//! pixels. An object outside its cap writes nothing at any of them: its
//! weight is zero, and a blend, a replace, a mask and a modifier are all
//! the identity at zero weight, so leaving it out changes no pixel. The two
//! exceptions are objects that read the field somewhere *else* — a clone
//! stamp, a warp, a liquify — which keep everything beneath them in their
//! layer, wherever it is; and an inverted mask, which covers everything
//! outside its footprint and so reaches every tile.

use ve_core::LonLat;

use crate::cache::{EVALUATOR_VERSION, SceneHash, object_digest, raster_digest};
use crate::scene::{FlatObject, Modifier, Scene};
use crate::tile::TileId;

/// The content hash of every object and every raster of a scene, computed
/// once per frame and combined per tile.
///
/// Hashing an object is the expensive part of keying a tile; a viewport is a
/// hundred tiles and each sees a fraction of the objects, so the objects are
/// hashed once and each tile's key is a hash of the digests it keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    pub objects: Vec<[u8; 32]>,
    pub rasters: Vec<[u8; 32]>,
}

/// Digests every object and raster of a scene, in order.
pub fn digests_of(scene: &Scene) -> Digests {
    Digests {
        objects: scene.objects.iter().map(object_digest).collect(),
        rasters: scene.rasters.iter().map(raster_digest).collect(),
    }
}

/// A tile's sub-scene and the key it is cached under.
#[derive(Debug, Clone, PartialEq)]
pub struct TileScene {
    /// The objects that reach the tile and every raster, in the scene's own
    /// order, with the layer tags they had.
    pub scene: Scene,
    /// Its content hash: what [`crate::cache::scene_hash`] gives for `scene`,
    /// built from the digests rather than by hashing the objects again.
    pub hash: SceneHash,
}

/// Whether an object reaches a tile: its cap comes within the tile's own
/// radius of the tile's centre. Conservative by the triangle inequality —
/// an object that does reach is never left out; one that only nearly does
/// may be kept, and costs a cap cull per pixel as it always did.
fn reaches(object: &FlatObject, centre: LonLat, radius_m: f64) -> bool {
    object.invert || object.frame.distance_m(centre) <= object.cap_radius_m + radius_m
}

/// Whether an object reads its layer at a position other than the one it
/// writes, and so needs everything beneath it wherever that is.
fn reads_elsewhere(object: &FlatObject) -> bool {
    object.clone_source.is_some()
        || matches!(object.modifier, Some(Modifier::Warp(_) | Modifier::Smear))
}

/// The tile's centre and the greatest distance from it to any point of the
/// tile, in metres on the globe. The farthest point of a rectangle bounded
/// by meridians and parallels from its centre is a corner.
fn tile_cap(tile: TileId) -> (LonLat, f64) {
    let b = tile.bounds();
    let centre = LonLat {
        lon: ve_core::geo::normalize_lon((b.west + b.east) * 0.5),
        lat: ((b.north + b.south) * 0.5).clamp(-90.0, 90.0),
    };
    let corners = [
        (b.west, b.north),
        (b.east, b.north),
        (b.west, b.south),
        (b.east, b.south),
    ];
    let radius = corners
        .iter()
        .map(|&(lon, lat)| {
            centre.distance_m(LonLat {
                lon: ve_core::geo::normalize_lon(lon),
                lat: lat.clamp(-90.0, 90.0),
            })
        })
        .fold(0.0, f64::max);
    // A hair of slack for the f64 in the two distances.
    (centre, radius * 1.001)
}

/// The part of `scene` that `tile` can see, and its key.
///
/// `digests` must be [`digests_of`] the same scene.
pub fn tile_scene(scene: &Scene, digests: &Digests, tile: TileId) -> TileScene {
    let (centre, radius_m) = tile_cap(tile);
    let mut keep: Vec<bool> = scene
        .objects
        .iter()
        .map(|object| reaches(object, centre, radius_m))
        .collect();
    // An object that reads elsewhere keeps its whole layer beneath it: what
    // it reads may lie under any tile.
    for index in 0..scene.objects.len() {
        if keep[index] && reads_elsewhere(&scene.objects[index]) {
            let layer = scene.objects[index].layer;
            for (below, object) in scene.objects[..index].iter().enumerate() {
                if object.layer == layer {
                    keep[below] = true;
                }
            }
        }
    }

    // The kept objects, and where each original index lands among them, so
    // a raster's `z` — the count of objects beneath it — can be remapped.
    let mut kept_before = Vec::with_capacity(scene.objects.len() + 1);
    let mut count = 0usize;
    for &kept in &keep {
        kept_before.push(count);
        if kept {
            count += 1;
        }
    }
    kept_before.push(count);

    let mut hasher = blake3::Hasher::new();
    hasher.update(&EVALUATOR_VERSION.to_le_bytes());
    hasher.update(&(count as u64).to_le_bytes());
    let mut objects = Vec::with_capacity(count);
    for (index, object) in scene.objects.iter().enumerate() {
        if keep[index] {
            hasher.update(&digests.objects[index]);
            objects.push(object.clone());
        }
    }
    // Every raster is kept: a lattice costs one extent check a pixel where
    // the tile lies outside it, and most are global.
    hasher.update(&(scene.rasters.len() as u64).to_le_bytes());
    let rasters = scene
        .rasters
        .iter()
        .zip(&digests.rasters)
        .map(|(raster, digest)| {
            let z = kept_before[raster.z.min(scene.objects.len())];
            hasher.update(&(z as u64).to_le_bytes());
            hasher.update(digest);
            let mut remapped = raster.clone();
            remapped.z = z;
            remapped
        })
        .collect();

    TileScene {
        scene: Scene { objects, rasters },
        hash: *hasher.finalize().as_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aeqd::Frame;
    use crate::cache::scene_hash;
    use crate::cpu::CpuEvaluator;
    use crate::evaluator::FieldEvaluator;
    use crate::scene::{DirectionMode, EdgeMode, Motion, OffsetMode, SpeedMode};
    use crate::sdf::Shape;
    use ve_core::angle::Angle;
    use ve_core::project::FieldKind;

    fn disc(lon: f64, lat: f64, radius_m: f64, speed: f64, layer: u32) -> FlatObject {
        FlatObject {
            layer,
            kind: FieldKind::Wind,
            erased: Vec::new(),
            frame: Frame::new(LonLat::new(lon, lat).expect("valid"), 0.0, 100.0),
            shape: Shape::Disc { radius_m },
            cap_radius_m: radius_m,
            speed: SpeedMode::Constant(speed),
            direction: DirectionMode::Constant(Angle::new(90.0)),
            feather: 0.2,
            edge_mode: EdgeMode::Blend,
            gradient_axis: Angle::new(90.0),
            gradient_extent: radius_m,
            path: Vec::new(),
            clone_source: None,
            clone_offset: OffsetMode::Aligned,
            modifier: None,
            smear: Vec::new(),
            invert: false,
            capture: None,
            erases: false,
            motion: Motion::default(),
        }
    }

    fn tile() -> TileId {
        // The Bay of Biscay at level 4: 11.25 degrees a side, west of 0 and
        // north of 45.
        TileId::new(4, 15, 3).expect("valid tile")
    }

    /// The key is the scene's own hash of the sub-scene, so the two ways of
    /// arriving at a key agree, and the sub-scene renders as the whole scene
    /// does at every pixel of the tile.
    #[test]
    fn a_tiles_sub_scene_renders_as_the_whole_scene_does() {
        let scene = Scene {
            objects: vec![
                disc(-5.0, 45.0, 600_000.0, 10.0, 0),
                disc(150.0, -30.0, 900_000.0, 20.0, 0),
                disc(-8.0, 43.0, 400_000.0, 5.0, 1),
                disc(100.0, 10.0, 300_000.0, 7.0, 1),
            ],
            rasters: Vec::new(),
        };
        let sub = tile_scene(&scene, &digests_of(&scene), tile());
        assert_eq!(sub.scene.objects.len(), 2, "two of the four reach the tile");
        assert_eq!(sub.hash, scene_hash(&sub.scene));

        let points = tile().sample_positions();
        let whole = CpuEvaluator
            .evaluate_samples(&scene, &points)
            .expect("whole");
        let part = CpuEvaluator
            .evaluate_samples(&sub.scene, &points)
            .expect("part");
        assert_eq!(whole, part);
    }

    /// The point of it: an edit on the far side of the world leaves the
    /// tile's key alone, and one within reach changes it.
    #[test]
    fn a_distant_edit_leaves_the_key_alone_and_a_near_one_changes_it() {
        let mut scene = Scene {
            objects: vec![
                disc(-5.0, 45.0, 600_000.0, 10.0, 0),
                disc(150.0, -30.0, 900_000.0, 20.0, 0),
            ],
            rasters: Vec::new(),
        };
        let before = tile_scene(&scene, &digests_of(&scene), tile()).hash;

        scene.objects[1].speed = SpeedMode::Constant(25.0);
        let far_edit = tile_scene(&scene, &digests_of(&scene), tile()).hash;
        assert_eq!(
            before, far_edit,
            "an edit out of reach must not re-key the tile"
        );

        scene.objects[0].speed = SpeedMode::Constant(12.0);
        let near_edit = tile_scene(&scene, &digests_of(&scene), tile()).hash;
        assert_ne!(before, near_edit, "an edit within reach must");
    }

    /// A clone stamp reads its layer from somewhere else, so what it reads
    /// comes with it even from out of reach; an inverted mask reaches
    /// everywhere; and a raster's z follows the objects kept beneath it.
    #[test]
    fn readers_keep_what_they_read_and_rasters_keep_their_place() {
        let mut clone = disc(-5.0, 45.0, 600_000.0, 0.0, 1);
        clone.clone_source = Some(LonLat::new(150.0, -30.0).expect("valid"));
        let mut inverted = disc(150.0, -30.0, 100_000.0, 0.0, 0);
        inverted.invert = true;
        inverted.erases = true;
        let grid =
            ve_core::raster::RasterGrid::new(4, 3, 0.0, 10.0, 1.0, 1.0, vec![[1.0, 0.0]; 12])
                .expect("valid grid");
        let scene = Scene {
            objects: vec![
                disc(150.0, -30.0, 900_000.0, 20.0, 0),
                inverted,
                disc(150.0, -30.0, 900_000.0, 20.0, 1),
                clone,
            ],
            rasters: vec![crate::scene::FlatRaster {
                z: 2,
                layer: 1,
                kind: FieldKind::Wind,
                grid: std::sync::Arc::new(grid),
                speed_range: None,
                erased: Vec::new(),
            }],
        };
        let sub = tile_scene(&scene, &digests_of(&scene), tile());
        // The distant layer-0 disc goes; the inverted mask stays; the clone
        // keeps the layer-1 disc it reads.
        assert_eq!(sub.scene.objects.len(), 3);
        assert!(sub.scene.objects[0].invert);
        assert_eq!(
            sub.scene.rasters[0].z, 1,
            "one object kept beneath the raster"
        );
        assert_eq!(sub.hash, scene_hash(&sub.scene));
    }
}
