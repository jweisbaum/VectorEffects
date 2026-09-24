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
//!
//! An imported field is culled the same way (M33), which is what keeps an
//! edit to one local to it: a tile the lattice cannot be read at drops the
//! raster outright, a stroke of the eraser is kept only by the tiles it
//! passes over, and the speed filter's upper end is clamped to the greatest
//! speed the tile holds — so a band wider than the tile's own field keys
//! the tile exactly as no band at all does.

use std::collections::HashMap;
use std::sync::Mutex;

use ve_core::LonLat;
use ve_core::document::SpeedRange;

use crate::cache::{EVALUATOR_VERSION, SceneHash, object_digest, raster_digest};
use crate::scene::{FlatObject, FlatRaster, Modifier, Scene};
use crate::tile::{TileBounds, TileId};

/// The content hash of every object of a scene, computed once per frame and
/// combined per tile.
///
/// Hashing an object is the expensive part of keying a tile; a viewport is a
/// hundred tiles and each sees a fraction of the objects, so the objects are
/// hashed once and each tile's key is a hash of the digests it keeps. A
/// raster is hashed per tile instead (M33): what the tile keeps of it — its
/// erasures, its band — is not the same from one tile to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    pub objects: Vec<[u8; 32]>,
}

/// Digests every object of a scene, in order.
pub fn digests_of(scene: &Scene) -> Digests {
    Digests {
        objects: scene.objects.iter().map(object_digest).collect(),
    }
}

/// The greatest speed a lattice holds inside a tile, memoised.
///
/// A tile's key needs it on every frame the map draws, and it depends only
/// on the lattice's content and the tile — neither of which changes — so it
/// is computed once. The lattice is identified by its own content hash, so
/// a re-imported or edited file is a different entry rather than a stale one.
type TileMaxima = HashMap<([u8; 32], TileId), Option<f32>>;
static TILE_MAXIMA: Mutex<Option<TileMaxima>> = Mutex::new(None);

/// How many entries the memo keeps: a few viewports of a few lattices.
const MAXIMA_KEPT: usize = 8192;

fn tile_maximum(raster: &FlatRaster, tile: TileId, bounds: &TileBounds) -> Option<f32> {
    let key = (raster.grid.hash, tile);
    if let Ok(mut held) = TILE_MAXIMA.lock() {
        let memo = held.get_or_insert_with(TileMaxima::new);
        if let Some(found) = memo.get(&key) {
            return *found;
        }
    }
    let found = raster
        .grid
        .max_speed_in(bounds.west, bounds.east, bounds.north, bounds.south);
    if let Ok(mut held) = TILE_MAXIMA.lock() {
        let memo = held.get_or_insert_with(TileMaxima::new);
        // Dropped wholesale rather than by age: the memo is a saving, not a
        // correctness requirement, and an LRU clock would cost more than the
        // lookups it protects.
        if memo.len() >= MAXIMA_KEPT {
            memo.clear();
        }
        memo.insert(key, found);
    }
    found
}

