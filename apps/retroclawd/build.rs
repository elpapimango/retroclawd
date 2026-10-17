use std::env;

fn main() {
    // Version, build number and git commit for `--version`, the help and the overlay (`src/version.rs`).
    build_info::emit();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon.ico");
        res.compile()
            .expect("failed to embed Windows icon resource");
    }
}
