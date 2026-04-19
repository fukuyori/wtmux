#[cfg(windows)]
fn main() {
    let icon_path = "assets/generated/wtmux.ico";

    println!("cargo:rerun-if-changed={icon_path}");

    if std::path::Path::new(icon_path).exists() {
        let mut res = winres::WindowsResource::new();
        res.set_icon(icon_path);
        res.compile()
            .unwrap_or_else(|err| panic!("failed to compile Windows resources: {err}"));
    } else {
        println!(
            "cargo:warning=Windows icon file not found at {icon_path}; wtmux.exe will be built without an embedded icon"
        );
    }
}

#[cfg(not(windows))]
fn main() {}
