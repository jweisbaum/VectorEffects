//! The WebDriver automation endpoint must not reach a shipped build.
//!
//! The plugin serves an axum WebDriver endpoint on loopback that can click,
//! type, screenshot and read the DOM — the whole interface, to any process on
//! the machine that reads the port it prints. Invariant 5 says nothing is
//! fetched that the user did not ask for, and an inbound socket that drives
//! the application is the same promise from the other side.
//!
//! `npm run check:offline` cannot enforce this: it reads source URLs, remote
//! references in the built bundle and the strength of the CSP, none of which
//! sees a listener. The enforcement is that the dependency is *not compiled
//! in* unless someone asks, and this is the test that says so — against the
//! manifest, because the manifest is where that decision is written and where
//! it would be undone.
//!
//! There is deliberately **no** `cfg!(feature = "webdriver")` assertion here.
//! It reads like a guard and is not one: it is a constant in any given build,
//! true in CI and false in an end-to-end run, so it says nothing the manifest
//! does not. Clippy rejects it as a constant assertion, and clippy is right.

/// The dependency is optional and no default feature turns it on.
///
/// Checked by reading `Cargo.toml` rather than by `cfg!(feature = ...)`: the
/// regression to catch is someone making the dependency unconditional or
/// adding it to a default feature list, and a `cfg!` check would simply
/// compile the other way and pass.
#[test]
fn the_webdriver_plugin_is_optional_and_off_by_default() {
    let manifest = include_str!("../Cargo.toml");
    const CRATE: &str = "tauri-plugin-webdriver-automation";

    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with(CRATE))
        .unwrap_or_else(|| panic!("{CRATE} is not in the manifest at all"));
    assert!(
        line.contains("optional = true"),
        "{CRATE} must be optional, or every build compiles the endpoint in: {line}"
    );

    // A `default` list anywhere in `[features]` must not reach it — directly
    // or through the feature that does.
    // Anchored to the start of a line: the word also appears in the prose
    // above, where the dependency explains itself, and splitting on the bare
    // token found the comment rather than the section.
    let features = manifest
        .split("\n[features]\n")
        .nth(1)
        .expect("the crate declares a [features] section");
    let features = features.split("\n[").next().unwrap_or(features);
    for line in features.lines() {
        let line = line.trim();
        if !line.starts_with("default") {
            continue;
        }
        assert!(
            !line.contains("webdriver") && !line.contains(CRATE),
            "a default feature turns the WebDriver endpoint on: {line}"
        );
    }

    // And the feature that does turn it on exists, so the flag in the npm
    // script and in CLAUDE.md is a real one rather than a typo that silently
    // builds without it.
    assert!(
        features
            .lines()
            .any(|line| line.trim_start().starts_with("webdriver =")),
        "the `webdriver` feature is gone; `npm run dev:webdriver` would build nothing"
    );
}

/// The guard bites: a manifest that shipped the endpoint fails the rule above.
///
/// Without this the two assertions could both be vacuous — a pattern that
/// matches nothing passes as happily as one that matches the right thing.
#[test]
fn the_guard_would_catch_a_manifest_that_shipped_it() {
    const CRATE: &str = "tauri-plugin-webdriver-automation";
    let shipped = format!("[dependencies]\n{CRATE} = \"0.1.3\"\n");
    let line = shipped
        .lines()
        .find(|line| line.trim_start().starts_with(CRATE))
        .expect("the stand-in names the crate");
    assert!(
        !line.contains("optional = true"),
        "the optional check would pass a manifest that compiles it in always"
    );

    let defaulted = "\n[features]\ndefault = [\"webdriver\"]\nwebdriver = []\n";
    let features = defaulted
        .split("\n[features]\n")
        .nth(1)
        .expect("a [features] section");
    let features = features.split("\n[").next().unwrap_or(features);
    assert!(
        features
            .lines()
            .any(|line| line.trim().starts_with("default") && line.contains("webdriver")),
        "the default-feature check would miss a default that turns it on"
    );
}
