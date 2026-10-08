//! Combined Whirlwind import. The Open Data and near-real-time pipelines
//! retain their existing readers, worker counts, filenames and encoding.

use super::*;
use std::io::{BufWriter, Write};
use ve_zarr::FieldSource;
use ve_zarr::hindsight::HindsightStore;

#[allow(
    clippy::too_many_arguments,
    reason = "one import's selected source, fields, extent, paths and progress"
)]
pub(super) fn fetch(
    provider: HistoricalDataSource,
    archives: &[Archive],
    request: &HistoryRequest,
    wanted: &[i64],
    directory: &Path,
    cache_dir: &Path,
    extent: &Extent,
    on_progress: &mut impl FnMut(HistoryProgress),
) -> Result<Vec<(Archive, PathBuf)>> {
    let mirror = provider
        .hindsight_provider()
        .ok_or_else(|| AppError::Internal("expected Hindsight source".into()))?;
    on_progress(HistoryProgress {
        archive: "Whirlwind Hindsight".into(),
        done: 0,
        total: (wanted.len() * archives.len()) as u32,
        work: Some(HistoryWork {
            phase: "preparing".into(),
            fraction: 0.0,
            downloaded_bytes: 0,
        }),
    });
    let mut source = HindsightStore::open_cached(mirror, archives, cache_dir)
        .doing("reach the archive", "Whirlwind Hindsight")?;
    validate_coverage(&provider.label(archives[0]), source.coverage(), request)?;
    source.set_window(extent.window);
    write_source(
        provider,
        archives,
        &source,
        request,
        wanted,
        directory,
        extent,
        on_progress,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the testable body of the combined import"
)]
fn write_source(
    provider: HistoricalDataSource,
    archives: &[Archive],
    source: &HindsightStore,
    request: &HistoryRequest,
    wanted: &[i64],
    directory: &Path,
    extent: &Extent,
    on_progress: &mut impl FnMut(HistoryProgress),
) -> Result<Vec<(Archive, PathBuf)>> {
    let started = std::time::Instant::now();
    let held: std::collections::BTreeMap<_, _> = source
        .steps_in_range(at_hour(wanted[0]), at_hour(wanted[wanted.len() - 1]))
        .doing("read available hours", "Whirlwind Hindsight")?
        .into_iter()
        .map(|step| (step.valid_time.hours_since_unix_epoch(), step))
        .collect();
    let steps: Vec<_> = wanted
        .iter()
        .filter_map(|t| held.get(&t.div_euclid(HOUR)).copied())
        .collect();
    if steps.is_empty() {
        return Err(bad_range("Whirlwind Hindsight holds no requested hour"));
    }
    let batches = source
        .batches(&steps)
        .doing("plan the download", "Whirlwind Hindsight")?;
    let range = (request.start_unix_s, request.end_unix_s);
    let paths: Vec<_> = archives
        .iter()
        .map(|archive| {
            (
                *archive,
                directory.join(extent.file_name(&provider.file_id(*archive), range)),
            )
        })
        .collect();
    // Stage the whole import, preserving any previous download on failure.
    let mut files = paths
        .iter()
        .map(|_| {
            tempfile::NamedTempFile::new_in(directory)
                .map(BufWriter::new)
                .map_err(AppError::from)
        })
        .collect::<Result<Vec<_>>>()?;
    let reference = reference_time(wanted[0].div_euclid(HOUR) * HOUR)?;
    let anchor = wanted[0].div_euclid(HOUR);
    let mut done = 0;
    for batch in batches {
        let batch_started = std::time::Instant::now();
        let batch_base = 0.02 + 0.95 * batch.start as f32 / steps.len() as f32;
        let batch_share = 0.95 * batch.len() as f32 / steps.len() as f32;
        let mut report = |phase: &str, fraction, downloaded_bytes| {
            on_progress(HistoryProgress {
                archive: "Whirlwind Hindsight".into(),
                done,
                total: (steps.len() * archives.len()) as u32,
                work: Some(HistoryWork {
                    phase: phase.into(),
                    fraction,
                    downloaded_bytes,
                }),
            });
        };
        report("indexing", batch_base, source.progress().downloaded_bytes);
        let fields = read_with_progress(source, &steps[batch.clone()], |p| {
            report(
                p.phase,
                batch_base + batch_share * 0.9 * p.fraction,
                p.downloaded_bytes,
            );
        })
        .doing("read fields", "Whirlwind Hindsight")?;
        let read_finished = std::time::Instant::now();
        let batch_done = done;
        for (step, fields) in steps[batch.clone()].iter().zip(fields) {
            let forecast = u32::try_from(step.valid_time.hours_since_unix_epoch() - anchor)
                .map_err(|_| bad_range("Hindsight hour precedes the requested range"))?;
            for ((archive, output), field) in archives.iter().zip(&mut files).zip(fields) {
                let field = extent.fit(field, "Whirlwind Hindsight")?;
                if !field.u.iter().all(|v| v.is_nan()) {
                    output.write_all(&encode_hour(extent.grid, reference, forecast, &[field])?)?;
                }
                done += 1;
                on_progress(HistoryProgress {
                    archive: provider.label(*archive),
                    done,
                    total: (steps.len() * archives.len()) as u32,
                    work: Some(HistoryWork {
                        phase: "writing".into(),
                        fraction: batch_base
                            + batch_share
                                * (0.9
                                    + 0.1 * (done - batch_done) as f32
                                        / (batch.len() * archives.len()) as f32),
                        downloaded_bytes: source.progress().downloaded_bytes,
                    }),
                });
            }
            tracing::info!(
                step = done / archives.len() as u32,
                of = steps.len(),
                elapsed_s = started.elapsed().as_secs_f64(),
                "Hindsight hour written"
            );
        }
        tracing::info!(
            batch_steps = batch.len(),
            read_s = read_finished.duration_since(batch_started).as_secs_f64(),
            write_s = read_finished.elapsed().as_secs_f64(),
            bytes = source.progress().downloaded_bytes,
            "Hindsight batch finished"
        );
    }
    for (file, (_, path)) in files.iter_mut().zip(&paths) {
        file.flush()
            .doing("finish the history file", path.display())?;
    }
    for (file, (_, path)) in files.into_iter().zip(&paths) {
        file.into_inner()
            .map_err(|e| AppError::from(e.into_error()))?
            .persist(path)
            .map_err(|e| e.error)
            .doing("save the history file", path.display())?;
    }
    Ok(paths)
}

