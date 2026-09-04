#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The decoder against real forecast files, checked by ecCodes.
//!
//! Ten samples of ten forecast files — GFS, GEFS, NCEP's two AI models,
//! ECMWF deterministic and ensemble, AIFS, ARPEGE, and GEM deterministic and
//! ensemble — covering every packing the centres ship. The files are 17 MB
//! and are not committed, so this is `#[ignore]`d and reads them from
//! `$VE_TEST_GRIBS` (default `~/temp_test_gribs`). What *is* committed is
//! `fixtures/reference_spots.txt`: ecCodes' own values at ten nodes of every
//! message, which is the independent reference this asserts against.
//!
//! ```text
//! VE_TEST_GRIBS=~/temp_test_gribs cargo test -p ve-grib --release \
//!     --test reference_set -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use ve_grib::decode;

const SPOTS: &str = include_str!("fixtures/reference_spots.txt");

/// One line of the reference table.
struct Spot {
    message: usize,
    node: usize,
    value: f32,
}

fn reference() -> BTreeMap<String, Vec<Spot>> {
    let mut out: BTreeMap<String, Vec<Spot>> = BTreeMap::new();
    for line in SPOTS.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split_whitespace();
        let file = parts.next().expect("file name").to_owned();
        let message = parts.next().expect("message index").parse().expect("index");
        let _short_name = parts.next();
        let node = parts.next().expect("node index").parse().expect("node");
        let value = parts.next().expect("value").parse().expect("value");
        out.entry(file).or_default().push(Spot {
            message,
            node,
            value,
        });
    }
    out
}

fn directory() -> PathBuf {
    match std::env::var_os("VE_TEST_GRIBS") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").expect("HOME is set");
            PathBuf::from(home).join("temp_test_gribs")
        }
    }
}

/// Every file in the reference set decodes, and every value ecCodes reported
/// comes back within the packing's own step.
///
/// The tolerance is that step — `2^E / 10^D`, read from the message itself —
/// and not a fudge factor: a value that survives packing at all survives it
/// to within one step, so anything looser would let a genuinely misread
/// sample through and anything tighter would fail on the encoder's rounding.
#[test]
#[ignore = "needs the 17 MB sample set; see the module docs"]
fn the_reference_set_decodes_as_eccodes_reads_it() {
    let dir = directory();
    assert!(
        dir.is_dir(),
        "no sample set at {}; set VE_TEST_GRIBS",
        dir.display()
    );

    let mut files = 0;
    let mut checked = 0;
    for (name, spots) in reference() {
        let path = dir.join(&name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

        let started = Instant::now();
        let decoded = decode::read_all(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let elapsed = started.elapsed();

        assert!(
            decoded.skipped.is_empty(),
            "{name}: {:?} was skipped",
            decoded.skipped
        );
        assert!(!decoded.messages.is_empty(), "{name}: no messages");

        let values: usize = decoded.messages.iter().map(|m| m.values.len()).sum();
        println!(
            "{name}: {} messages, {values} values, 5.{} packing, {:.0} ms",
            decoded.messages.len(),
            decoded.messages[0].header.packing_template,
            elapsed.as_secs_f64() * 1e3
        );

        for spot in spots {
            let message = decoded
                .messages
                .get(spot.message)
                .unwrap_or_else(|| panic!("{name}: no message {}", spot.message));
            let got = *message
                .values
                .get(spot.node)
                .unwrap_or_else(|| panic!("{name}: no node {}", spot.node));

            // The message's own packing step: what one count of `X` is worth.
            let step = step_of(&bytes, spot.message);
            assert!(
                (got - spot.value).abs() <= step,
                "{name} message {} node {}: {got} against ecCodes' {}, \
                 outside the {step} packing step",
                spot.message,
                spot.node,
                spot.value
            );
            checked += 1;
        }
        files += 1;
    }
    println!("{files} files, {checked} spot values");
    assert_eq!(files, 10, "the reference set is ten files");
}

/// A dense message decodes fast enough to import a whole file interactively.
///
/// GEM's 0.15° grid is the largest in the set at 2,882,400 values, and a file
/// holds one message per component per step, so a decode measured in seconds
/// would make opening a real forecast unbearable. Compressed packings are
/// where that risk lives: a pure-Rust JPEG 2000 decoder had to be shown fast
/// enough, not merely correct.
#[test]
#[ignore = "needs the 17 MB sample set; see the module docs"]
fn the_densest_message_decodes_in_under_a_second() {
    let path = directory().join("gem_deterministic_sample.grib");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

    let started = Instant::now();
    let decoded = decode::read_all(&bytes).expect("gem decodes");
    let elapsed = started.elapsed();

    let values: usize = decoded.messages.iter().map(|m| m.values.len()).sum();
    let per_message = elapsed.as_secs_f64() / decoded.messages.len() as f64;
    println!(
        "gem_deterministic: {values} values in {:.0} ms, {:.0} ms per message",
        elapsed.as_secs_f64() * 1e3,
        per_message * 1e3
    );
    assert_eq!(values, 2 * 2_882_400, "two components of a 2400x1201 grid");
    assert!(
        per_message < 1.0,
        "a dense message took {per_message:.2} s to decode"
    );
}

/// The packing step of message `index`: `2^E / 10^D` from its section 5.
///
/// Read here rather than exposed by the decoder, because it is the *test's*
/// business to know what a decoded value's precision is; the decoder's job is
/// to have applied it.
fn step_of(bytes: &[u8], index: usize) -> f32 {
    let mut at = 0;
    for _ in 0..index {
        let length = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().expect("length"));
        at += length as usize;
    }
    let mut section = at + 16;
    loop {
        let length =
            u32::from_be_bytes(bytes[section..section + 4].try_into().expect("length")) as usize;
        if bytes[section + 4] == 5 {
            let raw_e = u16::from_be_bytes([bytes[section + 15], bytes[section + 16]]);
            let e = if raw_e & 0x8000 != 0 {
                -i32::from(raw_e & 0x7fff)
            } else {
                i32::from(raw_e)
            };
            let raw_d = u16::from_be_bytes([bytes[section + 17], bytes[section + 18]]);
            let d = if raw_d & 0x8000 != 0 {
                -i32::from(raw_d & 0x7fff)
            } else {
                i32::from(raw_d)
            };
            return 2f32.powi(e) / 10f32.powi(d);
        }
        assert!(length > 0, "a zero-length section");
        section += length;
    }
}
