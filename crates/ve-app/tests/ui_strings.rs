#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The interface text that reaches the webview from the schema.
//!
//! Tool names, option labels and slider ends are written once, in `ve-core`,
//! and shown by the frontend, which translates them (spec.md 5.7). They are
//! listed in `ui/src/i18n/rust-strings.json` so the frontend's coverage test
//! can hold every catalogue to them: a label added here without a translation
//! fails there, and a label added without updating the list fails here.
//!
//! `VE_BLESS=1 cargo test -p ve-app --test ui_strings` rewrites the list.

use std::collections::BTreeSet;
use ve_core::schema::{self, ToolKind};

fn strings() -> Vec<String> {
    let mut all = BTreeSet::new();
    for tool in ToolKind::ALL {
        all.insert(tool.label());
        for spec in schema::all_specs(tool) {
            all.insert(spec.label);
            if let Some(slider) = spec.slider {
                all.insert(slider.low_label);
                all.insert(slider.high_label);
            }
        }
    }
    // The inspector's one relabelled row (`document::properties`).
    all.insert("Selection position");
    all.into_iter()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_frontend_lists_every_schema_label_it_must_translate() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/src/i18n/rust-strings.json");
    let expected = serde_json::to_string_pretty(&strings()).unwrap() + "\n";
    if std::env::var_os("VE_BLESS").is_some() {
        std::fs::write(&path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "ui/src/i18n/rust-strings.json is out of date; rerun with VE_BLESS=1 and translate the new labels"
    );
}
