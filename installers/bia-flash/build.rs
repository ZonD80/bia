// Windows targets: embed the app icon (assets/icon.ico, made by tools/make-icons.sh) in
// the .exe. Checks the target, not the build host, so cross builds get it too (they need
// windres or llvm-rc on PATH; without one the build goes on with a warning).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
