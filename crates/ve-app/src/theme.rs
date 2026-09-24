//! Application themes shared with the frontend, including display-only charts.
use std::{collections::BTreeMap, sync::OnceLock};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_chart::s57::Palette;

/// Keep the current appearance when older preferences have no theme.
pub const DEFAULT_THEME: &str = "sage";

/// A saved palette starts from a bundled theme and overrides named colours.
/// Keys are `roles.<name>`, `map.<name>` or `chart.<name>` from the catalogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "CustomTheme.ts")]
pub struct CustomTheme {
    pub base: String,
    pub colours: BTreeMap<String, String>,
}

/// An entry in the shared theme catalogue.
#[derive(Debug, Clone, Deserialize)]
pub struct Theme {
    pub id: String,
    roles: BTreeMap<String, String>,
    map: BTreeMap<String, String>,
    chart: BTreeMap<String, String>,
}

/// Read the bundled catalogue once; no external resource is loaded.
pub fn themes() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        serde_json::from_str(include_str!("../../../ui/src/settings/themes.json"))
            .unwrap_or_default()
    })
}

/// Whether a preference names a bundled theme.
pub fn known(id: &str) -> bool {
    themes().iter().any(|theme| theme.id == id)
}

impl CustomTheme {
    /// Only catalogue roles and six-digit RGB colours may enter preferences.
    pub fn valid(&self) -> bool {
        let Some(base) = themes().iter().find(|theme| theme.id == self.base) else {
            return false;
        };
        self.colours.iter().all(|(key, colour)| {
            base.colour(key).is_some()
                && colour.len() == 7
                && colour.starts_with('#')
                && colour.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        })
    }
}

impl Theme {
    fn colour(&self, key: &str) -> Option<&String> {
        let (group, name) = key.split_once('.')?;
        match group {
            "roles" => self.roles.get(name),
            "map" => self.map.get(name),
            "chart" => self.chart.get(name),
            _ => None,
        }
    }

    /// Chart land uses the exact same RGB values as the themed basemap.
    pub fn chart_palette(&self) -> Palette {
        let colour = |group: &BTreeMap<String, String>, name: &str, alpha| {
            crate::charts::rgba_of(
                group.get(name).map(String::as_str).unwrap_or("#000000"),
                alpha,
            )
        };
        Palette {
            land: colour(&self.map, "land", 1.0),
            shallow: colour(&self.chart, "shallow", 1.0),
            middle: colour(&self.chart, "middle", 1.0),
            deep: colour(&self.chart, "deep", 1.0),
            coast: colour(&self.chart, "coast", 1.0),
            contour: colour(&self.chart, "contour", 180.0 / 255.0),
            text: colour(&self.chart, "text", 220.0 / 255.0),
            built: colour(&self.chart, "built", 1.0),
            ..Palette::default()
        }
    }
}

/// The resolved chart palette for a preset or the saved custom appearance.
pub fn palette_for(id: &str, custom: Option<&CustomTheme>) -> Palette {
    if id == "custom"
        && let Some(custom) = custom.filter(|custom| custom.valid())
        && let Some(base) = themes().iter().find(|theme| theme.id == custom.base)
    {
        let mut theme = base.clone();
        for (key, colour) in &custom.colours {
            if let Some(name) = key.strip_prefix("map.") {
                theme.map.insert(name.to_owned(), colour.clone());
            } else if let Some(name) = key.strip_prefix("chart.") {
                theme.chart.insert(name.to_owned(), colour.clone());
            }
        }
        return theme.chart_palette();
    }
    chart_palette(id)
}

/// The chart palette, with the current default for unknown preferences.
pub fn chart_palette(id: &str) -> Palette {
    themes()
        .iter()
        .find(|theme| theme.id == id)
        .map(Theme::chart_palette)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_keeps_both_existing_appearances_and_the_default_chart_land() {
        assert_eq!(themes().len(), 6);
        assert!(known("original"));
        assert!(known(DEFAULT_THEME));
        assert_eq!(chart_palette(DEFAULT_THEME).land, Palette::default().land);
    }
}
