//! Writes the sample projects, for the repository and the documentation.
//!
//!     cargo run -p ve-app --example make_samples -- assets/samples
//!
//! Built through the real pipeline — project, tools, save — so each sample is
//! exactly the file the application would have produced by hand, and reopens
//! through the same path a user's file does. Deterministic: ids come from a
//! per-process counter and the archive is canonical, so running this twice
//! yields the same bytes and a change in the samples is a change in the
//! application.

use ve_app::commands::AppState;
use ve_app::create::{self, Gesture, NewObject, Tool, ToolOption};
use ve_app::document::PropertyValue;
use ve_app::edit::{self, BrushStroke};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};

fn number(property: &str, value: f64) -> ToolOption {
    ToolOption {
        property: property.to_owned(),
        value: PropertyValue::Number { value },
    }
}

fn new_state(root: &std::path::Path, label: &str) -> Result<AppState, Box<dyn std::error::Error>> {
    let dir = root.join(label);
    std::fs::create_dir_all(&dir)?;
    Ok(AppState::new(AppPaths::in_directory(&dir)?))
}

fn project(
    state: &AppState,
    name: &str,
    kind: &str,
    resolution: &str,
    step_hours: u32,
    steps: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    projects::create(
        state,
        NewProjectRequest {
            name: name.to_owned(),
            field_kind: kind.to_owned(),
            resolution: resolution.to_owned(),
            step_hours,
            step_count: steps,
        },
        true,
    )?;
    Ok(())
}

fn stroke(
    state: &AppState,
    points: Vec<[f64; 2]>,
    size_km: f32,
    speed_mps: f32,
    toward: f64,
    feather: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    edit::paint(
        state,
        BrushStroke {
            points,
            size_km,
            speed_mps,
            direction_toward_deg: toward,
            feather,
            layer: None,
            ..Default::default()
        },
    )?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "assets/samples".to_owned()),
    );
    std::fs::create_dir_all(&out)?;
    let scratch = std::env::temp_dir().join(format!("ve-samples-{}", std::process::id()));

    // 1. Trade winds: two easterly belts and the westerlies between, the
    //    global picture every sailor learns first.
    {
        let state = new_state(&scratch, "trades")?;
        project(&state, "Trade winds", "wind", "1.0", 6, 8)?;
        stroke(
            &state,
            vec![[-179.0, 15.0], [0.0, 15.0], [179.0, 15.0]],
            2_200.0,
            8.0,
            260.0,
            0.4,
        )?;
        stroke(
            &state,
            vec![[-179.0, -15.0], [0.0, -15.0], [179.0, -15.0]],
            2_200.0,
            8.0,
            290.0,
            0.4,
        )?;
        stroke(
            &state,
            vec![[-179.0, 45.0], [0.0, 45.0], [179.0, 45.0]],
            2_600.0,
            11.0,
            80.0,
            0.5,
        )?;
        stroke(
            &state,
            vec![[-179.0, -45.0], [0.0, -45.0], [179.0, -45.0]],
            2_600.0,
            14.0,
            95.0,
            0.5,
        )?;
        projects::save_as(
            &state,
            out.join("trade-winds.veproj")
                .to_string_lossy()
                .into_owned(),
        )?;
    }

    // 2. A cyclone over a steady flow: a stroke, then converge and rotate
    //    modifiers over it — the spiral built from objects.
    {
        let state = new_state(&scratch, "cyclone")?;
        project(&state, "Cyclone", "wind", "0.5", 3, 16)?;
        stroke(
            &state,
            vec![[-60.0, 30.0], [-20.0, 30.0]],
            2_400.0,
            10.0,
            90.0,
            0.3,
        )?;
        for (tool, options) in [
            (
                Tool::Divergence,
                vec![
                    number("SizeKm", 1_600.0),
                    number("Radial", -120.0),
                    number("Feather", 0.6),
                ],
            ),
            (
                Tool::Turn,
                vec![
                    number("SizeKm", 1_600.0),
                    number("TurnDeg", -70.0),
                    number("Feather", 0.6),
                ],
            ),
            (
                Tool::Intensity,
                vec![
                    number("SizeKm", 1_000.0),
                    number("Gain", 150.0),
                    number("Feather", 0.7),
                ],
            ),
        ] {
            create::create(
                &state,
                NewObject {
                    tool,
                    gesture: Gesture::Stroke {
                        points: vec![[-40.0, 30.0]],
                    },
                    options,
                    layer: None,
                },
            )?;
        }
        projects::save_as(
            &state,
            out.join("cyclone.veproj").to_string_lossy().into_owned(),
        )?;
    }

    // 3. An ocean gyre: a current project with a rotating circle stamp.
    {
        let state = new_state(&scratch, "gyre")?;
        project(&state, "Subtropical gyre", "current", "0.5", 6, 4)?;
        create::create(
            &state,
            NewObject {
                tool: Tool::Circle,
                gesture: Gesture::Point { at: [-50.0, 30.0] },
                options: vec![
                    number("DiameterKm", 4_000.0),
                    number("Speed", 0.6),
                    number("Feather", 0.5),
                ],
                layer: None,
            },
        )?;
        projects::save_as(
            &state,
            out.join("gyre.veproj").to_string_lossy().into_owned(),
        )?;
    }

    let _ = std::fs::remove_dir_all(&scratch);
    println!("samples written to {}", out.display());
    Ok(())
}
