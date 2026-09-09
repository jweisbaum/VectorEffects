//! IPC commands.
//!
//! Every type crossing this boundary derives `TS`, so the frontend never
//! hand-writes a Rust-shaped type. Regenerate with:
//! `cargo run -p ve-app --bin export-bindings`.
//!
//! This is also the only layer permitted to convert between stored
//! azimuth-toward and the user's display convention (spec.md 3.3).

use serde::Serialize;
use ts_rs::TS;

use crate::error::{AppError, Context, Result};
use crate::paths::{AppPaths, display_path};

/// Which field evaluator backend is active, and why.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "EvaluatorSelection.ts")]
pub struct EvaluatorSelection {
    /// Backend name, e.g. `"wgpu"` or `"cpu"`.
    pub backend: String,
    /// Why the GPU backend was not used, if it was not. `None` when GPU is active.
    pub fallback_reason: Option<String>,
}

impl EvaluatorSelection {
    /// Describes a chosen backend.
    fn of(backend: &str, reason: Option<String>) -> Self {
        Self {
            backend: backend.to_owned(),
            fallback_reason: reason,
        }
    }
}

/// The evaluators available this run.
///
/// Both are kept: the GPU is the fast path for preview, and the CPU is both the
/// fallback and the authority for export (spec.md 7.8). The GPU also declines
/// scenes containing a clone stamp, which needs recursion, so the fallback is
/// used even where a GPU exists.
#[derive(Debug)]
pub struct Evaluators {
    /// The GPU backend, when one could be created.
    pub gpu: Option<ve_render::gpu::GpuEvaluator>,
    /// How the choice is described to the user.
    pub selection: EvaluatorSelection,
}

impl Evaluators {
    /// Probes for a GPU, falling back to the CPU with a stated reason.
    pub fn detect() -> Self {
        if std::env::var("VE_FORCE_CPU").is_ok_and(|value| value == "1") {
            return Self {
                gpu: None,
                selection: EvaluatorSelection::of(
                    "cpu",
                    Some("forced by VE_FORCE_CPU=1".to_owned()),
                ),
            };
        }

        match ve_render::gpu::GpuEvaluator::new() {
            Ok(gpu) => {
                let selection = EvaluatorSelection::of(&format!("wgpu — {}", gpu.adapter), None);
                Self {
                    gpu: Some(gpu),
                    selection,
                }
            }
            Err(err) => Self {
                gpu: None,
                selection: EvaluatorSelection::of("cpu", Some(err.to_string())),
            },
        }
    }
}

/// Static facts about this build and installation.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "AppInfo.ts")]
pub struct AppInfo {
    /// Product name.
    pub name: String,
    /// Semantic version of this build.
    pub version: String,
    /// The active evaluator backend.
    pub evaluator: EvaluatorSelection,
    /// Where log files are written.
    pub log_dir: String,
    /// Where the evictable render cache lives. Safe to delete when closed.
    pub cache_dir: String,
    /// Whether to capture the first rendered frame to the log directory.
    ///
    /// Set by `VE_CAPTURE=1`. A development aid: it lets a rendered frame be
    /// inspected without screen-capture permissions.
    pub debug_capture: bool,
}

/// A field sample at one position, for the cursor readout.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "FieldSample.ts")]
pub struct FieldSample {
    /// Speed in metres per second.
    pub speed_mps: f64,
    /// Azimuth the vector points toward, degrees clockwise from north.
    ///
    /// Always the "toward" convention. The UI applies the project's display
    /// convention; no code below this boundary sees a "from" bearing
    /// (spec.md 3.3).
    pub azimuth_toward_deg: f64,
    /// Whether anything wrote the cell (M31, D58): a calm cell some layer
    /// wrote is defined; one nothing wrote is not, whatever the map shows
    /// beneath it.
    pub defined: bool,
    /// The kind of the layer the cell shows — `wind` or `current` — when
    /// the composite was sampled; the kind asked for otherwise.
    pub kind: String,
}

