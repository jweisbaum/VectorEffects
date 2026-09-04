//! The render cache.
//!
//! Tiles are keyed by the *content* of the scene that produced them, not by any
//! hand-maintained notion of what changed (spec.md 7.10). An edit that alters
//! what a frame looks like changes its hash; an edit that does not — renaming a
//! layer, moving the camera — leaves it alone. Invalidation is therefore exact
//! by construction, with no dirty flags to get wrong.
//!
//! **Nothing here is project data.** The cache lives in the OS cache directory
//! and deleting it while the app is closed is always safe and always lossless
//! (invariant 1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use crate::aeqd::Space;
use crate::error::{RenderError, Result};
use crate::preview::Quality;
use crate::scene::{
    DirectionMode, EdgeMode, FlatObject, Modifier, OffsetMode, Scene, SpeedMode, Warp,
};
use crate::sdf::Shape;
use crate::tile::TileId;

/// Default cap on disk usage.
pub const DEFAULT_CAPACITY_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Content hash of a scene.
pub type SceneHash = [u8; 32];

/// Hashes a flattened scene.
///
/// Everything that affects the rendered field is fed in, in z-order. Anything
/// that does not affect it is absent, which is what stops an irrelevant edit
/// from throwing away a cache.
pub fn scene_hash(scene: &Scene) -> SceneHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(scene.objects.len() as u64).to_le_bytes());
    for object in &scene.objects {
        hash_object(&mut hasher, object);
    }
    // An imported field is identified by its own content hash — the lattice
    // and every sample — and by where it sits in the stack. A different time
    // slice is a different grid, so scrubbing through a file's messages
    // changes the key exactly when the slice changes and not otherwise.
    hasher.update(&(scene.rasters.len() as u64).to_le_bytes());
    for raster in &scene.rasters {
        hasher.update(&(raster.z as u64).to_le_bytes());
        hasher.update(&raster.grid.hash);
        // The speed band decides which of the lattice's samples are drawn at
        // all, so a tile keyed without it would be served from before the
        // filter was set (spec.md 7.10).
        match raster.speed_range {
            None => hasher.update(&[0]),
            Some(band) => {
                hasher.update(&[1]);
                hasher.update(&band.min_mps.to_le_bytes());
                hasher.update(&band.max_mps.to_le_bytes())
            }
        };
    }
    *hasher.finalize().as_bytes()
}

fn hash_f64(hasher: &mut blake3::Hasher, value: f64) {
    hasher.update(&value.to_le_bytes());
}

