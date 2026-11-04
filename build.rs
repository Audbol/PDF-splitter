// Embeds the application icon and version info into the Windows executable.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(err) = res.compile() {
            // Don't fail the build if no resource compiler is available.
            println!("cargo:warning=could not embed Windows resources: {err}");
        }
    }
}
