//! Opt-in Hindsight benchmark. Compares separate hourly reads with combined
//! batches, and permits a second run against the same persistent cache.
//! cargo run -p ve-zarr --example hindsight_download -- r2 batch 24 /tmp/ve-hindsight-r2
//! cargo run -p ve-zarr --example hindsight_download -- r2 separate 24
use std::path::PathBuf;
use std::time::Instant;
use ve_zarr::hindsight::{HindsightStore, Provider};
use ve_zarr::{Archive, Field, FieldSource, Utc, Window};

fn check(fields: &[Field]) -> Result<(), Box<dyn std::error::Error>> {
    for field in fields {
        if field.u.len() != 25
            || field.v.len() != 25
            || !field
                .u
                .iter()
                .zip(&field.v)
                .any(|(u, v)| u.is_finite() && v.is_finite())
        {
            return Err("expected 25 ocean samples with populated vectors".into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let provider =
        match args.first().map(String::as_str) {
            Some("s3") => Provider::S3,
            Some("r2") => Provider::R2,
            Some("tigris") => Provider::Tigris,
            _ => return Err(
                "usage: hindsight_download <s3|r2|tigris> [batch|separate] [hours=24] [cache-dir]"
                    .into(),
            ),
        };
    let mode = args.get(1).map_or("batch", String::as_str);
    let hours: usize = args.get(2).map_or(Ok(24), |s| s.parse())?;
    if !(1..=240).contains(&hours) {
        return Err("hours must be 1–240".into());
    }
    let start = Utc::parse("2000-01-01T00:00").ok_or("invalid start")?;
    let end = Utc::from_hours_since_unix_epoch(start.hours_since_unix_epoch() + hours as i64 - 1);
    let window = Window {
        i0: 1278,
        ni: 5,
        j0: 198,
        nj: 5,
    };
    let began = Instant::now();
    let mut open_s = 0.0;
    let mut checksum = 0.0_f64;
    match mode {
        "batch" | "coverage" => {
            let temporary = tempfile::tempdir()?;
            let cache = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| temporary.path().to_owned());
            let opened = Instant::now();
            let mut source = HindsightStore::open_cached(provider, &Archive::ALL, &cache)?;
            open_s += opened.elapsed().as_secs_f64();
            if mode == "coverage" {
                println!(
                    "{provider:?} coverage={:?} elapsed_s={open_s:.3}",
                    source
                        .coverage()
                        .map(|(first, last)| (first.to_iso(), last.to_iso()))
                );
                return Ok(());
            }
            source.set_window(window);
            let steps = source.steps_in_range(start, end)?;
            for batch in source.batches(&steps)? {
                let mut first_update = None;
                let mut updates = 0;
                let fields = std::thread::scope(|scope| {
                    let worker = scope.spawn(|| source.read_batch(&steps[batch]));
                    let mut last = source.progress();
                    while !worker.is_finished() {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        let progress = source.progress();
                        if progress != last {
                            first_update.get_or_insert(began.elapsed().as_secs_f64());
                            updates += 1;
                            last = progress;
                        }
                    }
                    worker
                        .join()
                        .unwrap_or_else(|p| std::panic::resume_unwind(p))
                })?;
                println!(
                    "first_update_s={first_update:?} updates={updates} bytes={}",
                    source.progress().downloaded_bytes
                );
                for fields in fields {
                    check(&fields)?;
                    checksum += fields
                        .iter()
                        .map(|f| f64::from(f.u[12]) + f64::from(f.v[12]))
                        .sum::<f64>();
                }
            }
        }
        "separate" => {
            for archive in Archive::ALL {
                let opened = Instant::now();
                let mut source = HindsightStore::open(provider, archive)?;
                open_s += opened.elapsed().as_secs_f64();
                source.set_window(window);
                for step in source.steps_in_range(start, end)? {
                    let fields = source.read_step(&step)?;
                    check(&fields)?;
                    checksum += fields
                        .iter()
                        .map(|f| f64::from(f.u[12]) + f64::from(f.v[12]))
                        .sum::<f64>();
                }
            }
        }
        _ => return Err("mode must be batch, separate or coverage".into()),
    }
    let total = began.elapsed().as_secs_f64();
    println!(
        "{provider:?} mode={mode} hours={hours} open_s={open_s:.3} read_s={:.3} total_s={total:.3} checksum={checksum:.6}",
        total - open_s
    );
    Ok(())
}