fn hash_object(hasher: &mut blake3::Hasher, object: &FlatObject) {
    hash_f64(hasher, object.frame.anchor.lon);
    hash_f64(hasher, object.frame.anchor.lat);
    hash_f64(hasher, object.frame.rotation_deg);
    hash_f64(hasher, object.frame.scale);
    // The space changes what the geometry means, so two objects that differ
    // only in it must not share a tile.
    hasher.update(&[match object.frame.space {
        Space::Geodesic => 0,
        Space::Projected => 1,
    }]);
    hash_f64(hasher, object.cap_radius_m);
    hash_f64(hasher, object.feather);
    hash_f64(hasher, object.gradient_axis.degrees());
    hash_f64(hasher, object.gradient_extent);
    hasher.update(&[match object.edge_mode {
        EdgeMode::Blend => 0,
        EdgeMode::Replace => 1,
    }]);

    match &object.shape {
        Shape::Capsule { chains, radius_m } => {
            hasher.update(&[0]);
            hash_f64(hasher, *radius_m);
            hasher.update(&(chains.len() as u64).to_le_bytes());
            for chain in chains {
                hash_points(hasher, chain);
            }
        }
        Shape::Disc { radius_m } => {
            hasher.update(&[1]);
            hash_f64(hasher, *radius_m);
        }
        Shape::Annulus {
            radius_m,
            half_width_m,
        } => {
            hasher.update(&[2]);
            hash_f64(hasher, *radius_m);
            hash_f64(hasher, *half_width_m);
        }
        Shape::Rect {
            half_width_m,
            half_height_m,
        } => {
            hasher.update(&[3]);
            hash_f64(hasher, *half_width_m);
            hash_f64(hasher, *half_height_m);
        }
        Shape::Polygon { ring } => {
            hasher.update(&[4]);
            hash_points(hasher, ring);
        }
        // A distinct tag, not a reuse of the capsule's: a round and a square
        // stroke over the same chains look different, so they must not share a
        // cache entry.
        Shape::SweptSquare {
            chains,
            half_size_m,
        } => {
            hasher.update(&[5]);
            hash_f64(hasher, *half_size_m);
            hasher.update(&(chains.len() as u64).to_le_bytes());
            for chain in chains {
                hash_points(hasher, chain);
            }
        }
    }

    match object.speed {
        SpeedMode::Constant(speed) => {
            hasher.update(&[0]);
            hash_f64(hasher, speed);
        }
        SpeedMode::Radial {
            centre,
            edge,
            extent,
        } => {
            hasher.update(&[1]);
            hash_f64(hasher, centre);
            hash_f64(hasher, edge);
            hash_f64(hasher, extent);
        }
        SpeedMode::Axis { start, end } => {
            hasher.update(&[2]);
            hash_f64(hasher, start);
            hash_f64(hasher, end);
        }
    }

    match &object.direction {
        DirectionMode::Constant(bearing) => {
            hasher.update(&[0]);
            hash_f64(hasher, bearing.degrees());
        }
        DirectionMode::Toward(target) => {
            hasher.update(&[1]);
            hash_f64(hasher, target.lon);
            hash_f64(hasher, target.lat);
        }
        DirectionMode::Away(target) => {
            hasher.update(&[5]);
            hash_f64(hasher, target.lon);
            hash_f64(hasher, target.lat);
        }
        DirectionMode::Axis { start, end } => {
            hasher.update(&[2]);
            hash_f64(hasher, start.degrees());
            hash_f64(hasher, end.degrees());
        }
        DirectionMode::AlongPath { offset } => {
            hasher.update(&[3]);
            hash_f64(hasher, offset.degrees());
        }
        DirectionMode::Tangential { clockwise } => {
            hasher.update(&[4]);
            hasher.update(&[u8::from(*clockwise)]);
        }
    }

    hash_points(hasher, &object.path);

    // Where a clone stamp reads from, and whether that place travels with the
    // brush. Both change every pixel the object paints without touching its
    // footprint, so a tile keyed without them would be served from the cache
    // after the source moved — the stale-frame failure the cache exists to
    // avoid rather than to cause.
    match object.clone_source {
        None => hasher.update(&[0]),
        Some(source) => {
            hasher.update(&[1]);
            hash_f64(hasher, source.lon);
            hash_f64(hasher, source.lat);
            hasher.update(&[match object.clone_offset {
                OffsetMode::Aligned => 0,
                OffsetMode::Fixed => 1,
            }])
        }
    };

    // The object's own movement, which is added to every vector it paints and
    // so changes the frame without moving the footprint (spec.md 9.3). Two
    // steps of a travelling stroke have the same geometry and different
    // fields; a key that ignored this would serve the first for the second.
    for component in object.motion.omega {
        hash_f64(hasher, component);
    }
    hash_f64(hasher, object.motion.scale_rate);

    // Which side of its footprint the object writes on. A mask and its
    // inverse cover disjoint halves of the globe from the same geometry, so
    // the flag has to reach the key (spec.md 7.10).
    hasher.update(&[u8::from(object.invert)]);

    // What a modifier does to the field beneath it. Same reasoning as the
    // clone stamp's source: every one of these changes what the object writes
    // without moving its footprint, so a key that ignored them would serve a
    // tile painted by the previous amount (spec.md 7.10).
    match object.modifier {
        None => hasher.update(&[0]),
        Some(Modifier::Gain(gain)) => {
            hasher.update(&[1]);
            hash_f64(hasher, gain);
            hasher
        }
        Some(Modifier::Radial(fraction)) => {
            hasher.update(&[2]);
            hash_f64(hasher, fraction);
            hasher
        }
        Some(Modifier::Turn(degrees)) => {
            hasher.update(&[3]);
            hash_f64(hasher, degrees);
            hasher
        }
        Some(Modifier::Warp(Warp::Push { x, y })) => {
            hasher.update(&[4]);
            hash_f64(hasher, x);
            hash_f64(hasher, y);
            hasher
        }
        Some(Modifier::Warp(Warp::Twist { degrees })) => {
            hasher.update(&[5]);
            hash_f64(hasher, degrees);
            hasher
        }
    };
}

