fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon resource skipped: {e}");
        }
    }
}
