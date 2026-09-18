//! The Claude Desktop extension (spec.md 8.8): an `.mcpb` bundle the
//! application writes and hands to Claude Desktop's own installer.
//!
//! A desktop extension is a zip holding a `manifest.json` and a **local stdio
//! server**; it cannot name an HTTP service. So the bundle carries
//! `bridge.js`, which stands between Claude Desktop's stdin and stdout and
//! this application's loopback endpoint, run by the Node that Claude Desktop
//! ships. It is written here rather than packed with the `mcpb` tool because
//! it is three files and the people who press the button have neither Node
//! nor npm.
//!
//! **The bundle holds no secret.** The bridge reads the port and the token
//! from the application's settings file each time it needs them — a file
//! only this account can read (`session.rs`), which any process of this
//! account, Claude Desktop's included, could read anyway. So a rotated token
//! or a changed port needs no reinstall, and the file the installer copies
//! into Claude Desktop's folder gives nothing away.
//!
//! Installing is Claude Desktop's to do and the person's to confirm: this
//! opens the bundle in it and goes no further.

use std::io::Write;
use std::path::Path;

use crate::error::{AppError, Context, Result};

/// The stdio server inside the bundle.
pub const BRIDGE: &str = include_str!("bridge.js");

/// The application's icon; the manifest asks for 512 px or more.
const ICON: &[u8] = include_bytes!("../../icons/icon.png");

/// The bundle's file name, in the application's own folder.
pub const BUNDLE_NAME: &str = "VectorEffects.mcpb";

/// `manifest.json` (MCPB manifest 0.3).
///
/// `settings_file` reaches the bridge as `VE_SETTINGS`: where the settings
/// live is this application's to know, and it differs by platform and under
/// test, so it is stated rather than worked out a second time in JavaScript.
pub fn manifest(settings_file: &Path) -> serde_json::Value {
    serde_json::json!({
        "manifest_version": "0.3",
        "name": "vectoreffects",
        "display_name": "VectorEffects",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Drive VectorEffects from Claude: download real past wind and current, draw and animate weather, and export GRIB2 or Zarr.",
        "long_description": "Connects Claude to the VectorEffects application running on this computer, through its MCP service. VectorEffects has to be open with Settings > MCP service enabled; nothing leaves this machine.",
        "author": { "name": "VectorEffects" },
        "icon": "icon.png",
        "server": {
            "type": "node",
            "entry_point": "server/index.js",
            "mcp_config": {
                "command": "node",
                "args": ["${__dirname}/server/index.js"],
                "env": { "VE_SETTINGS": settings_file.to_string_lossy() }
            }
        },
        // The tools are the running application's, listed when it is asked.
        "tools_generated": true,
        "compatibility": { "platforms": ["darwin", "win32"] }
    })
}

/// Writes the bundle: a zip of the manifest, the bridge and the icon.
pub fn write_bundle(bundle: &Path, settings_file: &Path) -> Result<()> {
    let manifest = serde_json::to_vec_pretty(&manifest(settings_file))?;
    let file = std::fs::File::create(bundle).doing("write", bundle.display())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in [
        ("manifest.json", manifest.as_slice()),
        ("server/index.js", BRIDGE.as_bytes()),
        ("icon.png", ICON),
    ] {
        zip.start_file(name, options)
            .doing("write", bundle.display())?;
        zip.write_all(bytes).doing("write", bundle.display())?;
    }
    zip.finish().doing("write", bundle.display())?;
    Ok(())
}

/// Whether Claude Desktop exists for this platform at all.
pub const fn supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

/// Hands the bundle to Claude Desktop, which shows its own install dialog.
fn open_in_claude_desktop(bundle: &Path) -> Result<()> {
    if launch(bundle)? {
        return Ok(());
    }
    Err(AppError::Doing {
        doing: "open the extension in",
        what: "Claude Desktop".to_owned(),
        why: format!(
            "Claude Desktop does not seem to be installed. The extension is at {}; once Claude Desktop is installed, open that file",
            bundle.display()
        ),
    })
}

/// Whether the platform's opener took the bundle.
///
/// By name rather than by the file's association, so that whatever else has
/// claimed `.mcpb` is not what opens.
#[cfg(target_os = "macos")]
fn launch(bundle: &Path) -> Result<bool> {
    Ok(std::process::Command::new("open")
        .args(["-a", "Claude"])
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .output()
        .doing("run", "open")?
        .status
        .success())
}

/// `start` opens a file with whatever is registered for it; the empty string
/// is the window title it would otherwise take the path for.
#[cfg(windows)]
fn launch(bundle: &Path) -> Result<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Ok(std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(bundle)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .status()
        .doing("run", "cmd")?
        .success())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn launch(_bundle: &Path) -> Result<bool> {
    Err(AppError::Doing {
        doing: "open the extension in",
        what: "Claude Desktop".to_owned(),
        why: "Claude Desktop runs on macOS and Windows".to_owned(),
    })
}

/// Writes the bundle beside the settings and opens it in Claude Desktop.
pub fn register(settings_file: &Path) -> Result<()> {
    if !supported() {
        return launch(Path::new(BUNDLE_NAME)).map(|_| ());
    }
    let bundle = settings_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(BUNDLE_NAME);
    write_bundle(&bundle, settings_file)?;
    open_in_claude_desktop(&bundle)
}
