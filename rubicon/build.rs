fn main() {
    #[cfg(any(feature = "export-globals", feature = "import-globals"))]
    {
        use std::env;

        // Get the Rust compiler version and set it as an environment variable.
        let rustc_version = rustc_version::version().unwrap();
        println!("cargo:rustc-env=RUBICON_RUSTC_VERSION={}", rustc_version);

        // Pass the target triple.
        let target = env::var("TARGET").unwrap();
        println!("cargo:rustc-env=RUBICON_TARGET_TRIPLE={}", target);
    }
}
