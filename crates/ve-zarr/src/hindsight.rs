//! The three mirrors of Whirlwind Hindsight's routing Zarr v3 archive.
//!
//! The time coordinate extends beyond the populated archive. Completion markers,
//! not that coordinate, determine which time chunks can be imported.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use zarrs::storage::ReadableStorage;
use zarrs_storage::StoreKey;

use crate::hindsight_progress::Progress;
pub use crate::hindsight_progress::ReadProgress;
use crate::http::HttpStore;
use crate::regrid::{CellGrid, native_window, place, to_era5_window};
use crate::routing::RoutingStore;
use crate::s3::Credentials;
use crate::source::{step_at_hour, steps_between};
use crate::{Archive, Field, FieldSource, Result, Step, Utc, Variable, Window, ZarrError};

/// Mirrors in the order shown in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    S3,
    R2,
    Tigris,
}

impl Provider {
    fn storage(
        self,
        cache_dir: Option<&std::path::Path>,
        progress: Arc<Progress>,
    ) -> Result<Arc<HttpStore>> {
        let (url, credentials) = match self {
            Self::S3 => (
                "https://whirlwind-hindsight.s3.us-east-1.amazonaws.com/hindsight",
                None,
            ),
            Self::R2 => (
                "https://3d5456ad10ebc32c8a3c259aa84e6c64.r2.cloudflarestorage.com/whirlwind-hindsight/hindsight",
                Some(Credentials {
                    access_key: "18bf3daf0129f869e08697abf38a6d0e",
                    secret_key: "a4159f3410e7047c23930847dcd6fe678cc5ff5dbeb9b44e0d5e881c03384bac",
                    region: "auto",
                }),
            ),
            Self::Tigris => (
                "https://fly.storage.tigris.dev/whirlwind-hindsight/hindsight",
                Some(Credentials {
                    access_key: "tid_dDVavduKOeciqUWKMFVJuSeNrNtSyxBlSweeILUAKJxFHQdOqH",
                    secret_key: "tsec_RM-E+RA20+u74rIXEKqUYzbmpO5eHb4lwBWyOcYo+QN-cIQQgHkk0AqGbXqJB0H+NL7iP+",
                    region: "auto",
                }),
            ),
        };
        // The R2 account API token is not used by the S3 data API. Only the
        // supplied reader key pair is embedded in this read-only transport.
        let store = HttpStore::s3(url, credentials)?.with_progress(progress);
        Ok(Arc::new(match cache_dir {
            Some(root) => store.with_disk_cache(root),
            None => store,
        }))
    }
}

/// One vector field from the shared archive, converted to the history grid.
pub struct HindsightStore {
    routing: RoutingStore,
    storage: ReadableStorage,
    hours: Vec<i64>,
    vectors: Vec<(Variable, [usize; 2])>,
    shard_cache: zarrs::array::ArrayShardedReadableExtCache,
    grid: CellGrid,
    window: Window,
    complete: Mutex<BTreeMap<u64, bool>>,
    coverage: Option<(Utc, Utc)>,
    progress: Arc<Progress>,
}

impl std::fmt::Debug for HindsightStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HindsightStore")
            .field("vectors", &self.vectors)
            .field("grid", &self.grid)
            .finish_non_exhaustive()
    }
}

impl HindsightStore {
    pub fn open(provider: Provider, archive: Archive) -> Result<Self> {
        Self::open_remote(provider, &[archive], None)
    }

    pub fn open_cached(
        provider: Provider,
        archives: &[Archive],
        cache_dir: &std::path::Path,
    ) -> Result<Self> {
        Self::open_remote(provider, archives, Some(cache_dir))
    }

    fn open_remote(
        provider: Provider,
        archives: &[Archive],
        cache_dir: Option<&std::path::Path>,
    ) -> Result<Self> {
        let progress = Arc::new(Progress::default());
        let storage = provider.storage(cache_dir, progress.clone())?;
        let chunks = storage.completed_chunks()?;
        let mut source = Self::from_storage_many(storage, archives)?;
        source.progress = progress;
        source.set_coverage(&chunks)?;
        Ok(source)
    }