fn hash_points(hasher: &mut blake3::Hasher, points: &[[f64; 2]]) {
    hasher.update(&(points.len() as u64).to_le_bytes());
    for point in points {
        hash_f64(hasher, point[0]);
        hash_f64(hasher, point[1]);
    }
}

/// Everything that identifies one cached tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// Content hash of the scene.
    pub scene: SceneHash,
    /// Which tile.
    pub tile: TileId,
    /// How coarsely it was rendered.
    pub quality: Quality,
}

impl TileKey {
    /// The cache file name for this key.
    pub fn digest(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.scene);
        hasher.update(&self.tile.z.to_le_bytes());
        hasher.update(&self.tile.x.to_le_bytes());
        hasher.update(&self.tile.y.to_le_bytes());
        hasher.update(&[self.quality.stride() as u8]);
        hasher.finalize().to_hex().to_string()
    }
}

/// What the cache currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Tiles stored.
    pub entries: usize,
    /// Total bytes on disk.
    pub bytes: u64,
    /// Cap, in bytes.
    pub capacity: u64,
}

#[derive(Debug, Default)]
struct Index {
    entries: HashMap<String, (u64, u64)>,
    bytes: u64,
    clock: u64,
}

/// A disk-backed, size-capped tile cache.
#[derive(Debug)]
pub struct RenderCache {
    root: PathBuf,
    capacity: u64,
    index: Mutex<Index>,
    /// Keys being rendered right now, so a second request for one of them
    /// waits for the first rather than rendering it again. A `Vec`: a handful
    /// of entries, scanned, never iterated as a map.
    in_flight: Mutex<Vec<TileKey>>,
    landed: Condvar,
}

impl RenderCache {
    /// Opens (or creates) a cache under `root`.
    ///
    /// Existing files are adopted rather than discarded, so a restart keeps
    /// whatever was already rendered.
    pub fn open(root: impl Into<PathBuf>, capacity: u64) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;

