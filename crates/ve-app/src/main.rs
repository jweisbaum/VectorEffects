// Suppress the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        if let Err(error) = ve_app::mcp::native::bridge() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(err) = ve_app::run() {
        eprintln!("VectorEffects failed to start: {err:#}");
        std::process::exit(1);
    }
}
