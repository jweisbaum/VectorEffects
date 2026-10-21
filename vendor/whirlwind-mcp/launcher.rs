//! Tiny native plugin launcher. Built into the app; no shell or language runtime is required.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use std::process::{Command, ExitCode, Stdio};
fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(app) = args.next() else {
        eprintln!("Missing installed application path");
        return ExitCode::FAILURE;
    };
    let mut command = Command::new(app);
    command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(error) => {
            eprintln!(
                "Could not start the app's MCP bridge: {error}. Repair the connection in app Settings."
            );
            ExitCode::FAILURE
        }
    }
}
