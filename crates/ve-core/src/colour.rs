//! The colour gradients the map paints speed with (spec.md 5.3).
//!
//! A gradient is presentation and reaches no exported file: the tiles carry
//! speed and direction, and the colour is chosen when a pixel is drawn. So
//! changing one costs no tile and no re-evaluation, which is why it can be a
//! live setting at all — the same reason the scale can be (§5.3).
//!
//! # Why the table is here and not in the frontend
//!
//! The document stores which gradient a project uses, so the document has to
//! know which ones exist: a project naming a gradient nothing can draw would
//! open looking like something else without saying so. Keeping the stops
//! beside the names means there is one table rather than a list of names here
//! and a list of colours there, which is the arrangement that drifts.
//!
//! # Why a gradient is stops rather than a formula
//!
//! Every gradient below is sampled from a published palette, so it is a list
//! of colours by construction. They are **approximations**, taken at the
//! anchor points the palettes are usually quoted at rather than at their full
//! resolution: a ramp is read as "faster is hotter" and not as a measurement,
//! and the difference between eight stops and two hundred is not visible once
//! it is stretched over a colour bar an inch tall.

use serde::{Deserialize, Serialize};

use crate::project::FieldKind;

/// One gradient: linear RGB stops, evenly spaced from calm to the top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    /// What the document stores.
    pub id: &'static str,
    /// What the settings offer.
    pub label: &'static str,
    /// A sentence on what it is for.
    pub note: &'static str,
    /// Stops as `0.0`–`1.0` RGB, calm first, evenly spaced.
    pub stops: &'static [[f32; 3]],
}

/// The gradient a project uses when it names none, and the one every project
/// made before the setting existed is still drawn with.
pub const DEFAULT_ID: &str = "vector";

/// The gradient the currents start on.
///
/// Currents are read for where the water is going as much as for how fast, so
/// they open on a palette built for exactly that — light where the water is
/// slack and dark where it runs — rather than on the wind's.
pub const DEFAULT_CURRENT_ID: &str = "speed";

