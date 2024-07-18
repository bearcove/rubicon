use std::sync::atomic::Ordering;

use exports::{self as _, mokio};
use soprintln::soprintln;

fn main() {
    soprintln::init!();
    let exe_path = std::env::current_exe().expect("Failed to get current exe path");
    let project_root = exe_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    std::env::set_current_dir(project_root).expect("Failed to change directory");

    soprintln!("app starting up...");

    let modules = ["../mod_a", "../mod_b"];
    for module in modules {
        soprintln!("building {module}");

        #[cfg(target_os = "macos")]
        let rustflags = "-Clink-arg=-undefined -Clink-arg=dynamic_lookup";
        #[cfg(not(target_os = "macos"))]
        let rustflags = "";

        let output = std::process::Command::new("cargo")
            .arg("b")
            .env("RUSTFLAGS", rustflags)
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

    fn module_path(name: &str) -> String {
        #[cfg(target_os = "windows")]
        let prefix = "";
        #[cfg(not(target_os = "windows"))]
        let prefix = "lib";

        #[cfg(target_os = "windows")]
        let extension = "dll";
        #[cfg(target_os = "macos")]
        let extension = "dylib";
        #[cfg(target_os = "linux")]
        let extension = "so";

        format!(
            "../mod_{}/target/debug/{}mod_{}.{}",
            name, prefix, name, extension
        )
    }

    soprintln!("loading modules...");
    let lib_a = unsafe { libloading::Library::new(module_path("a")).unwrap() };
    let lib_a = Box::leak(Box::new(lib_a));
    let init_a: libloading::Symbol<unsafe extern "C" fn()> = unsafe { lib_a.get(b"init").unwrap() };
    let init_a = Box::leak(Box::new(init_a));

    let lib_b = unsafe { libloading::Library::new(module_path("b")).unwrap() };
    let lib_b = Box::leak(Box::new(lib_b));
    let init_b: libloading::Symbol<unsafe extern "C" fn()> = unsafe { lib_b.get(b"init").unwrap() };
    let init_b = Box::leak(Box::new(init_b));

    soprintln!("DANGEROUS is now {}", unsafe {
        mokio::DANGEROUS += 1;
        mokio::DANGEROUS
    });

    soprintln!(
        "PL1 = {}, TL1 = {} (initial)",
        mokio::MOKIO_PL1.load(Ordering::Relaxed),
        mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)),
    );

    for _ in 0..2 {
        unsafe { init_a() };
        soprintln!(
            "PL1 = {}, TL1 = {} (after init_a)",
            mokio::MOKIO_PL1.load(Ordering::Relaxed),
            mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)),
        );

        unsafe { init_b() };
        soprintln!(
            "PL1 = {}, TL1 = {} (after init_b)",
            mokio::MOKIO_PL1.load(Ordering::Relaxed),
            mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)),
        );
    }

    soprintln!("now starting a couple threads");

    let mut join_handles = vec![];
    for id in 1..=3 {
        let init_a = &*init_a;
        let init_b = &*init_b;

        let thread_name = format!("worker-{}", id);
        let jh = std::thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                soprintln!("in a separate thread named: {}", thread_name);

                soprintln!(
                    "PL1 = {}, TL1 = {} (initial)",
                    mokio::MOKIO_PL1.load(Ordering::Relaxed),
                    mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)),
                );

                for _ in 0..2 {
                    unsafe { init_a() };
                    soprintln!(
                        "PL1 = {}, TL1 = {} (after init_a)",
                        mokio::MOKIO_PL1.load(Ordering::Relaxed),
                        mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)),
                    );

                    unsafe { init_b() };
                    soprintln!(
                        "PL1 = {}, TL1 = {} (after init_b)",
                        mokio::MOKIO_PL1.load(Ordering::Relaxed),
                        mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)),
                    );
                }

                // TL1 should be 4 (incremented by each `init_X()` call)
                assert_eq!(mokio::MOKIO_TL1.with(|s| s.load(Ordering::Relaxed)), 4);

                id
            })
            .unwrap();
        join_handles.push(jh);
    }

    // join all the threads
    for jh in join_handles {
        let id = jh.join().unwrap();
        soprintln!("thread {} joined", id);
    }

    // PL1 should be exactly 16
    // 2 per turn, 2 turns on the main thread, 2 turns on each of the 3 worker threads: 16 total
    assert_eq!(mokio::MOKIO_PL1.load(Ordering::Relaxed), 16);

    // DANGEROUS should be between 1 and 20
    assert!(unsafe { mokio::DANGEROUS } >= 1 && unsafe { mokio::DANGEROUS } <= 20);
}