/// Samples the field at one position.
///
/// The readout calls this rather than decoding the tile texture, so it reports
/// the true value instead of the tile's quantisation, and no second copy of
/// the tile bytes has to be kept on the JavaScript side.
///
/// With a `kind`, the field of that kind alone — what the eyedropper wants
/// for the layer it will paint. Without one, the composite the map shows
/// (M31): every layer stacked, and the kind of the one that wins the cell.
#[tauri::command]
pub fn sample_field(
    state: tauri::State<'_, AppState>,
    lon: f64,
    lat: f64,
    step: u32,
    kind: Option<String>,
) -> Result<FieldSample> {
    let position = ve_core::LonLat::new(lon, lat)?;
    let kind = kind
        .as_deref()
        .map(crate::projects::parse_field_kind)
        .transpose()?;

    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
    let Some(open) = session.open.as_mut() else {
        return Ok(FieldSample {
            speed_mps: 0.0,
            azimuth_toward_deg: 0.0,
            defined: false,
            kind: crate::projects::kind_name(kind.unwrap_or_default()).to_owned(),
        });
    };

    let scene = match kind {
        Some(kind) => ve_render::scene::flatten_kind(&open.project, step, kind),
        None => ve_render::scene::flatten(&open.project, step),
    };
    let sample = ve_render::cpu::composite(&scene, position);
    let (speed, azimuth) = ve_core::vector::speed_azimuth_from_uv(sample.uv);
    Ok(FieldSample {
        speed_mps: speed,
        azimuth_toward_deg: azimuth.degrees(),
        defined: sample.coverage > 0.0,
        kind: crate::projects::kind_name(kind.unwrap_or(sample.kind)).to_owned(),
    })
}

/// Records a message from the frontend in the application log.
///
/// Without this, a failure in the webview goes only to a developer console
/// nobody is watching, and the app just silently shows nothing. Frontend errors
/// belong in the same log file as everything else.
#[tauri::command]
pub fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "frontend", "{message}"),
        "warn" => tracing::warn!(target: "frontend", "{message}"),
        _ => tracing::info!(target: "frontend", "{message}"),
    }
}