/// Every gradient, in the order the settings list them.
///
/// The app's own first, then the perceptually uniform ones, then the ones a
/// chart reader is likely to recognise, then grey for a page that will be
/// printed.
pub const GRADIENTS: &[Gradient] = &[
    Gradient {
        id: "vector",
        label: "VectorEffects",
        note: "The application's own: dark at calm, hot at the top.",
        stops: &[
            [0.05, 0.09, 0.16],
            [0.12, 0.35, 0.62],
            [0.16, 0.68, 0.66],
            [0.55, 0.80, 0.35],
            [0.96, 0.78, 0.24],
            [0.92, 0.42, 0.20],
            [0.78, 0.16, 0.36],
        ],
    },
    Gradient {
        id: "viridis",
        label: "Viridis",
        note: "Perceptually uniform, and legible to every kind of colour blindness.",
        stops: &[
            [0.267, 0.005, 0.329],
            [0.267, 0.224, 0.514],
            [0.192, 0.408, 0.556],
            [0.129, 0.569, 0.549],
            [0.208, 0.718, 0.473],
            [0.565, 0.843, 0.263],
            [0.992, 0.906, 0.145],
        ],
    },
    Gradient {
        id: "plasma",
        label: "Plasma",
        note: "Perceptually uniform, and brighter at the top than Viridis.",
        stops: &[
            [0.050, 0.030, 0.528],
            [0.361, 0.004, 0.645],
            [0.612, 0.090, 0.620],
            [0.799, 0.278, 0.470],
            [0.929, 0.473, 0.326],
            [0.992, 0.700, 0.180],
            [0.940, 0.975, 0.131],
        ],
    },
    Gradient {
        id: "inferno",
        label: "Inferno",
        note: "Perceptually uniform, black at calm: the strongest contrast at the top.",
        stops: &[
            [0.001, 0.000, 0.014],
            [0.166, 0.045, 0.279],
            [0.397, 0.083, 0.433],
            [0.622, 0.165, 0.388],
            [0.831, 0.283, 0.260],
            [0.961, 0.489, 0.084],
            [0.988, 0.760, 0.153],
            [0.988, 0.998, 0.645],
        ],
    },
    Gradient {
        id: "white-blue-green-yellow-red",
        label: "Blue to red (NCL)",
        note: "The NCAR palette much of the published meteorology is drawn with.",
        stops: &[
            [1.000, 1.000, 1.000],
            [0.298, 0.620, 0.910],
            [0.204, 0.788, 0.627],
            [0.498, 0.831, 0.298],
            [0.949, 0.890, 0.235],
            [0.949, 0.549, 0.118],
            [0.851, 0.169, 0.118],
        ],
    },
    Gradient {
        id: "haxby",
        label: "Haxby",
        note: "The GMT bathymetry palette: blue through green to a warm top.",
        stops: &[
            [0.039, 0.000, 0.475],
            [0.125, 0.376, 1.000],
            [0.251, 0.690, 1.000],
            [0.502, 1.000, 0.867],
            [0.627, 0.941, 0.627],
            [0.941, 0.941, 0.439],
            [1.000, 0.690, 0.376],
            [1.000, 0.376, 0.376],
        ],
    },
    Gradient {
        id: "speed",
        label: "Speed",
        note: "The cmocean palette drawn for current speed: pale when slack, dark when it runs.",
        stops: &[
            [1.000, 0.992, 0.929],
            [0.831, 0.902, 0.643],
            [0.576, 0.812, 0.404],
            [0.310, 0.710, 0.290],
            [0.102, 0.588, 0.251],
            [0.043, 0.443, 0.208],
            [0.039, 0.298, 0.169],
            [0.078, 0.157, 0.118],
        ],
    },
    Gradient {
        id: "thermal",
        label: "Thermal",
        note: "The cmocean palette: deep blue at calm, through violet, to a pale top.",
        stops: &[
            [0.016, 0.137, 0.200],
            [0.173, 0.200, 0.584],
            [0.455, 0.286, 0.573],
            [0.694, 0.373, 0.510],
            [0.922, 0.475, 0.345],
            [0.984, 0.706, 0.298],
            [0.910, 0.980, 0.357],
        ],
    },
    Gradient {
        id: "greyscale",
        label: "Greyscale",
        note: "No hue at all, for a chart that will be printed or read in mono.",
        stops: &[
            [0.078, 0.078, 0.086],
            [0.267, 0.271, 0.282],
            [0.451, 0.459, 0.475],
            [0.639, 0.647, 0.663],
            [0.827, 0.835, 0.847],
            [0.976, 0.980, 0.988],
        ],
    },
];

/// The gradient an identifier names, or `None` if nothing does.
pub fn gradient(id: &str) -> Option<&'static Gradient> {
    GRADIENTS.iter().find(|entry| entry.id == id)
}

/// The gradient an identifier names, falling back to the default.
///
/// A project written by a later version may name a gradient this one does not
/// have. Drawing it with the default and carrying the name through unchanged
/// is what lets the file go back to that version intact; refusing to open it
/// would be a worse answer to "one of the colours is unfamiliar".
pub fn gradient_or_default(id: &str) -> &'static Gradient {
    gradient(id)
        .or_else(|| gradient(DEFAULT_ID))
        .unwrap_or(&GRADIENTS[0])
}

/// Which gradient each kind of field is drawn with (spec.md 5.3).
///
/// One per kind, because one tile carries both and the map draws them at once
/// (M31): a single gradient for the two would say a six-knot current and a
/// sixty-knot gale were the same colour, which is exactly what the separate
/// scales exist to avoid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColourGradients {
    /// The gradient the wind layers are drawn with.
    pub wind: String,
    /// And the current layers.
    pub current: String,
}

impl Default for ColourGradients {
    fn default() -> Self {
        Self {
            wind: DEFAULT_ID.to_owned(),
            current: DEFAULT_CURRENT_ID.to_owned(),
        }
    }
}

