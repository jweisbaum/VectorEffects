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
    for product in Product::ALL
        .into_iter()
        .filter(|p| ![Product::Ccmp, Product::Seawinds].contains(p) && !p.variable().is_scalar())
    {
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
            Variable::SeaSurfaceTemperature => unreachable!("filtered out above"),
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

/// CCMP from NOAA's ERDDAP: the axes open, the newest six-hourly time is
/// within a few days, and its field is a wind that stops short of the poles.
#[test]
#[ignore = "reaches the network; set VE_TEST_NRT=1"]
fn ccmp_opens_and_its_newest_field_is_plausible() {
    if std::env::var("VE_TEST_NRT").is_err() {
        println!("VE_TEST_NRT is not set; nothing fetched");
        return;
    }
    let began = std::time::Instant::now();
    let source = Product::Ccmp.open().expect("CCMP opens");
    let (first, last) = source.coverage().expect("coverage");
    println!(
        "ccmp: {} to {}, opened in {:.1} s",
        first.to_iso(),
        last.to_iso(),
        began.elapsed().as_secs_f64()
    );
    let step = source.step_at(last).expect("the newest step");
    let field = &source.read_step(&step).expect("the newest field")[0];
    let speeds: Vec<f32> = field
        .u
        .iter()
        .zip(&field.v)
        .filter(|(u, v)| u.is_finite() && v.is_finite())
        .map(|(u, v)| u.hypot(*v))
        .collect();
    let mean = speeds.iter().sum::<f32>() / speeds.len() as f32;
    println!("  {} defined, mean {mean:.2} m/s", speeds.len());
    // 78.5 S to 78.5 N of 1440 x 721: about 87 % of the nodes.
    let fraction = speeds.len() as f64 / field.u.len() as f64;
    assert!((0.8..0.92).contains(&fraction), "{fraction}");
    assert!((2.0..15.0).contains(&mean), "{mean}");
    assert!(field.u[0].is_nan(), "nothing at the north pole");
}

/// Blended Seawinds from NOAA CoastWatch: the directory lists recent days,
/// and the newest day's last field is a wind over nearly the whole globe —
/// the product blends every satellite and fills the gaps between swaths.
#[test]
#[ignore = "reaches the network; set VE_TEST_NRT=1"]
fn seawinds_opens_and_its_newest_field_is_plausible() {
    if std::env::var("VE_TEST_NRT").is_err() {
        println!("VE_TEST_NRT is not set; nothing fetched");
        return;
    }
    let began = std::time::Instant::now();
    let source = Product::Seawinds.open().expect("Blended Seawinds opens");
    let (first, last) = source.coverage().expect("coverage");
    println!(
        "seawinds: {} to {}, opened in {:.1} s",
        first.to_iso(),
        last.to_iso(),
        began.elapsed().as_secs_f64()
    );
    let began = std::time::Instant::now();
    let step = source.step_at(last).expect("the newest step");
    let field = &source.read_step(&step).expect("the newest field")[0];
    let speeds: Vec<f32> = field
        .u
        .iter()
        .zip(&field.v)
        .filter(|(u, v)| u.is_finite() && v.is_finite())
        .map(|(u, v)| u.hypot(*v))
        .collect();
    let mean = speeds.iter().sum::<f32>() / speeds.len() as f32;
    println!(
        "  {} defined, mean {mean:.2} m/s, read in {:.1} s",
        speeds.len(),
        began.elapsed().as_secs_f64()
    );
    // Ocean only, and not under sea ice: well over half the nodes.
    let fraction = speeds.len() as f64 / field.u.len() as f64;
    assert!((0.5..0.8).contains(&fraction), "{fraction}");
    assert!((3.0..15.0).contains(&mean), "{mean}");
}

/// The three SST products: each opens, its newest day is within a few days,
/// and its field is a sea temperature — defined over most of the ocean,
/// between the freezing point of sea water and the warmest tropical sea.
#[test]
#[ignore = "reaches the network; set VE_TEST_NRT=1"]
fn every_sst_product_opens_and_its_newest_day_is_a_sea() {
    if std::env::var("VE_TEST_NRT").is_err() {
        println!("VE_TEST_NRT is not set; nothing fetched");
        return;
    }
    for product in [Product::Oisst, Product::GeoPolar, Product::Ostia] {
        let began = std::time::Instant::now();
        let source = product.open().expect("opens");
        let (first, last) = source.coverage().expect("coverage");
        assert_eq!(last.hour, 0, "a day is filed under its midnight");
        let step = source.step_at(last).expect("the newest day");
        let field = &source.read_step(&step).expect("the newest field")[0];
        assert_eq!(field.variable, Variable::SeaSurfaceTemperature);
        assert!(field.v.is_empty(), "a scalar has no second component");
        let defined: Vec<f32> = field.u.iter().copied().filter(|t| t.is_finite()).collect();
        let fraction = defined.len() as f64 / field.u.len() as f64;
        let mean = defined.iter().sum::<f32>() / defined.len() as f32;
        let (coldest, warmest) = defined
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(*t), hi.max(*t)));
        println!(
            "{}: {} to {}, {:.0} % defined, mean {mean:.1} °C, {coldest:.1} to {warmest:.1}, in {:.1} s",
            product.id(),
            first.to_iso(),
            last.to_iso(),
            fraction * 100.0,
            began.elapsed().as_secs_f64()
        );
        assert!((0.5..0.85).contains(&fraction), "{fraction}");
        assert!((10.0..22.0).contains(&mean), "{mean}");
        assert!(coldest > -3.0 && warmest < 36.0, "{coldest}..{warmest}");
    }
}