/// What a tile keeps of an imported field (M33), or `None` where the tile
/// can read no node of it at all.
///
/// The band is clamped to the field the tile actually holds: a band that
/// admits everything here is no band at all, and one whose top is above
/// everything here keeps the same nodes as any wider band. A blended sample
/// can be slower than either node it came from but never faster, so the top
/// is the only end that can be bounded this way.
fn tile_raster(
    raster: &FlatRaster,
    tile: TileId,
    bounds: &TileBounds,
    centre: LonLat,
    radius_m: f64,
) -> Option<FlatRaster> {
    let most = tile_maximum(raster, tile, bounds)?;
    let mut kept = raster.clone();
    kept.speed_range = raster.speed_range.and_then(|band| {
        if band.min_mps <= 0.0 && band.max_mps >= most {
            return None;
        }
        Some(SpeedRange {
            min_mps: band.min_mps,
            max_mps: band.max_mps.min(most),
        })
    });
    kept.erased.retain(|erasure| {
        if erasure.projection == crate::aeqd::Space::Orthographic.choice() {
            // The screen radius is compressed near the limb; it cannot be
            // used as a geographic radius to discard part of the stroke.
            return true;
        }
        erasure
            .chains
            .iter()
            .flatten()
            .any(|point| centre.distance_m(*point) <= erasure.radius_m + radius_m)
    });
    Some(kept)
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
        || matches!(
            object.modifier,
            Some(Modifier::Warp(_) | Modifier::Smear | Modifier::Relocate { .. })
        )
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
    let bounds = tile.bounds();
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
    crate::cache::hash_speed_ranges(&mut hasher, scene);
    hasher.update(&(count as u64).to_le_bytes());
    let mut objects = Vec::with_capacity(count);
    for (index, object) in scene.objects.iter().enumerate() {
        if keep[index] {
            hasher.update(&digests.objects[index]);
            objects.push(object.clone());
        }
    }
    // Each raster as this tile sees it: dropped where the lattice cannot be
    // read, and carrying only the erasures and the band that reach it (M33).
    let rasters: Vec<FlatRaster> = scene
        .rasters
        .iter()
        .filter_map(|raster| {
            let mut kept = tile_raster(raster, tile, &bounds, centre, radius_m)?;
            // Operators can raise the layer's final speed above any source
            // node, so the source maximum cannot bound their output filter.
            if scene
                .objects
                .iter()
                .any(|object| object.layer == raster.layer)
            {
                kept.speed_range = raster.speed_range;
            }
            kept.z = kept_before[raster.z.min(scene.objects.len())];
            Some(kept)
        })
        .collect();
    hasher.update(&(rasters.len() as u64).to_le_bytes());
    for raster in &rasters {
        hasher.update(&(raster.z as u64).to_le_bytes());
        hasher.update(&raster_digest(raster));
    }

    TileScene {
        scene: Scene {
            objects,
            rasters,
            speed_ranges: scene.speed_ranges.clone(),
        },
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
            transition: Default::default(),
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
            speed_ranges: Default::default(),
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
            speed_ranges: Default::default(),
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
        // Over the tile, or it would be culled with everything else that is
        // not (M33).
        let grid =
            ve_core::raster::RasterGrid::new(4, 3, -11.0, 56.0, 1.0, 1.0, vec![[1.0, 0.0]; 12])
                .expect("valid grid");
        let scene = Scene {
            speed_ranges: Default::default(),
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

    fn raster_of(lon0: f64, lat0: f64, speeds: &[f32], band: Option<(f32, f32)>) -> FlatRaster {
        let uv: Vec<[f32; 2]> = speeds.iter().map(|s| [*s, 0.0]).collect();
        let grid =
            ve_core::raster::RasterGrid::new(speeds.len() as u32, 1, lon0, lat0, 1.0, 1.0, uv)
                .expect("valid grid");
        FlatRaster {
            z: 0,
            layer: 0,
            kind: FieldKind::Wind,
            grid: std::sync::Arc::new(grid),
            speed_range: band.map(|(min_mps, max_mps)| SpeedRange { min_mps, max_mps }),
            erased: Vec::new(),
        }
    }

    /// Spec 7.10 (M33): an imported field reaches only the tiles its lattice
    /// covers, so a tile somewhere else keeps nothing of it and is not
    /// re-keyed when its filter or its erasures change.
    #[test]
    fn a_tile_outside_a_lattice_keeps_nothing_of_it() {
        let here = raster_of(-11.0, 50.0, &[5.0, 9.0], None);
        let elsewhere = raster_of(150.0, -30.0, &[5.0, 9.0], None);
        let scene = Scene {
            speed_ranges: Default::default(),
            objects: Vec::new(),
            rasters: vec![here, elsewhere],
        };
        let sub = tile_scene(&scene, &digests_of(&scene), tile());
        assert_eq!(sub.scene.rasters.len(), 1, "only the one over the tile");
        assert_eq!(sub.hash, scene_hash(&sub.scene));

        // And filtering the distant one leaves this tile's key alone.
        let mut filtered = scene.clone();
        filtered.rasters[1].speed_range = Some(SpeedRange {
            min_mps: 1.0,
            max_mps: 2.0,
        });
        assert_eq!(
            tile_scene(&filtered, &digests_of(&filtered), tile()).hash,
            sub.hash
        );
    }

    /// A band that admits everything the tile holds keys it as no band does,
    /// and one whose top is above everything here keys it as any wider band
    /// does — so dragging the filter over speeds a tile has none of leaves
    /// that tile alone (M33).
    #[test]
    fn a_band_is_clamped_to_the_speeds_the_tile_holds() {
        let key = |band: Option<(f32, f32)>| {
            let scene = Scene {
                speed_ranges: Default::default(),
                objects: Vec::new(),
                rasters: vec![raster_of(-11.0, 50.0, &[5.0, 9.0], band)],
            };
            tile_scene(&scene, &digests_of(&scene), tile()).hash
        };
        let unfiltered = key(None);
        assert_eq!(
            key(Some((0.0, 20.0))),
            unfiltered,
            "a band over everything is no band"
        );
        assert_eq!(
            key(Some((2.0, 20.0))),
            key(Some((2.0, 50.0))),
            "two bands above the tile's fastest are one band"
        );
        assert_ne!(
            key(Some((2.0, 20.0))),
            unfiltered,
            "a band that drops something is not no band"
        );
        assert_ne!(
            key(Some((2.0, 7.0))),
            key(Some((2.0, 20.0))),
            "a band that cuts into the field re-keys the tile"
        );
    }

    /// An erased stroke is kept by the tiles it passes over and no others,
    /// so erasing in one place does not re-render the map (M33).
    #[test]
    fn an_erasure_is_kept_only_where_it_reaches() {
        let erasure = |lon: f64, lat: f64| crate::scene::FlatRasterErasure {
            projection_origin: None,
            projection: 0,
            chains: vec![vec![LonLat::new(lon, lat).expect("valid")]],
            radius_m: 100_000.0,
            square: false,
            projected: false,
            feather: 0.5,
        };
        let mut raster = raster_of(-11.0, 50.0, &[5.0, 9.0], None);
        let keyed = |raster: &FlatRaster| {
            let scene = Scene {
                speed_ranges: Default::default(),
                objects: Vec::new(),
                rasters: vec![raster.clone()],
            };
            tile_scene(&scene, &digests_of(&scene), tile())
        };
        let before = keyed(&raster).hash;

        raster.erased.push(erasure(150.0, -30.0));
        let sub = keyed(&raster);
        assert!(
            sub.scene.rasters[0].erased.is_empty(),
            "a distant erasure is not this tile's"
        );
        assert_eq!(sub.hash, before, "and does not re-key it");

        raster.erased.push(erasure(-6.0, 50.0));
        let sub = keyed(&raster);
        assert_eq!(
            sub.scene.rasters[0].erased.len(),
            1,
            "the one over the tile is kept"
        );
        assert_ne!(sub.hash, before, "and re-keys it");
    }
}
