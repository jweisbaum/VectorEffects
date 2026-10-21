//! Compile the dependency-free native launcher for the same target as the app.
fn main() {
    println!("cargo:rerun-if-changed=launcher.rs");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let target = std::env::var("TARGET").expect("TARGET");
    let binary = output.join(if target.contains("windows") {
        "whirlwind-launch.exe"
    } else {
        "whirlwind-launch"
    });
    println!(
        "cargo:rustc-env=WHIRLWIND_NATIVE_LAUNCHER={}",
        binary.display()
    );
    let mut rustc = std::process::Command::new(std::env::var_os("RUSTC").expect("RUSTC"));
    rustc
        .args([
            "--edition=2024",
            "--crate-name",
            "whirlwind_launch",
            "--target",
            &target,
            "-C",
            "opt-level=s",
            "-C",
            "strip=symbols",
            "launcher.rs",
            "-o",
        ])
        .arg(binary);
    if let Some(linker) = std::env::var_os(format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_uppercase().replace('-', "_")
    )) {
        rustc
            .arg("-C")
            .arg(format!("linker={}", linker.to_string_lossy()));
    }
    assert!(
        rustc.status().expect("compile native launcher").success(),
        "Native MCP launcher compilation failed"
    );
}