        let mut index = Index::default();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if !metadata.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                index.clock += 1;
                index.bytes += metadata.len();
                index.entries.insert(name, (metadata.len(), index.clock));
            }
        }

        Ok(Self {
            root,
            capacity,
            index: Mutex::new(index),
            in_flight: Mutex::new(Vec::new()),
            landed: Condvar::new(),
        })
    }

    /// The tile for `key`: from the cache, or rendered once and stored.
    ///
    /// Single-flight. The map asks for the tiles of the step it is showing at
    /// the same moment the render pool starts on that step, and without this
    /// both would evaluate the same tile — an identical result, twice the cost,
    /// on the one step the user is waiting for. The second caller waits for
    /// the first and reads what it stored. A render that fails releases the
    /// key, so a waiter retries rather than hanging on a tile that never lands.
    pub fn get_or_render(
        &self,
        key: &TileKey,
        render: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        loop {
            if let Some(cached) = self.get(key) {
                return Ok(cached);
            }
            let mut in_flight = self
                .in_flight
                .lock()
                .map_err(|_| RenderError::Flatten("in-flight lock was poisoned".to_owned()))?;
            if !in_flight.contains(key) {
                in_flight.push(*key);
                break;
            }
            // Someone else is on it: sleep until any tile lands, then look again.
            let guard = self
                .landed
                .wait(in_flight)
                .map_err(|_| RenderError::Flatten("in-flight lock was poisoned".to_owned()))?;
            drop(guard);
        }

        let rendered = render();
        if let Ok(bytes) = &rendered
            && let Err(err) = self.put(key, bytes)
        {
            // A cache write failure must not fail the request.
            tracing::warn!(%err, "could not cache tile");
        }
        if let Ok(mut in_flight) = self.in_flight.lock() {
            in_flight.retain(|k| k != key);
        }
        self.landed.notify_all();
        rendered
    }

    fn path_for(&self, digest: &str) -> PathBuf {
        self.root.join(digest)
    }

    /// Whether a tile is cached, without reading it.
    ///
    /// A readiness probe, not a fetch: it neither touches the disk nor marks
    /// the entry recently used. A timeline asking after every tile of every
    /// step several times a second must not reorder eviction by asking.
    pub fn contains(&self, key: &TileKey) -> bool {
        self.index
            .lock()
            .is_ok_and(|index| index.entries.contains_key(&key.digest()))
    }

    /// Reads a tile, if it is cached.
    pub fn get(&self, key: &TileKey) -> Option<Vec<u8>> {
        let digest = key.digest();
        let mut index = self.index.lock().ok()?;
        if !index.entries.contains_key(&digest) {
            return None;
        }

        match std::fs::read(self.path_for(&digest)) {
            Ok(bytes) => {
                // Touch it, so eviction sees it as recently used.
                index.clock += 1;
                let clock = index.clock;
                if let Some(entry) = index.entries.get_mut(&digest) {
                    entry.1 = clock;
                }
                Some(bytes)
            }
            Err(_) => {
                // The file vanished underneath us: forget it rather than
                // reporting a cache hit that cannot be served.
                if let Some((size, _)) = index.entries.remove(&digest) {
                    index.bytes = index.bytes.saturating_sub(size);
                }
                None
            }
        }
    }

    /// Stores a tile, evicting least-recently-used entries if over capacity.
    pub fn put(&self, key: &TileKey, bytes: &[u8]) -> Result<()> {
        let digest = key.digest();
        write_atomically(&self.path_for(&digest), bytes)?;

        let mut index = self
            .index
            .lock()
            .map_err(|_| RenderError::Flatten("cache lock was poisoned".to_owned()))?;
        index.clock += 1;
        let clock = index.clock;
        if let Some((old, _)) = index.entries.insert(digest, (bytes.len() as u64, clock)) {
            index.bytes = index.bytes.saturating_sub(old);
        }
        index.bytes += bytes.len() as u64;

        self.evict(&mut index);
        Ok(())
    }

    fn evict(&self, index: &mut Index) {
        while index.bytes > self.capacity {
            let Some((digest, _)) = index
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(digest, entry)| (digest.clone(), *entry))
            else {
                break;
            };
            if let Some((size, _)) = index.entries.remove(&digest) {
                index.bytes = index.bytes.saturating_sub(size);
            }
            let _ = std::fs::remove_file(self.path_for(&digest));
        }
    }

    /// What the cache currently holds.
    pub fn stats(&self) -> CacheStats {
        match self.index.lock() {
            Ok(index) => CacheStats {
                entries: index.entries.len(),
                bytes: index.bytes,
                capacity: self.capacity,
            },
            Err(_) => CacheStats {
                entries: 0,
                bytes: 0,
                capacity: self.capacity,
            },
        }
    }

    /// Empties the cache. Always safe: nothing here is project data.
    pub fn clear(&self) -> Result<()> {
        let mut index = self
            .index
            .lock()
            .map_err(|_| RenderError::Flatten("cache lock was poisoned".to_owned()))?;
        for digest in index.entries.keys() {
            let _ = std::fs::remove_file(self.path_for(digest));
        }
        index.entries.clear();
        index.bytes = 0;
        Ok(())
    }
}

