use exports as _;

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
    let lib_a = Box::leak(Box::new(lib_a));
    let init_a: libloading::Symbol<unsafe extern "C" fn()> = unsafe { lib_a.get(b"init").unwrap() };
    let init_a = Box::leak(Box::new(init_a));

    let lib_b =
        unsafe { libloading::Library::new("../mod_b/target/debug/libmod_b.dylib").unwrap() };
    let lib_b = Box::leak(Box::new(lib_b));
    let init_b: libloading::Symbol<unsafe extern "C" fn()> = unsafe { lib_b.get(b"init").unwrap() };
    let init_b = Box::leak(Box::new(init_b));

    unsafe { init_a() };
    unsafe { init_b() };
    unsafe { init_a() };
    unsafe { init_b() };

    rubicon::soprintln!("now doing that in separate threads");

    let mut join_handles = vec![];
    for id in 1..=3 {
        let init_a = &*init_a;
        let init_b = &*init_b;

        let thread_name = format!("worker-{}", id);
        let jh = std::thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                rubicon::soprintln!("in a separate thread named: {}", thread_name);

                unsafe { init_a() };
                unsafe { init_b() };
                unsafe { init_a() };
                unsafe { init_b() };

                id
            })
            .unwrap();
        join_handles.push(jh);
    }

    // join all the threads
    for jh in join_handles {
        let id = jh.join().unwrap();
        rubicon::soprintln!("thread {} joined", id);
    }
}
