fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The static SimConnect library calls the Windows Registry APIs, but
        // the upstream msfs-rs build script does not link their import library.
        println!("cargo:rustc-link-lib=advapi32");
    }
}