impl ColourGradients {
    /// The identifier one kind of field is drawn with.
    pub fn id_for(&self, kind: FieldKind) -> &str {
        match kind {
            FieldKind::Wind => &self.wind,
            FieldKind::Current => &self.current,
        }
    }

    /// The same choice with one kind's gradient replaced.
    pub fn with_id(mut self, kind: FieldKind, id: impl Into<String>) -> Self {
        match kind {
            FieldKind::Wind => self.wind = id.into(),
            FieldKind::Current => self.current = id.into(),
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identifier is what a project file keeps, so a duplicate would make
    /// one of the two unreachable and the file ambiguous.
    #[test]
    fn every_identifier_is_unique_and_usable() {
        let mut seen = std::collections::BTreeSet::new();
        for entry in GRADIENTS {
            assert!(seen.insert(entry.id), "{} appears twice", entry.id);
            assert!(!entry.label.is_empty(), "{} has no label", entry.id);
            assert!(!entry.note.is_empty(), "{} has no note", entry.id);
            assert_eq!(gradient(entry.id), Some(entry));
        }
        assert!(gradient("no-such-gradient").is_none());
    }

    /// A gradient needs two stops to be a gradient, and its colours have to be
    /// colours: a channel outside the range is a value the shader clamps
    /// silently, so the table would be wrong without anything saying so.
    #[test]
    fn every_gradient_is_a_run_of_real_colours() {
        for entry in GRADIENTS {
            assert!(
                entry.stops.len() >= 2,
                "{} has {} stops",
                entry.id,
                entry.stops.len()
            );
            for stop in entry.stops {
                for channel in stop {
                    assert!(
                        (0.0..=1.0).contains(channel),
                        "{} has a channel at {channel}",
                        entry.id
                    );
                }
            }
        }
    }

    /// A ramp has to *go* somewhere. Calm and the top must not look alike, or
    /// the two speeds furthest apart are the ones hardest to tell apart; and
    /// no two stops in a row may be the same colour, which would flatten a
    /// stretch of the scale into one shade. Measured as distance in RGB,
    /// which is crude but is the thing being asserted — that they differ —
    /// rather than a restatement of the numbers in the table.
    #[test]
    fn every_gradient_travels_and_never_stalls() {
        let apart = |a: &[f32; 3], b: &[f32; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        for entry in GRADIENTS {
            let first = entry.stops.first().expect("a first stop");
            let last = entry.stops.last().expect("a last stop");
            assert!(
                apart(first, last) > 0.4,
                "{}: calm and the top are only {:.2} apart",
                entry.id,
                apart(first, last)
            );
            for pair in entry.stops.windows(2) {
                assert!(
                    apart(&pair[0], &pair[1]) > 0.05,
                    "{}: two stops in a row are {:.3} apart",
                    entry.id,
                    apart(&pair[0], &pair[1])
                );
            }
        }
    }

    /// The two defaults have to exist, or a new project opens on a gradient
    /// that falls back to something else.
    #[test]
    fn the_defaults_are_in_the_table() {
        assert!(gradient(DEFAULT_ID).is_some());
        assert!(gradient(DEFAULT_CURRENT_ID).is_some());
        let chosen = ColourGradients::default();
        assert_eq!(chosen.id_for(FieldKind::Wind), DEFAULT_ID);
        assert_eq!(chosen.id_for(FieldKind::Current), DEFAULT_CURRENT_ID);
    }

    /// A gradient this version does not know is drawn with the default rather
    /// than refused, so a file from a later version still opens.
    #[test]
    fn an_unknown_gradient_falls_back_rather_than_failing() {
        assert_eq!(gradient_or_default("from-a-later-version").id, DEFAULT_ID);
        assert_eq!(gradient_or_default("viridis").id, "viridis");
    }

    #[test]
    fn one_kind_is_replaced_without_touching_the_other() {
        let chosen = ColourGradients::default().with_id(FieldKind::Wind, "viridis");
        assert_eq!(chosen.wind, "viridis");
        assert_eq!(chosen.current, DEFAULT_CURRENT_ID);
    }
}