    fn set_coverage(&mut self, chunks: &std::collections::BTreeSet<u64>) -> Result<()> {
        let bounds: Vec<_> = chunks
            .iter()
            .map(|chunk| self.routing.time_chunk_bounds(*chunk))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        let first = bounds.iter().map(|r| r.start).min();
        let last = bounds.iter().map(|r| r.end - 1).max();
        self.coverage = match (first, last) {
            (Some(first), Some(last)) => {
                if !self.is_complete(first)? || !self.is_complete(last)? {
                    return Err(ZarrError::Open(
                        "Hindsight completion records changed; check availability again".into(),
                    ));
                }
                Some((
                    Utc::from_hours_since_unix_epoch(self.hours[first]),
                    Utc::from_hours_since_unix_epoch(self.hours[last]),
                ))
            }
            _ => None,
        };
        Ok(())
    }

    /// An inexpensive snapshot for a caller polling while `read_batch` runs.
    pub fn progress(&self) -> ReadProgress {
        self.progress.snapshot()
    }

    #[cfg(test)]
    fn from_storage(storage: ReadableStorage, archive: Archive) -> Result<Self> {
        Self::from_storage_many(storage, &[archive])
    }

    /// Open selected vectors on an existing routing-store transport.
    pub fn from_storage_many(storage: ReadableStorage, archives: &[Archive]) -> Result<Self> {
        let routing = RoutingStore::from_storage(storage.clone())?;
        let vectors = archives
            .iter()
            .map(|archive| {
                let (variable, names) = match archive {
                    Archive::Era5Wind => (Variable::Wind10m, ["u10", "v10"]),
                    Archive::GlobCurrent => (Variable::SurfaceCurrent, ["ucur", "vcur"]),
                };
                let parameter = |name| {
                    routing
                        .parameters
                        .iter()
                        .position(|p| p == name)
                        .ok_or_else(|| {
                            ZarrError::Layout(format!("Hindsight has no {name} component"))
                        })
                };
                Ok((variable, [parameter(names[0])?, parameter(names[1])?]))
            })
            .collect::<Result<Vec<_>>>()?;
        if vectors.is_empty() {
            return Err(ZarrError::Layout("no Hindsight fields requested".into()));
        }
        let grid = CellGrid {
            lat0: routing.latitude[0],
            dlat: (routing.latitude[routing.latitude.len() - 1] - routing.latitude[0])
                / (routing.latitude.len() - 1) as f64,
            nlat: routing.latitude.len(),
            lon0: routing.longitude[0],
            dlon: (routing.longitude[routing.longitude.len() - 1] - routing.longitude[0])
                / (routing.longitude.len() - 1) as f64,
            nlon: routing.longitude.len(),
        };
        if (grid.dlon * grid.nlon as f64 - 360.0).abs() > 1e-6
            || grid.lat0 < 90.0 + grid.dlat
            || routing.latitude[grid.nlat - 1] > -90.0 - grid.dlat
        {
            return Err(ZarrError::Layout(
                "Hindsight must cover the global latitude/longitude grid".into(),
            ));
        }
        Ok(Self {
            hours: routing.times.iter().map(|t| t / 3600).collect(),
            shard_cache: routing.hindsight_shard_cache(),
            routing,
            storage,
            vectors,
            grid,
            window: Window::global(),
            complete: Mutex::new(BTreeMap::new()),
            coverage: None,
            progress: Arc::new(Progress::default()),
        })
    }