/// CMC from NASA: the catalogue lists recent days without a login, and a
/// token PO.DAAC does not accept is reported as such — the archive answers
/// it with its login page, which is not followed. With a real token in
/// `VE_TEST_EARTHDATA_TOKEN`, the newest day is read and is a sea.
#[test]
#[ignore = "reaches the network; set VE_TEST_NRT=1"]
fn cmc_lists_its_days_and_names_a_refused_token() {
    if std::env::var("VE_TEST_NRT").is_err() {
        println!("VE_TEST_NRT is not set; nothing fetched");
        return;
    }
    let refused = match Product::Cmc.open_with(Some("not-a-real-token")) {
        Ok(source) => {
            let (first, last) = source.coverage().expect("coverage");
            println!("cmc: {} to {}", first.to_iso(), last.to_iso());
            let step = source.step_at(last).expect("the newest step");
            match source.read_step(&step) {
                Ok(_) => panic!("a made-up token was accepted"),
                Err(err) => err.to_string(),
            }
        }
        Err(err) => panic!("the catalogue did not open: {err}"),
    };
    println!("  a made-up token: {refused}");
    assert!(refused.contains("did not accept the token"), "{refused}");
    assert!(!refused.contains("not-a-real-token"), "{refused}");

    let Ok(token) = std::env::var("VE_TEST_EARTHDATA_TOKEN") else {
        println!("  VE_TEST_EARTHDATA_TOKEN is not set; no day read");
        return;
    };
    let source = Product::Cmc.open_with(Some(&token)).expect("opens");
    let (_, last) = source.coverage().expect("coverage");
    let began = std::time::Instant::now();
    let field = &source
        .read_step(&source.step_at(last).expect("newest"))
        .expect("the newest day")[0];
    let sea: Vec<f32> = field.u.iter().copied().filter(|t| t.is_finite()).collect();
    let mean = sea.iter().sum::<f32>() / sea.len() as f32;
    println!(
        "  {} defined, mean {mean:.2} °C, read in {:.1} s",
        sea.len(),
        began.elapsed().as_secs_f64()
    );
    let fraction = sea.len() as f64 / field.u.len() as f64;
    assert!((0.55..0.8).contains(&fraction), "{fraction}");
    assert!((10.0..20.0).contains(&mean), "{mean}");
}
