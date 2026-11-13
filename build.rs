fn main() {
    let style = slint_build::CompilerConfiguration::new().with_style("material".into());
    slint_build::compile_with_config("ui/library.slint", style)
        .expect("compile Elearnin’ interface");
}