/// Decodes standard base64. Small enough not to warrant a dependency, and only
/// the debug capture path uses it.
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut lookup = [255u8; 256];
    for (i, c) in TABLE.iter().enumerate() {
        lookup[*c as usize] = i as u8;
    }

    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte == b'=' || byte == b'\n' || byte == b'\r' {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return None;
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// Writes a rendered frame to the log directory, for development checks.
///
/// The renderer reads its own pixels back and sends them here, so a frame can
/// be inspected without screen-capture permissions and without photographing
/// anything but the app's own canvas.
#[tauri::command]
pub fn save_debug_capture(
    state: tauri::State<'_, AppState>,
    name: String,
    png_base64: String,
) -> Result<String> {
    // Base64 rather than a byte array: a megabyte of pixels as a JSON array of
    // a million numbers is pathologically slow to serialise.
    let bytes = decode_base64(&png_base64)
        .ok_or_else(|| AppError::Internal("capture was not valid base64".to_owned()))?;
    // Names come from the frontend, so keep them to a safe basename.
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let path = state.paths.log_dir.join(format!("capture-{safe}.png"));
    std::fs::write(&path, bytes).doing("write the capture to", path.display())?;
    tracing::info!(path = %path.display(), "wrote debug capture");
    Ok(display_path(&path))
}

/// Shared application state.
#[derive(Debug)]
pub struct AppState {
    /// Resolved application directories.
    pub paths: AppPaths,
    /// The evaluator backends available this run.
    pub evaluators: Evaluators,
    /// The open project and the recent-files list.
    pub session: std::sync::Mutex<crate::session::Session>,
    /// Rendered tiles, keyed by the content of the scene that produced them.
    ///
    /// Evictable and never project data: deleting it while the app is closed is
    /// always safe (invariant 1).
    pub tiles: ve_render::cache::RenderCache,
}

impl AppState {
    /// Builds the state from resolved paths, restoring the recent-files list.
    pub fn new(paths: AppPaths) -> Self {
        let session = crate::session::Session::load(&paths.settings_file());
        // A cache that cannot be opened is not fatal: rendering simply costs
        // more. Falling back keeps a read-only or full disk from stopping the
        // application entirely.
        let tiles = ve_render::cache::RenderCache::open(
            &paths.cache_dir,
            ve_render::cache::DEFAULT_CAPACITY_BYTES,
        )
        .unwrap_or_else(|err| {
            tracing::warn!(%err, "render cache unavailable; rendering uncached");
            ve_render::cache::RenderCache::open(
                std::env::temp_dir().join("vectoreffects-cache"),
                ve_render::cache::DEFAULT_CAPACITY_BYTES,
            )
            .unwrap_or_else(|_| unreachable!("temp dir cache must open"))
        });

        Self {
            evaluators: Evaluators::detect(),
            session: std::sync::Mutex::new(session),
            tiles,
            paths,
        }
    }
}

/// Returns the bundled basemap asset as raw bytes.
///
/// Sent unparsed: the renderer wants typed arrays, so decoding here only to
/// re-encode for the webview would be wasted work. Validated at startup, so a
/// corrupt asset is already an error by the time this is called.
#[tauri::command]
pub fn basemap() -> tauri::ipc::Response {
    tauri::ipc::Response::new(ve_render::basemap::EMBEDDED.to_vec())
}

/// The URL prefix for tile requests.
///
/// Tauri maps custom URI schemes differently per platform, so the frontend is
/// told the prefix rather than reimplementing that rule.
#[tauri::command]
pub fn tile_base_url() -> String {
    crate::protocol::base_url()
}

/// Returns build and installation facts for the About panel.
#[tauri::command]
pub fn app_info(state: tauri::State<'_, AppState>) -> Result<AppInfo> {
    tracing::debug!("app_info requested");
    Ok(AppInfo {
        name: "VectorEffects".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        evaluator: state.evaluators.selection.clone(),
        log_dir: display_path(&state.paths.log_dir),
        cache_dir: display_path(&state.paths.cache_dir),
        debug_capture: std::env::var("VE_CAPTURE").is_ok_and(|v| v == "1"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_known_vectors() {
        assert_eq!(decode_base64("").as_deref(), Some(&[][..]));
        assert_eq!(decode_base64("TWE=").as_deref(), Some(&b"Ma"[..]));
        assert_eq!(decode_base64("TWFu").as_deref(), Some(&b"Man"[..]));
        assert_eq!(
            decode_base64("bGlnaHQgdw==").as_deref(),
            Some(&b"light w"[..])
        );
        // PNG magic, which is what this path actually carries.
        assert_eq!(
            decode_base64("iVBORw0KGgo=").as_deref(),
            Some(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a][..])
        );
    }

    #[test]
    fn base64_rejects_junk() {
        assert!(decode_base64("not valid!").is_none());
    }

    #[test]
    fn base64_round_trips_arbitrary_bytes() {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let original: Vec<u8> = (0u16..=511).map(|v| (v % 256) as u8).collect();

        let mut encoded = String::new();
        for chunk in original.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    encoded.push(TABLE[((n >> (18 - i * 6)) & 63) as usize] as char);
                } else {
                    encoded.push('=');
                }
            }
        }
        assert_eq!(
            decode_base64(&encoded).as_deref(),
            Some(original.as_slice())
        );
    }

    /// Whichever backend is chosen, the About panel must be able to say what
    /// it is — and a CPU selection must explain why it is not the GPU.
    #[test]
    fn the_chosen_backend_describes_itself() {
        let evaluators = Evaluators::detect();
        let selection = &evaluators.selection;

        assert!(!selection.backend.is_empty());
        if evaluators.gpu.is_some() {
            assert!(selection.backend.starts_with("wgpu"), "{selection:?}");
            assert!(
                selection.fallback_reason.is_none(),
                "the GPU needs no excuse"
            );
        } else {
            assert_eq!(selection.backend, "cpu");
            assert!(
                selection.fallback_reason.is_some(),
                "a CPU selection must say why the GPU was not used"
            );
        }
    }
}