    /// Group requested hours within one archive time chunk and a 256 MiB
    /// working-set estimate. Regional requests normally decode all 72 hours
    /// together; global requests split sooner to keep memory bounded.
    pub fn batches(&self, steps: &[Step]) -> Result<Vec<std::ops::Range<usize>>> {
        let native = native_window(self.grid, &self.window);
        let cells = native.rows.len() * native.columns.iter().map(|c| c.len()).sum::<usize>();
        let first_parameter = self
            .vectors
            .iter()
            .flat_map(|(_, p)| p)
            .copied()
            .min()
            .unwrap_or(0);
        let last_parameter = self
            .vectors
            .iter()
            .flat_map(|(_, p)| p)
            .copied()
            .max()
            .unwrap_or(0);
        let per_source_time = cells
            .saturating_mul(last_parameter - first_parameter + 1)
            .saturating_mul(8);
        let per_output_time = self
            .window
            .len()
            .saturating_mul(self.vectors.len())
            .saturating_mul(8);
        let scratch = self.grid.len().saturating_mul(8);
        let mut out = Vec::new();
        let mut start = 0;
        while start < steps.len() {
            let chunk = self.routing.time_chunk(steps[start].index as usize)?;
            let mut end = start + 1;
            while end < steps.len() {
                if steps[end].index <= steps[end - 1].index {
                    return Err(ZarrError::TimeRange(
                        "Hindsight hours must be strictly increasing".into(),
                    ));
                }
                let span = (steps[end].index - steps[start].index + 1) as usize;
                let memory = span
                    .saturating_mul(per_source_time)
                    .saturating_add((end - start + 1).saturating_mul(per_output_time))
                    .saturating_add(scratch);
                if self.routing.time_chunk(steps[end].index as usize)? != chunk
                    || memory > 256 * 1024 * 1024
                {
                    break;
                }
                end += 1;
            }
            out.push(start..end);
            start = end;
        }
        Ok(out)
    }

    /// Read both selected vectors and several requested hours in one decode.
    /// Call with one of `batches`' bounded groups. Gaps in the requested hours
    /// are not emitted, and the time coordinate remains authoritative.
    pub fn read_batch(&self, steps: &[Step]) -> Result<Vec<Vec<Field>>> {
        self.progress.begin("indexing", 0.0, 0.0, 0);
        let Some(first) = steps.first() else {
            return Ok(Vec::new());
        };
        let batches = self.batches(steps)?;
        if batches.len() != 1 {
            return Err(ZarrError::TimeRange(
                "Hindsight batch exceeds its time or memory bound".into(),
            ));
        }
        for step in steps {
            let time = step.index as usize;
            if self.hours.get(time).copied() != Some(step.valid_time.hours_since_unix_epoch())
                || !self.is_complete(time)?
            {
                return Err(ZarrError::TimeRange(
                    "Hindsight step is not available".into(),
                ));
            }
        }
        let first_parameter = self
            .vectors
            .iter()
            .flat_map(|(_, p)| p)
            .copied()
            .min()
            .unwrap_or(0);
        let last_parameter = self
            .vectors
            .iter()
            .flat_map(|(_, p)| p)
            .copied()
            .max()
            .unwrap_or(0);
        let parameters = first_parameter..last_parameter + 1;
        let times = first.index as usize..steps[steps.len() - 1].index as usize + 1;
        let native = native_window(self.grid, &self.window);
        let blocks = native
            .columns
            .iter()
            .enumerate()
            .map(|(index, columns)| {
                self.routing.read_hindsight_subset(
                    &self.storage,
                    &self.shard_cache,
                    [
                        times.clone(),
                        parameters.clone(),
                        native.rows.clone(),
                        columns.clone(),
                    ],
                    &self.progress,
                    index as f32 * 0.9 / native.columns.len() as f32,
                    0.9 / native.columns.len() as f32,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(steps.len());
        self.progress.begin(
            "resampling",
            0.9,
            0.1,
            (steps.len() * self.vectors.len()) as u64,
        );
        for step in steps {
            let mut fields = Vec::with_capacity(self.vectors.len());
            for (variable, pair) in &self.vectors {
                let mut components = [
                    vec![f32::NAN; self.grid.len()],
                    vec![f32::NAN; self.grid.len()],
                ];
                for (columns, values) in native.columns.iter().zip(&blocks) {
                    let cells = native.rows.len() * columns.len();
                    for (component, parameter) in components.iter_mut().zip(pair) {
                        let start = ((step.index as usize - times.start) * parameters.len()
                            + parameter
                            - parameters.start)
                            * cells;
                        place(
                            self.grid,
                            component,
                            native.rows.clone(),
                            columns.clone(),
                            &values[start..start + cells],
                        );
                    }
                }
                let [u, v] =
                    components.map(|values| to_era5_window(self.grid, &values, &self.window));
                fields.push(Field {
                    variable: *variable,
                    u,
                    v,
                });
                self.progress.advance("resampling", 1);
            }
            out.push(fields);
        }
        Ok(out)
    }

    fn is_complete(&self, time: usize) -> Result<bool> {
        let chunk = self.routing.time_chunk(time)?;
        let mut known = self.complete.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(complete) = known.get(&chunk) {
            return Ok(*complete);
        }
        let key = StoreKey::new(format!(".historysyncer/complete/{chunk}.json"))
            .map_err(|e| ZarrError::Open(e.to_string()))?;
        let marker = self
            .storage
            .get(&key)
            .map_err(|e| ZarrError::Open(e.to_string()))?;
        let complete = match marker {
            None => false,
            Some(bytes) => {
                let marker: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                    ZarrError::Layout(format!("invalid Hindsight completion marker: {e}"))
                })?;
                if marker["chunk"].as_u64() != Some(chunk) {
                    return Err(ZarrError::Layout(
                        "Hindsight completion marker names a different time chunk".into(),
                    ));
                }
                true
            }
        };
        known.insert(chunk, complete);
        Ok(complete)
    }
}

impl FieldSource for HindsightStore {
    fn name(&self) -> &'static str {
        "Whirlwind Hindsight"
    }
    fn variables(&self) -> Vec<Variable> {
        self.vectors.iter().map(|(v, _)| *v).collect()
    }
    fn coverage(&self) -> Option<(Utc, Utc)> {
        self.coverage
    }
    fn step_at(&self, time: Utc) -> Option<Step> {
        let step = step_at_hour(&self.hours, time)?;
        self.is_complete(step.index as usize).ok()?.then_some(step)
    }
    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        let mut complete = Vec::new();
        for step in steps_between(&self.hours, start, end, self.name())? {
            if self.is_complete(step.index as usize)? {
                complete.push(step);
            }
        }
        if complete.is_empty() {
            return Err(ZarrError::TimeRange(
                "Whirlwind Hindsight has not populated that time range on this source".into(),
            ));
        }
        Ok(complete)
    }
    fn set_window(&mut self, window: Window) {
        self.window = window;
    }
    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        self.read_batch(std::slice::from_ref(step))
            .map(|mut fields| fields.remove(0))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::single_range_in_vec_init,
    reason = "expected results are lists of batch ranges"
)]
mod tests {
    use super::*;

