#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The near-real-time products against the live Marine Data Store.
//!
//! Fetches on purpose, so it is ignored and asks for `VE_TEST_NRT=1`:
//!
//! ```bash
//! VE_TEST_NRT=1 cargo test -p ve-zarr --release --test arco_live -- --ignored --nocapture
//! ```

use ve_zarr::{Product, Utc, Variable};

/// Each product is found through the catalogue, reaches to within a few days
/// of now, and its newest field is a field: values where there should be
/// some, and none that no wind or current has.
#[test]
#[ignore = "reaches the network; set VE_TEST_NRT=1"]
fn every_product_opens_and_its_newest_field_is_plausible() {
    if std::env::var("VE_TEST_NRT").is_err() {
        println!("VE_TEST_NRT is not set; nothing fetched");
        return;
    }
    let now_hours = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs() as i64
        / 3600;
    for product in Product::ALL {
        let began = std::time::Instant::now();
        let source = product.open().expect("the product opens");
        let (first, last) = source.coverage().expect("coverage");
        let behind = now_hours - last.hours_since_unix_epoch();
        println!(
            "{}: {} to {} ({behind} h behind), opened in {:.1} s",
            product.id(),
            first.to_iso(),
            last.to_iso(),
            began.elapsed().as_secs_f64()
        );
        assert!((0..24 * 5).contains(&behind), "{behind} h behind");

        // The newest step with anything in it: just after midnight a store
        // lists a day it has not written yet, which reads as empty.
        let period = i64::from(product.period_hours());
        let began = std::time::Instant::now();
        let (k, fields) = (0..3)
            .filter_map(|k| {
                let step = source.step_at(Utc::from_hours_since_unix_epoch(
                    last.hours_since_unix_epoch() - k * period,
                ))?;
                let fields = source.read_step(&step).expect("a recent field");
                fields[0]
                    .u
                    .iter()
                    .any(|v| v.is_finite())
                    .then_some((k, fields))
            })
            .next()
            .expect("a recent step with data");
        println!("  newest step with data: {k} back from the last listed");
        // A store more than a day past its last listed time has written it.
        if behind >= 24 + period {
            assert_eq!(k, 0, "the last listed step is empty though it is old");
        }
        assert_eq!(fields.len(), 1);
        let field = &fields[0];
        assert_eq!(field.variable, product.variable());
        assert_eq!(field.u.len(), ve_zarr::POINTS_PER_STEP);
        let speeds: Vec<f32> = field
            .u
            .iter()
            .zip(&field.v)
            .filter(|(u, v)| u.is_finite() && v.is_finite())
            .map(|(u, v)| u.hypot(*v))
            .collect();
        let fastest = speeds.iter().copied().fold(0.0_f32, f32::max);
        let mean = speeds.iter().sum::<f32>() / speeds.len() as f32;
        println!(
            "  {} defined of {}, mean {mean:.2} m/s, fastest {fastest:.2} m/s, read in {:.1} s",
            speeds.len(),
            field.u.len(),
            began.elapsed().as_secs_f64()
        );
        // The ocean is most of the planet — except to a swath instrument,
        // which sees a day's worth of bands of it.
        let fraction = speeds.len() as f64 / field.u.len() as f64;
        if product == Product::Ascat {
            assert!(
                (0.15..0.9).contains(&fraction),
                "{fraction} of the globe seen"
            );
        } else {
            assert!(fraction > 0.5, "{fraction} defined");
        }
        let (ceiling, typical) = match product.variable() {
            Variable::Wind10m => (80.0, 2.0..15.0),
            Variable::SurfaceCurrent => (5.0, 0.02..1.0),
        };
        assert!(fastest < ceiling, "{fastest} m/s");
        assert!(typical.contains(&mean), "mean {mean} m/s");
    }
}

/// What the import does: several steps of a 0.125 degree product read at
/// once, the way `fetch_source_to_file` reads them. A burst is what the
/// object store refused before the store took one request per connection
/// and a cap on how many it has in flight.
#[test]
#[ignore = "reaches the network; set VE_TEST_NRT=1"]
fn a_burst_of_steps_of_a_fine_product_arrives() {
    if std::env::var("VE_TEST_NRT").is_err() {
        println!("VE_TEST_NRT is not set; nothing fetched");
        return;
    }
    for product in [Product::Duacs, Product::WindL4, Product::Ascat] {
        let source = product.open().expect("the product opens");
        let (_, last) = source.coverage().expect("coverage");
        let last = last.hours_since_unix_epoch();
        let period = i64::from(product.period_hours());
        let steps: Vec<_> = (0..8)
            .filter_map(|k| source.step_at(Utc::from_hours_since_unix_epoch(last - k * period)))
            .collect();
        assert!(steps.len() >= 6, "{} recent steps", steps.len());
        let began = std::time::Instant::now();
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    loop {
                        let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(step) = steps.get(at) else { return };
                        // An empty read is a day listed and not written yet.
                        let fields = source.read_step(step).expect("a step of the burst");
                        if at > 0 {
                            assert!(fields[0].u.iter().any(|v| v.is_finite()), "step {at}");
                        }
                    }
                });
            }
        });
        println!(
            "{}: {} steps, four at a time, in {:.1} s",
            product.id(),
            steps.len(),
            began.elapsed().as_secs_f64()
        );
    }
}
