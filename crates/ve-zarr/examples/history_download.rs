//! Opt-in timing of the real archive reader, including decoding and regridding.
//! Run with `cargo run -p ve-zarr --release --example history_download --
//! era5-wind 4 4` (archive, hours, workers). Nothing is saved to disk.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use ve_zarr::{Archive, Utc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let archive = args
        .first()
        .and_then(|id| Archive::parse(id))
        .ok_or("usage: history_download <era5-wind|globcurrent> [hours=4] [workers=4]")?;
    let hours: usize = args.get(1).map_or(Ok(4), |s| s.parse())?;
    let workers: usize = args.get(2).map_or(Ok(4), |s| s.parse())?;
    if !(1..=24).contains(&hours) || !(1..=8).contains(&workers) {
        return Err("hours must be 1–24 and workers 1–8".into());
    }
    let started = Instant::now();
    let source = archive.open()?;
    let opened = started.elapsed();
    eprintln!("{} open_s={:.3}", archive.id(), opened.as_secs_f64());
    // A fixed, final historical range makes runs comparable.
    let first = Utc {
        year: 2024,
        month: 1,
        day: 1,
        hour: 0,
    }
    .hours_since_unix_epoch();
    let steps: Vec<_> = (0..hours)
        .map(|i| {
            source
                .step_at(Utc::from_hours_since_unix_epoch(first + i as i64))
                .ok_or("benchmark hour is absent")
        })
        .collect::<Result<_, _>>()?;
    let next = AtomicUsize::new(0);
    let downloaded = Instant::now();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..workers.min(hours))
            .map(|_| {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(step) = steps.get(i) else {
                            break;
                        };
                        let fields = source.read_step(step)?;
                        std::hint::black_box(fields);
                    }
                    ve_zarr::Result::Ok(())
                })
            })
            .collect();
        for job in jobs {
            job.join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        }
        ve_zarr::Result::Ok(())
    })?;
    println!(
        "{} hours={hours} workers={workers} open_s={:.3} read_s={:.3} total_s={:.3}",
        archive.id(),
        opened.as_secs_f64(),
        downloaded.elapsed().as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