    #[test]
    fn independent_shard_indexes_overlap_instead_of_holding_the_cache_lock() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use zarrs_storage::byte_range::ByteRangeIterator;
        use zarrs_storage::{MaybeBytesIterator, ReadableStorageTraits, StorageError};
        struct Delayed {
            inner: FilesystemStore,
            first: Mutex<std::collections::BTreeSet<String>>,
            active: AtomicUsize,
            peak: AtomicUsize,
            serialize: Option<Mutex<()>>,
        }
        impl ReadableStorageTraits for Delayed {
            fn get_partial_many<'a>(
                &'a self,
                key: &StoreKey,
                ranges: ByteRangeIterator<'a>,
            ) -> std::result::Result<MaybeBytesIterator<'a>, StorageError> {
                let first = key.as_str().starts_with("data/c/")
                    && self.first.lock().unwrap().insert(key.as_str().into());
                let _serial = if first {
                    self.serialize.as_ref().map(|gate| gate.lock().unwrap())
                } else {
                    None
                };
                if first {
                    let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                    self.peak.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
                let result = self.inner.get_partial_many(key, ranges);
                if first {
                    self.active.fetch_sub(1, Ordering::SeqCst);
                }
                result
            }
            fn size_key(&self, key: &StoreKey) -> std::result::Result<Option<u64>, StorageError> {
                self.inner.size_key(key)
            }
            fn supports_get_partial(&self) -> bool {
                true
            }
        }
        let out = tempfile::tempdir().unwrap();
        let layout = Layout::new(1_000_000, 1, 1).unwrap();
        let mut writer =
            Writer::create(out.path(), layout, "test", "test", "2012-01-01T00:00:00").unwrap();
        for band in 0..4 {
            writer
                .write_chunk(0, band, 0, &vec![f16::from_f32(7.0); layout.chunk_len()])
                .unwrap();
        }
        writer.finish_time().unwrap();
        std::fs::create_dir_all(out.path().join(".historysyncer/complete")).unwrap();
        std::fs::write(
            out.path().join(".historysyncer/complete/0.json"),
            r#"{"chunk":0}"#,
        )
        .unwrap();
        for serial in [true, false] {
            let storage = Arc::new(Delayed {
                inner: FilesystemStore::new(out.path()).unwrap(),
                first: Mutex::default(),
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                serialize: serial.then(Mutex::default),
            });
            let source = HindsightStore::from_storage_many(storage.clone(), &Archive::ALL).unwrap();
            let step = source
                .step_at(Utc::parse("2012-01-01T00:00").unwrap())
                .unwrap();
            let began = std::time::Instant::now();
            assert_eq!(source.read_step(&step).unwrap().len(), 2);
            let peak = storage.peak.load(Ordering::SeqCst);
            println!(
                "serialized_indexes={serial} seconds={:.3} peak={peak}",
                began.elapsed().as_secs_f64()
            );
            if serial {
                assert_eq!(peak, 1);
            } else {
                assert!(
                    (2..=8).contains(&peak),
                    "independent index reads must overlap, bounded by eight: {peak}"
                );
            }
        }
    }

    #[test]
    fn coverage_uses_completed_chunks_and_coordinates_not_the_allocated_axis() {
        let out = tempfile::tempdir().unwrap();
        // Six-hourly coordinates: 12 samples per time chunk, with a short
        // last chunk. Unwritten chunk 2 remains a hole inside the bounds.
        let layout = Layout::new(1_000_000, 40, 6).unwrap();
        Writer::create(out.path(), layout, "test", "test", "2012-01-01T00:00:00")
            .unwrap()
            .finish_time()
            .unwrap();
        let markers = out.path().join(".historysyncer/complete");
        std::fs::create_dir_all(&markers).unwrap();
        for chunk in [1, 3] {
            std::fs::write(
                markers.join(format!("{chunk}.json")),
                format!("{{\"chunk\":{chunk}}}"),
            )
            .unwrap();
        }
        let storage = Arc::new(FilesystemStore::new(out.path()).unwrap());
        let mut source = HindsightStore::from_storage_many(storage.clone(), &Archive::ALL).unwrap();
        source.set_coverage(&[1, 3, 999].into()).unwrap();
        assert_eq!(
            source.coverage(),
            Some((
                Utc::parse("2012-01-04T00:00").unwrap(),
                Utc::parse("2012-01-10T18:00").unwrap()
            ))
        );
        assert!(
            source
                .step_at(Utc::parse("2012-01-07T00:00").unwrap())
                .is_none()
        );
        // A new query observes removal/growth, with no stale process cache.
        std::fs::remove_file(markers.join("1.json")).unwrap();
        let mut reopened = HindsightStore::from_storage_many(storage, &Archive::ALL).unwrap();
        reopened.set_coverage(&[3].into()).unwrap();
        assert_eq!(
            reopened.coverage().unwrap().0,
            Utc::parse("2012-01-10T00:00").unwrap()
        );
        reopened.set_coverage(&[].into()).unwrap();
        assert!(reopened.coverage().is_none());
    }
    use crate::export::{Layout, Writer, f16};
    use zarrs::filesystem::FilesystemStore;

    #[test]
    fn global_read_keeps_missing_chunks_in_a_populated_shard_empty() {
        let out = tempfile::tempdir().unwrap();
        let layout = Layout::new(1_000_000, 1, 1).unwrap();
        let mut writer =
            Writer::create(out.path(), layout, "test", "test", "2012-01-01T00:00:00").unwrap();
        writer
            .write_chunk(0, 1, 0, &vec![f16::from_f32(7.0); layout.chunk_len()])
            .unwrap();
        writer.finish_time().unwrap();
        std::fs::create_dir_all(out.path().join(".historysyncer/complete")).unwrap();
        std::fs::write(
            out.path().join(".historysyncer/complete/0.json"),
            r#"{"chunk":0}"#,
        )
        .unwrap();
        let source = HindsightStore::from_storage_many(
            Arc::new(FilesystemStore::new(out.path()).unwrap()),
            &Archive::ALL,
        )
        .unwrap();
        let step = source
            .step_at(Utc::parse("2012-01-01T00:00").unwrap())
            .unwrap();
        let fields = source.read_step(&step).unwrap();
        assert_eq!(fields.len(), 2);
        for field in fields {
            for values in [&field.u, &field.v] {
                assert_eq!(values.len(), crate::POINTS_PER_STEP);
                // A populated point at 75 N, 175 W, and an empty neighbour
                // at 65 N in the same shard, followed by an absent shard.
                assert_eq!(values[60 * 1440 + 740], 7.0);
                assert!(values[100 * 1440 + 740].is_nan());
                assert!(values[360 * 1440].is_nan());
            }
        }
    }

    /// Opt in to real archive reads; the normal suite stays offline.
    #[test]
    #[ignore = "downloads a sparse polar region from all three Hindsight mirrors"]
    fn live_mirrors_read_a_region_with_empty_and_populated_chunks() {
        let window = Window {
            i0: 1278,
            ni: 5,
            j0: 0,
            nj: 205,
        };
        let mut reference: Option<Vec<Field>> = None;
        for provider in [Provider::S3, Provider::R2, Provider::Tigris] {
            let cache = tempfile::tempdir().unwrap();
            let mut source =
                HindsightStore::open_cached(provider, &Archive::ALL, cache.path()).unwrap();
            source.set_window(window);
            let step = source
                .step_at(Utc::parse("2000-01-01T00:00").unwrap())
                .unwrap();
            let fields = source.read_step(&step).unwrap();
            assert_eq!(fields.len(), 2);
            for field in &fields {
                assert_eq!(field.u.len(), window.len());
                assert_eq!(field.v.len(), window.len());
            }
            let wind = &fields[0];
            assert!(wind.u.iter().any(|v| v.is_nan()));
            assert!(wind.u.iter().any(|v| v.is_finite()));
            if let Some(reference) = &reference {
                for (actual, expected) in fields.iter().zip(reference) {
                    for (a, b) in actual
                        .u
                        .iter()
                        .chain(&actual.v)
                        .zip(expected.u.iter().chain(&expected.v))
                    {
                        assert!(
                            a == b || (a.is_nan() && b.is_nan()),
                            "{provider:?} disagrees with S3"
                        );
                    }
                }
            } else {
                reference = Some(fields);
            }
            eprintln!("{provider:?}: sparse polar wind/current read passed");
        }
    }

    #[test]
    fn global_batches_split_under_the_memory_budget_but_regional_hours_stay_together() {
        let out = tempfile::tempdir().unwrap();
        let layout = Layout::new(250_000, 72, 1).unwrap();
        let mut writer =
            Writer::create(out.path(), layout, "test", "test", "2012-01-01T00:00:00").unwrap();
        writer.finish_time().unwrap();
        let mut source = HindsightStore::from_storage_many(
            Arc::new(FilesystemStore::new(out.path()).unwrap()),
            &Archive::ALL,
        )
        .unwrap();
        let steps: Vec<_> = (0..72)
            .map(|index| Step {
                index,
                valid_time: Utc::from_hours_since_unix_epoch(source.hours[index as usize]),
            })
            .collect();
        let batches = source.batches(&steps).unwrap();
        assert!(
            batches.len() > 1,
            "a global 72-hour output would exceed one GiB"
        );
        assert_eq!(
            batches.iter().cloned().flatten().collect::<Vec<_>>(),
            (0..72).collect::<Vec<_>>()
        );
        assert!(
            batches
                .iter()
                .all(|b| b.len() * 1440 * 721 * 4 * 4 < 256 * 1024 * 1024)
        );
        source.set_window(Window {
            i0: 100,
            ni: 5,
            j0: 100,
            nj: 5,
        });
        assert_eq!(source.batches(&steps).unwrap(), [0..72]);
    }

    #[test]
    fn reads_selected_vectors_reorders_the_seam_and_excludes_unpopulated_times() {
        let out = tempfile::tempdir().unwrap();
        let layout = Layout::new(1_000_000, 14, 6).unwrap();
        let mut writer =
            Writer::create(out.path(), layout, "test", "test", "2012-01-01T00:00:00").unwrap();
        for (column, value) in [(0, 20.0), (35, 10.0)] {
            let mut values = vec![f16::NAN; layout.chunk_len()];
            for t in 0..12 {
                if t == 4 {
                    continue;
                }
                for p in 0..4 {
                    for r in 0..10 {
                        for c in 0..10 {
                            values[((t * 4 + p) * 10 + r) * 10 + c] =
                                f16::from_f32(value + p as f32 * 10.0 + t as f32);
                        }
                    }
                }
            }
            writer.write_chunk(0, 1, column, &values).unwrap();
        }
        writer.finish_time().unwrap();
        let markers = out.path().join(".historysyncer/complete");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("0.json"), r#"{"chunk":0}"#).unwrap();
        let storage: ReadableStorage = Arc::new(FilesystemStore::new(out.path()).unwrap());
        let mut together =
            HindsightStore::from_storage_many(storage.clone(), &Archive::ALL).unwrap();
        together.set_window(Window {
            i0: 719,
            ni: 3,
            j0: 60,
            nj: 1,
        });
        let steps = together
            .steps_in_range(
                Utc::parse("2012-01-01T00:00").unwrap(),
                Utc::parse("2012-01-03T18:00").unwrap(),
            )
            .unwrap();
        assert_eq!(together.batches(&steps).unwrap(), [0..12]);
        let selected = [steps[0], steps[4], steps[11]];
        let batch = together.read_batch(&selected).unwrap();
        assert_eq!(batch.len(), 3, "only requested hours are returned");
        assert_eq!(batch[0][0].u, [17.5, 20.0, 20.0]);
        assert_eq!(batch[0][1].u, [37.5, 40.0, 40.0]);
        assert!(
            batch[1]
                .iter()
                .all(|f| f.u.iter().chain(&f.v).all(|v| v.is_nan()))
        );
        assert_eq!(batch[2][0].u, [28.5, 31.0, 31.0]);
        assert!(together.read_batch(&[steps[1], steps[0]]).is_err());
        let mut wrong_time = steps[0];
        wrong_time.valid_time = steps[1].valid_time;
        assert!(together.read_batch(&[wrong_time]).is_err());
        let second = Step {
            index: 12,
            valid_time: Utc::parse("2012-01-04T00:00").unwrap(),
        };
        assert_eq!(
            together.batches(&[steps[11], second]).unwrap(),
            [0..1, 1..2]
        );
        assert!(
            together.read_batch(&[second]).is_err(),
            "a missing completion marker still refuses the hour"
        );
        for (archive, offset, variable) in [
            (Archive::Era5Wind, 0.0, Variable::Wind10m),
            (Archive::GlobCurrent, 20.0, Variable::SurfaceCurrent),
        ] {
            let mut source = HindsightStore::from_storage(storage.clone(), archive).unwrap();
            assert_eq!(
                source.coverage(),
                None,
                "the time coordinate is not populated coverage"
            );
            source.set_window(Window {
                i0: 719,
                ni: 3,
                j0: 60,
                nj: 1,
            });
            let start = Utc::parse("2012-01-01T00:00").unwrap();
            let steps = source
                .steps_in_range(start, Utc::parse("2012-01-04T06:00").unwrap())
                .unwrap();
            assert_eq!(steps.len(), 12, "uncompleted second chunk is excluded");
            let field = source.read_step(&steps[0]).unwrap().remove(0);
            assert_eq!(field.variable, variable);
            assert_eq!(field.u, [17.5 + offset, 20.0 + offset, 20.0 + offset]);
            assert_eq!(field.v, [27.5 + offset, 30.0 + offset, 30.0 + offset]);
            let missing = source.read_step(&steps[4]).unwrap().remove(0);
            assert!(missing.u.iter().chain(&missing.v).all(|v| v.is_nan()));
            assert!(
                source
                    .steps_in_range(
                        Utc::parse("2012-01-04T00:00").unwrap(),
                        Utc::parse("2012-01-04T06:00").unwrap()
                    )
                    .is_err()
            );
        }
    }
}
