fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "packaging/windows/Elearnin.ico";
        println!("cargo:rerun-if-changed={icon}");
        winresource::WindowsResource::new()
            .set_icon(icon)
            .set("ProductName", "Elearnin")
            .set("FileDescription", "Elearnin offline course library")
            .set("OriginalFilename", "Elearnin.exe")
            .set("CompanyName", "Eng-ReQTeCH")
            .compile()
            .expect("compile Windows icon and version information");
    }
    let style = slint_build::CompilerConfiguration::new().with_style("material".into());
    slint_build::compile_with_config("ui/library.slint", style)
        .expect("compile Elearnin’ interface");
}
