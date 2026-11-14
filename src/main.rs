#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    elearnin::run_desktop()
}

#[cfg(target_os = "android")]
fn main() {}
