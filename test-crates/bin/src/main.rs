fn main() {
    std::env::set_var("SO_PRINTLN", "1");
    rubicon::soprintln!("app starting up...");

    let modules = ["../mod_a", "../mod_b"];
    for module in modules {
        let output = std::process::Command::new("cargo")
            .arg("b")
            .env(
                "RUSTFLAGS",
                "-Clink-arg=-undefined -Clink-arg=dynamic_lookup",
            )
            .current_dir(module)
            .output()
            .expect("Failed to execute cargo build");

        if !output.status.success() {
            eprintln!(
                "Error building {}: {}",
                module,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    let lib_a =
        unsafe { libloading::Library::new("../mod_a/target/debug/libmod_a.dylib").unwrap() };
    let init_a: libloading::Symbol<unsafe extern "C" fn()> = unsafe { lib_a.get(b"init").unwrap() };

    let lib_b =
        unsafe { libloading::Library::new("../mod_b/target/debug/libmod_b.dylib").unwrap() };
    let init_b: libloading::Symbol<unsafe extern "C" fn()> = unsafe { lib_b.get(b"init").unwrap() };

    unsafe { init_a() };
    unsafe { init_b() };
}
