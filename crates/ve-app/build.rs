//! The Tauri build step, and the Windows application manifest.
//!
//! On Windows the application needs a manifest that selects version 6 of the
//! Common Controls; without one the loader cannot resolve what the dialogs
//! and menus import, and the process dies before `main` with
//! `STATUS_ENTRYPOINT_NOT_FOUND` (0xc0000139). `tauri-build` embeds that
//! manifest as a resource, and a resource is linked into the **binaries
//! only** — so the unit-test and integration-test executables, which link the
//! same code, could not start at all once the MCP service made them reach it.
//!
//! So `tauri-build` is told to leave the manifest out, and the linker embeds
//! the same one into every target instead: binaries, tests and examples.
//! `windows-app-manifest.xml` is `tauri-build`'s default, verbatim. CI checks
//! the built `ve-app.exe` still carries it, since a shipped application
//! without it would not open.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let windows = if windows_msvc {
        tauri_build::WindowsAttributes::new_without_app_manifest()
    } else {
        tauri_build::WindowsAttributes::new()
    };
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))?;

    if windows_msvc {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR")?)
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    Ok(())
}