/// Writes via a temporary and renames, so a torn write is never readable.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("partial");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aeqd::Frame;
    use ve_core::LonLat;
    use ve_core::angle::Angle;

    struct TempCache {
        root: PathBuf,
        cache: RenderCache,
    }

    impl TempCache {
        fn new(label: &str, capacity: u64) -> Self {
            let root = std::env::temp_dir().join(format!(
                "ve-cache-{}-{label}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            let cache = RenderCache::open(&root, capacity).expect("opens");
            Self { root, cache }
        }
    }

    impl Drop for TempCache {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn object(speed: f64) -> FlatObject {
        FlatObject {
            frame: Frame::new(
                LonLat {
                    lon: 10.0,
                    lat: 20.0,
                },
                0.0,
                100.0,
            ),
            shape: Shape::Disc {
                radius_m: 500_000.0,
            },
            cap_radius_m: 500_000.0,
            speed: SpeedMode::Constant(speed),
            direction: DirectionMode::Constant(Angle::new(90.0)),
            feather: 0.2,
            edge_mode: EdgeMode::Blend,
            gradient_axis: Angle::new(90.0),
            gradient_extent: 500_000.0,
            path: Vec::new(),
            clone_source: None,
            clone_offset: OffsetMode::Aligned,
            modifier: None,
            invert: false,
            motion: crate::scene::Motion::default(),
        }
    }

    fn key(scene: &Scene) -> TileKey {
        TileKey {
            scene: scene_hash(scene),
            tile: TileId::new(2, 1, 1).expect("valid"),
            quality: Quality::Standard,
        }
    }

    #[test]
    fn an_identical_scene_hashes_identically() {
        let a = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        let b = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        assert_eq!(scene_hash(&a), scene_hash(&b));
    }

    fn raster(z: usize, value: f32) -> crate::scene::FlatRaster {
        let grid =
            ve_core::raster::RasterGrid::new(4, 3, 0.0, 10.0, 1.0, 1.0, vec![[value, 0.0]; 12])
                .expect("valid grid");
        crate::scene::FlatRaster {
            z,
            grid: std::sync::Arc::new(grid),
            speed_range: None,
        }
    }

    /// An imported field is part of what a frame looks like: its samples, and
    /// where it sits in the stack, both key the tile.
    #[test]
    fn an_imported_field_keys_the_frame() {
        let base = Scene {
            rasters: vec![raster(0, 5.0)],
            objects: vec![object(10.0)],
        };
        let original = scene_hash(&base);
        assert_ne!(
            original,
            scene_hash(&Scene {
                rasters: Vec::new(),
                objects: vec![object(10.0)],
            }),
            "a raster present at all"
        );
        assert_eq!(
            original,
            scene_hash(&Scene {
                rasters: vec![raster(0, 5.0)],
                objects: vec![object(10.0)],
            }),
            "the same samples again"
        );
        assert_ne!(
            original,
            scene_hash(&Scene {
                rasters: vec![raster(0, 6.0)],
                objects: vec![object(10.0)],
            }),
            "a different time slice"
        );
        assert_ne!(
            original,
            scene_hash(&Scene {
                rasters: vec![raster(1, 5.0)],
                objects: vec![object(10.0)],
            }),
            "the same slice above the object instead of below it"
        );
    }

    /// The property the whole design rests on: a change to what a frame looks
    /// like must change its hash.
    #[test]
    fn any_visible_change_changes_the_hash() {
        let base = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        let original = scene_hash(&base);

        let mut speed = base.clone();
        speed.objects[0].speed = SpeedMode::Constant(11.0);
        assert_ne!(scene_hash(&speed), original, "speed");

        let mut moved = base.clone();
        moved.objects[0].frame.anchor.lon += 0.001;
        assert_ne!(scene_hash(&moved), original, "position");

        let mut feather = base.clone();
        feather.objects[0].feather = 0.3;
        assert_ne!(scene_hash(&feather), original, "feather");

        let mut direction = base.clone();
        direction.objects[0].direction = DirectionMode::Constant(Angle::new(91.0));
        assert_ne!(scene_hash(&direction), original, "direction");

        let mut shape = base.clone();
        shape.objects[0].shape = Shape::Disc {
            radius_m: 500_001.0,
        };
        assert_ne!(scene_hash(&shape), original, "shape");

        let mut edge = base.clone();
        edge.objects[0].edge_mode = EdgeMode::Replace;
        assert_ne!(scene_hash(&edge), original, "edge mode");

        let mut added = base.clone();
        added.objects.push(object(5.0));
        assert_ne!(scene_hash(&added), original, "another object");

        // A clone stamp's whole output is decided by where it reads from and
        // whether that place moves with the brush. Neither touches the
        // footprint, so a hash taken from the geometry alone would serve the
        // previous source's pixels from the cache.
        let mut cloning = base.clone();
        cloning.objects[0].clone_source = Some(LonLat {
            lon: 40.0,
            lat: 0.0,
        });
        let becomes_a_clone = scene_hash(&cloning);
        assert_ne!(becomes_a_clone, original, "becoming a clone stamp");

        let mut moved_source = cloning.clone();
        moved_source.objects[0].clone_source = Some(LonLat {
            lon: 41.0,
            lat: 0.0,
        });
        assert_ne!(scene_hash(&moved_source), becomes_a_clone, "clone source");

        let mut fixed = cloning.clone();
        fixed.objects[0].clone_offset = OffsetMode::Fixed;
        assert_ne!(scene_hash(&fixed), becomes_a_clone, "offset mode");
    }

    /// The offset mode means nothing without a source, so it must not change
    /// the hash of an object that is not a clone stamp — a hash that moved
    /// would evict every tile whenever an unrelated default was touched.
    #[test]
    fn the_offset_mode_alone_does_not_change_a_hash() {
        let base = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        let mut fixed = base.clone();
        fixed.objects[0].clone_offset = OffsetMode::Fixed;
        assert_eq!(scene_hash(&fixed), scene_hash(&base));
    }

    /// Z-order is visible, so reordering must invalidate.
    #[test]
    fn reordering_objects_changes_the_hash() {
        let a = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0), object(20.0)],
        };
        let b = Scene {
            rasters: Vec::new(),
            objects: vec![object(20.0), object(10.0)],
        };
        assert_ne!(scene_hash(&a), scene_hash(&b));
    }

    #[test]
    fn different_tiles_and_qualities_are_different_entries() {
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        let base = key(&scene);

        let other_tile = TileKey {
            tile: TileId::new(2, 2, 1).expect("valid"),
            ..base
        };
        assert_ne!(base.digest(), other_tile.digest());

        let other_quality = TileKey {
            quality: Quality::Draft,
            ..base
        };
        assert_ne!(base.digest(), other_quality.digest());
    }

    /// The probe answers exactly what `get` would, and answering does not
    /// count as use — the entry's place in the eviction order is unchanged.
    #[test]
    fn contains_reports_without_touching() {
        let temp = TempCache::new("contains", 1 << 20);
        let cache = &temp.cache;
        let key = key(&Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        });
        assert!(!cache.contains(&key));
        cache.put(&key, &[1, 2, 3]).expect("put");
        assert!(cache.contains(&key));

        let before = cache.index.lock().expect("lock").entries[&key.digest()].1;
        assert!(cache.contains(&key));
        let after = cache.index.lock().expect("lock").entries[&key.digest()].1;
        assert_eq!(before, after, "a probe must not mark the entry used");
    }

    #[test]
    fn a_stored_tile_comes_back() {
        let temp = TempCache::new("roundtrip", DEFAULT_CAPACITY_BYTES);
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        let k = key(&scene);

        assert!(temp.cache.get(&k).is_none(), "empty to start");
        temp.cache.put(&k, &[1, 2, 3, 4]).expect("stores");
        assert_eq!(temp.cache.get(&k).as_deref(), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(temp.cache.stats().entries, 1);
        assert_eq!(temp.cache.stats().bytes, 4);
    }

    #[test]
    fn an_edited_scene_misses() {
        let temp = TempCache::new("miss", DEFAULT_CAPACITY_BYTES);
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        temp.cache.put(&key(&scene), &[9; 16]).expect("stores");

        let edited = Scene {
            rasters: Vec::new(),
            objects: vec![object(12.0)],
        };
        assert!(temp.cache.get(&key(&edited)).is_none(), "an edit must miss");
        // And the original is still there: only affected frames are lost.
        assert!(temp.cache.get(&key(&scene)).is_some());
    }

    #[test]
    fn the_least_recently_used_entry_is_evicted_first() {
        let temp = TempCache::new("evict", 300);
        let tile = |x| TileKey {
            scene: [0; 32],
            tile: TileId::new(3, x, 0).expect("valid"),
            quality: Quality::Standard,
        };

        temp.cache.put(&tile(0), &[0; 100]).expect("stores");
        temp.cache.put(&tile(1), &[1; 100]).expect("stores");
        temp.cache.put(&tile(2), &[2; 100]).expect("stores");

        // Touch the oldest so it is no longer the least recently used.
        assert!(temp.cache.get(&tile(0)).is_some());

        temp.cache.put(&tile(3), &[3; 100]).expect("stores");
        assert!(temp.cache.stats().bytes <= 300, "must stay under the cap");
        assert!(
            temp.cache.get(&tile(1)).is_none(),
            "the untouched one goes first"
        );
        assert!(
            temp.cache.get(&tile(0)).is_some(),
            "the touched one survives"
        );
        assert!(temp.cache.get(&tile(3)).is_some(), "the newest survives");
    }

    #[test]
    fn a_restart_adopts_what_is_already_there() {
        let temp = TempCache::new("restart", DEFAULT_CAPACITY_BYTES);
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        temp.cache.put(&key(&scene), &[7; 64]).expect("stores");

        let reopened = RenderCache::open(&temp.root, DEFAULT_CAPACITY_BYTES).expect("reopens");
        assert_eq!(reopened.stats().entries, 1);
        assert_eq!(reopened.get(&key(&scene)).as_deref(), Some(&[7u8; 64][..]));
    }

    /// Deleting the cache must always be safe and lossless (invariant 1).
    #[test]
    fn clearing_leaves_nothing_behind() {
        let temp = TempCache::new("clear", DEFAULT_CAPACITY_BYTES);
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        temp.cache.put(&key(&scene), &[1; 32]).expect("stores");

        temp.cache.clear().expect("clears");
        assert_eq!(temp.cache.stats().entries, 0);
        assert_eq!(temp.cache.stats().bytes, 0);
        assert!(temp.cache.get(&key(&scene)).is_none());
    }

    /// A file removed underneath the cache must be a miss, not a broken read.
    #[test]
    fn a_vanished_file_is_forgotten() {
        let temp = TempCache::new("vanish", DEFAULT_CAPACITY_BYTES);
        let scene = Scene {
            rasters: Vec::new(),
            objects: vec![object(10.0)],
        };
        let k = key(&scene);
        temp.cache.put(&k, &[5; 8]).expect("stores");

        std::fs::remove_file(temp.root.join(k.digest())).expect("removes");
        assert!(temp.cache.get(&k).is_none());
        assert_eq!(
            temp.cache.stats().entries,
            0,
            "the index must forget it too"
        );
    }

    #[test]
    fn an_empty_scene_has_a_stable_hash() {
        assert_eq!(scene_hash(&Scene::default()), scene_hash(&Scene::default()));
        assert_ne!(
            scene_hash(&Scene::default()),
            scene_hash(&Scene {
                rasters: Vec::new(),
                objects: vec![object(1.0)]
            })
        );
    }

    /// Two callers for one tile evaluate it once. The second arrives while
    /// the first is rendering, waits, and is served what the first stored.
    #[test]
    fn concurrent_requests_for_one_tile_render_it_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = Arc::new(TempCache::new("single-flight", 1 << 20));
        let key = TileKey {
            scene: [7; 32],
            tile: TileId::new(2, 1, 1).expect("valid"),
            quality: Quality::Exact,
        };
        let renders = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(std::sync::Barrier::new(2));

        let workers: Vec<_> = (0..2)
            .map(|_| {
                let temp = Arc::clone(&temp);
                let renders = Arc::clone(&renders);
                let started = Arc::clone(&started);
                std::thread::spawn(move || {
                    started.wait();
                    temp.cache
                        .get_or_render(&key, || {
                            renders.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            Ok(vec![1, 2, 3])
                        })
                        .expect("render")
                })
            })
            .collect();
        for worker in workers {
            assert_eq!(worker.join().expect("thread"), vec![1, 2, 3]);
        }
        assert_eq!(renders.load(Ordering::SeqCst), 1, "rendered twice");
        assert!(temp.cache.contains(&key));
    }

    /// A failed render releases the key: the next request renders again.
    #[test]
    fn a_failed_render_does_not_wedge_the_key() {
        let temp = TempCache::new("failed-flight", 1 << 20);
        let key = TileKey {
            scene: [9; 32],
            tile: TileId::new(2, 1, 1).expect("valid"),
            quality: Quality::Exact,
        };
        assert!(
            temp.cache
                .get_or_render(&key, || Err(RenderError::Flatten("boom".to_owned())))
                .is_err()
        );
        let bytes = temp
            .cache
            .get_or_render(&key, || Ok(vec![4]))
            .expect("second try");
        assert_eq!(bytes, vec![4]);
    }
}
