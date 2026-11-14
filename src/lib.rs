mod app;
mod bue;
mod library;
mod platform;

#[cfg(not(target_os = "android"))]
pub fn run_desktop() -> anyhow::Result<()> {
    use anyhow::Context;
    let directories = directories::ProjectDirs::from("", "", "Elearnin")
        .context("Could not locate your application data directory")?;
    app::run(directories.data_local_dir().to_owned())
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(activity: slint::android::AndroidApp) {
    let result = (|| -> anyhow::Result<()> {
        let root = platform::init(&activity)?;
        slint::android::init(activity)?;
        app::run(root)
    })();
    if let Err(error) = result {
        eprintln!("Elearnin startup failed: {error}");
    }
}
