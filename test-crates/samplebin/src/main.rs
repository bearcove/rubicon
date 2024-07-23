use std::sync::atomic::Ordering;

use exports::{self as _, mokio};
use soprintln::soprintln;

fn main() {
    struct ModuleSpec {
        name: &'static str,
        channel: String,
        features: Vec<String>,
    }

    let mut modules = [
        ModuleSpec {
            name: "mod_a",
            channel: "stable".to_string(),
            features: Default::default(),
        },
        ModuleSpec {
            name: "mod_b",
            channel: "stable".to_string(),
            features: Default::default(),
        },
    ];

    for arg in std::env::args().skip(1) {
        if let Some(rest) = arg.strip_prefix("--features:") {
            let parts: Vec<&str> = rest.splitn(2, '=').collect();
            if parts.len() != 2 {
                panic!("Invalid argument format: expected --features:module=feature1,feature2");
            }
            let mod_name = parts[0];
            let features = parts[1].split(',').map(|s| s.to_owned());
            let module = modules
                .iter_mut()
                .find(|m| m.name == mod_name)
                .unwrap_or_else(|| panic!("Unknown module: {}", mod_name));

            for feature in features {
                module.features.push(feature);
            }
        } else if let Some(rest) = arg.strip_prefix("--channel:") {
            let parts: Vec<&str> = rest.splitn(2, '=').collect();
            if parts.len() != 2 {
                panic!("Invalid argument format: expected --channel:module=(stable|nightly)");
            }
            let mod_name = parts[0];
            let channel = parts[1];
            if channel != "stable" && channel != "nightly" {
                panic!(
                    "Invalid channel: {}. Expected 'stable' or 'nightly'",
                    channel
                );
            }
            let module = modules
                .iter_mut()
                .find(|m| m.name == mod_name)
                .unwrap_or_else(|| panic!("Unknown module: {}", mod_name));
            module.channel = channel.to_string();
        } else {
            panic!("Unknown argument: {}", arg);
        }
    }

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

    for module in modules {
        soprintln!(
            "building {} with features {:?}",
            module.name,
            module.features.join(", ")
        );

        cfg_if::cfg_if! {
            if #[cfg(target_os = "macos")] {
                let rustflags = "-Clink-arg=-undefined -Clink-arg=dynamic_lookup";
            } else if #[cfg(target_os = "windows")] {
                let rustflags = "-Clink-arg=/FORCE:UNRESOLVED";
            } else {
                let rustflags = "";
            }
        }

        let mut cmd = std::process::Command::new("cargo");
        cmd.arg(format!("+{}", module.channel))
            .arg("build")
            .env("RUSTFLAGS", rustflags)
            .current_dir(format!("../{}", module.name));
        if !module.features.is_empty() {
            cmd.arg("--features").arg(module.features.join(","));
        }

        let output = cmd.output().expect("Failed to execute cargo build");

        if !output.status.success() {
            eprintln!(
                "Error building {}: {}",
                module.name,
                String::from_utf8_lossy(&output.stderr)
            );
            std::process::exit(1);
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

    // same for DANGEROUS, it's just guarded by a mutex internally
    assert_eq!(mokio::get_dangerous(), 16);
}