/// Keep the event callback on the import thread. A stalled request emits no
/// invented work; its partial byte count is visible while the body arrives.
fn read_with_progress(
    source: &HindsightStore,
    steps: &[ve_zarr::Step],
    mut report: impl FnMut(ve_zarr::hindsight::ReadProgress),
) -> ve_zarr::Result<Vec<Vec<Field>>> {
    std::thread::scope(|scope| {
        let mut last = source.progress();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let worker = scope.spawn(move || {
            let result = source.read_batch(steps);
            let _ = send.send(());
            result
        });
        while matches!(
            receive.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            let progress = source.progress();
            if progress != last {
                report(progress);
                last = progress;
            }
        }
        worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ve_zarr::export::{Layout, Writer, f16};

    #[test]
    fn combined_import_keeps_files_fields_hours_regions_and_provenance_separate() {
        let fixture = tempfile::tempdir().unwrap();
        let layout = Layout::new(1_000_000, 3, 6).unwrap();
        let mut writer = Writer::create(
            fixture.path(),
            layout,
            "test",
            "test",
            "2012-01-01T00:00:00",
        )
        .unwrap();
        let mut values = vec![f16::NAN; layout.chunk_len()];
        for time in [0, 2] {
            for parameter in 0..4 {
                for cell in 0..100 {
                    values[(time * 4 + parameter) * 100 + cell] =
                        f16::from_f32((1 + time + parameter) as f32);
                }
            }
        }
        writer.write_chunk(0, 1, 0, &values).unwrap();
        writer.finish_time().unwrap();
        std::fs::create_dir_all(fixture.path().join(".historysyncer/complete")).unwrap();
        std::fs::write(
            fixture.path().join(".historysyncer/complete/0.json"),
            r#"{"chunk":0}"#,
        )
        .unwrap();
        let storage = Arc::new(zarrs::filesystem::FilesystemStore::new(fixture.path()).unwrap());
        let archives = [Archive::GlobCurrent, Archive::Era5Wind];
        let mut source = HindsightStore::from_storage_many(storage.clone(), &archives).unwrap();
        let settings = ProjectSettings {
            region: Some(
                Region::snapped(-179.0, -178.0, 74.0, 75.0, false, Resolution::Deg025).unwrap(),
            ),
            ..ProjectSettings::new(
                ve_core::FieldKind::Wind,
                Resolution::Deg025,
                ve_core::project::StepHours::H6,
                3,
            )
        };
        let extent = Extent::of(&settings);
        source.set_window(extent.window);
        let start = Utc::parse("2012-01-01T00:00")
            .unwrap()
            .hours_since_unix_epoch()
            * HOUR;
        let wanted = [start, start + 6 * HOUR, start + 12 * HOUR];
        let request = HistoryRequest {
            archives: archives.iter().map(|a| a.id().into()).collect(),
            start_unix_s: start,
            end_unix_s: wanted[2],
            set_start_time: false,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut progress = Vec::new();
        let paths = write_source(
            HistoricalDataSource::WhirlwindSource2,
            &archives,
            &source,
            &request,
            &wanted,
            directory.path(),
            &extent,
            &mut |p| progress.push(p),
        )
        .unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(progress.last().unwrap().done, 6);
        assert!(progress.windows(2).all(|w| w[0].done <= w[1].done));
        assert!(
            progress
                .windows(2)
                .all(|w| w[0].work.as_ref().unwrap().fraction
                    <= w[1].work.as_ref().unwrap().fraction)
        );
        assert_eq!(
            progress[0].done, 0,
            "report before the first batch finishes"
        );
        let comparison = tempfile::tempdir().unwrap();
        for ((archive, path), kind) in paths
            .iter()
            .zip([ve_core::FieldKind::Current, ve_core::FieldKind::Wind])
        {
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&HistoricalDataSource::WhirlwindSource2.file_id(*archive))
            );
            let imported = ve_grib::import::read_file(path, None).unwrap();
            assert_eq!(imported.sequences.len(), 1);
            let sequence = &imported.sequences[0];
            assert_eq!(sequence.kind, kind);
            assert_eq!(sequence.frames.len(), 2, "empty middle hour stays absent");
            assert_eq!(sequence.frames[0].valid_unix_s, start);
            assert_eq!(sequence.frames[1].valid_unix_s, wanted[2]);
            let mut single =
                HindsightStore::from_storage_many(storage.clone(), &[*archive]).unwrap();
            single.set_window(extent.window);
            let single: Arc<dyn FieldSource> = Arc::new(single);
            let id = HistoricalDataSource::WhirlwindSource2.file_id(*archive);
            let original = fetch_source_to_file(
                &Origin {
                    id: &id,
                    label: "fixture",
                },
                &single,
                (start, wanted[2]),
                &wanted,
                false,
                comparison.path(),
                &extent,
                |_| {},
            )
            .unwrap();
            assert_eq!(
                std::fs::read(path).unwrap(),
                std::fs::read(original).unwrap(),
                "batching must preserve the original pipeline's GRIB bytes"
            );
        }
    }
}
