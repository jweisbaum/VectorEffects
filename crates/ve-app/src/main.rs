// Suppress the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(err) = ve_app::run() {
        eprintln!("VectorEffects failed to start: {err:#}");
        std::process::exit(1);
    }
}
