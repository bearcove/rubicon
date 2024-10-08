use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Default)]
struct EnvVars {
    library_search_paths: Vec<String>,
}

impl EnvVars {
    fn new() -> Self {
        EnvVars {
            library_search_paths: Vec::new(),
        }
    }

    fn add_library_path(&mut self, path: String) {
        self.library_search_paths.push(path);
    }

    fn each_kv<F>(&self, mut f: F)
    where
        F: FnMut(&str, &str),
    {
        let platform = env::consts::OS;
        let (env_var, separator) = match platform {
            "macos" => ("DYLD_LIBRARY_PATH", ":"),
            "windows" => ("PATH", ";"),
            "linux" => ("LD_LIBRARY_PATH", ":"),
            _ => {
                eprintln!("❌ Unsupported platform: {}", platform);
                std::process::exit(1);
            }
        };

        let value = self.library_search_paths.join(separator);
        f(env_var, &value);
    }

    fn with_additional_library_path(&self, path: String) -> Self {
        let mut new_env_vars = self.clone();
        new_env_vars.add_library_path(path);
        new_env_vars
    }
}

fn set_env_variables() -> EnvVars {
    let mut env_vars = EnvVars::new();

    let rust_sysroot = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("Failed to execute rustc")
        .stdout;
    let rust_sysroot = String::from_utf8_lossy(&rust_sysroot).trim().to_string();

    let rust_nightly_sysroot = Command::new("rustc")
        .args(["+nightly", "--print", "sysroot"])
        .output()
        .expect("Failed to execute rustc +nightly")
        .stdout;
    let rust_nightly_sysroot = String::from_utf8_lossy(&rust_nightly_sysroot)
        .trim()
        .to_string();

    let platform = env::consts::OS;

    env_vars.add_library_path(format!("{}/lib", rust_sysroot));
    env_vars.add_library_path(format!("{}/lib", rust_nightly_sysroot));

    match platform {
        "macos" | "linux" => {
            // okay
        }
        "windows" => {
            let current_path = env::var("PATH").unwrap_or_default();
            env_vars.add_library_path(current_path);
        }
        _ => {
            eprintln!("❌ Unsupported platform: {}", platform);
            std::process::exit(1);
        }
    }

    println!("\nEnvironment Variables Summary:");
    env_vars.each_kv(|key, value| {
        println!("{}: {}", key, value);
    });

    env_vars
}

fn run_command(command: &[&str], env_vars: &EnvVars) -> io::Result<(bool, String)> {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::thread;

    let program = command[0];
    let args = &command[1..];

    println!("Running command: {} {:?}", program, args);

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    env_vars.each_kv(|key, value| {
        command.env(key, value);
    });

    let mut child = command.spawn()?;

    let (tx_stdout, rx_stdout) = mpsc::channel();
    let (tx_stderr, rx_stderr) = mpsc::channel();

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");

    let stdout_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line.expect("Failed to read line from stdout");
            println!("{}", line);
            tx_stdout.send(line).expect("Failed to send stdout line");
        }
    });

    let stderr_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let line = line.expect("Failed to read line from stderr");
            eprintln!("{}", line);
            tx_stderr.send(line).expect("Failed to send stderr line");
        }
    });

    let mut output = String::new();

    for line in rx_stdout.iter() {
        output.push_str(&line);
        output.push('\n');
    }

    for line in rx_stderr.iter() {
        output.push_str(&line);
        output.push('\n');
    }

    stdout_thread.join().expect("stdout thread panicked");
    stderr_thread.join().expect("stderr thread panicked");

    let status = child.wait()?;
    if !status.success() {
        if let Some(exit_code) = status.code() {
            eprintln!(
                "\n🔍 \x1b[1;90mProcess exited with code {} (0x{:X})\x1b[0m",
                exit_code, exit_code
            );
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = status.signal() {
                    let signal_name = match signal {
                        1 => "SIGHUP",
                        2 => "SIGINT",
                        3 => "SIGQUIT",
                        4 => "SIGILL",
                        6 => "SIGABRT",
                        8 => "SIGFPE",
                        9 => "SIGKILL",
                        11 => "SIGSEGV",
                        13 => "SIGPIPE",
                        14 => "SIGALRM",
                        15 => "SIGTERM",
                        _ => "Unknown",
                    };
                    eprintln!(
                        "\n🔍 \x1b[1;90mProcess terminated by signal {} ({})\x1b[0m",
                        signal, signal_name
                    );
                } else {
                    eprintln!("\n🔍 \x1b[1;90mProcess exited with unknown status\x1b[0m");
                }
            }
            #[cfg(not(unix))]
            {
                eprintln!("\n🔍 \x1b[1;90mProcess exited with unknown status\x1b[0m");
            }
        }
    }
    Ok((status.success(), output))
}

fn check_feature_mismatch(output: &str) -> bool {
    output.contains("Feature mismatch for crate")
}

struct TestCase {
    name: &'static str,
    build_command: &'static [&'static str],
    run_command: &'static [&'static str],
    expected_result: &'static str,
    check_feature_mismatch: bool,
    allowed_to_fail: bool,
}

static TEST_CASES: &[TestCase] = &[
    TestCase {
        name: "Tests pass (debug)",
        build_command: &[
            "cargo",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &["./test-crates/samplebin/target/debug/samplebin"],
        expected_result: "success",
        check_feature_mismatch: false,
        allowed_to_fail: false,
    },
    TestCase {
        name: "Tests pass (release)",
        build_command: &[
            "cargo",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
            "--release",
        ],
        run_command: &["./test-crates/samplebin/target/release/samplebin"],
        expected_result: "success",
        check_feature_mismatch: false,
        allowed_to_fail: false,
    },
    TestCase {
        name: "Bin stable, mod_a nightly (should fail)",
        build_command: &[
            "cargo",
            "+stable",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--channel:mod_a=nightly",
        ],
        expected_result: "fail",
        check_feature_mismatch: true,
        allowed_to_fail: cfg!(target_os = "linux"),
    },
    TestCase {
        name: "Bin nightly, mod_a stable (should fail)",
        build_command: &[
            "cargo",
            "+nightly",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--channel:mod_a=stable",
        ],
        expected_result: "fail",
        check_feature_mismatch: true,
        allowed_to_fail: cfg!(target_os = "linux"),
    },
    TestCase {
        name: "All nightly (should work)",
        build_command: &[
            "cargo",
            "+nightly",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--channel:mod_a=nightly",
            "--channel:mod_b=nightly",
        ],
        expected_result: "success",
        check_feature_mismatch: false,
        allowed_to_fail: false,
    },
    TestCase {
        name: "Bin has mokio-timer feature (should fail)",
        build_command: &[
            "cargo",
            "build",
            "--features=exports/mokio-timer",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &["./test-crates/samplebin/target/debug/samplebin"],
        expected_result: "fail",
        check_feature_mismatch: true,
        allowed_to_fail: false,
    },
    TestCase {
        name: "mod_a has mokio-timer feature (should fail)",
        build_command: &[
            "cargo",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--features:mod_a=mokio/timer",
        ],
        expected_result: "fail",
        check_feature_mismatch: true,
        allowed_to_fail: false,
    },
    TestCase {
        name: "mod_b has mokio-timer feature (should fail)",
        build_command: &[
            "cargo",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--features:mod_b=mokio/timer",
        ],
        expected_result: "fail",
        check_feature_mismatch: true,
        allowed_to_fail: false,
    },
    TestCase {
        name: "all mods have mokio-timer feature (should fail)",
        build_command: &[
            "cargo",
            "build",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--features:mod_a=mokio/timer",
            "--features:mod_b=mokio/timer",
        ],
        expected_result: "fail",
        check_feature_mismatch: true,
        allowed_to_fail: false,
    },
    TestCase {
        name: "bin and mods have mokio-timer feature (should work)",
        build_command: &[
            "cargo",
            "build",
            "--features=exports/mokio-timer",
            "--manifest-path",
            "test-crates/samplebin/Cargo.toml",
        ],
        run_command: &[
            "./test-crates/samplebin/target/debug/samplebin",
            "--features:mod_a=mokio/timer",
            "--features:mod_b=mokio/timer",
        ],
        expected_result: "success",
        check_feature_mismatch: false,
        allowed_to_fail: false,
    },
];

fn run_tests() -> io::Result<()> {
    println!("\n🚀 \x1b[1;36mChanging working directory to Git root...\x1b[0m");
    let mut git_root = env::current_dir()?;

    while !Path::new(&git_root).join(".git").exists() {
        if let Some(parent) = git_root.parent() {
            git_root = parent.to_path_buf();
        } else {
            eprintln!("❌ \x1b[1;31mGit root not found. Exiting.\x1b[0m");
            std::process::exit(1);
        }
    }

    env::set_current_dir(&git_root)?;
    println!(
        "📂 \x1b[1;32mChanged working directory to:\x1b[0m {}",
        git_root.display()
    );

    println!("🌟 \x1b[1;36mSetting up environment variables...\x1b[0m");
    let env_vars = set_env_variables();

    println!("🌙 \x1b[1;34mInstalling nightly Rust...\x1b[0m");
    run_command(&["rustup", "toolchain", "add", "nightly"], &env_vars)?;

    println!("\n🧪 \x1b[1;35mRunning tests...\x1b[0m");

    for (index, test) in TEST_CASES.iter().enumerate() {
        {
            let test_info = format!("Running test {}: {}", index + 1, test.name);
            let box_width = test_info.chars().count() + 4;
            let padding = box_width - 2 - test_info.chars().count();
            let left_padding = padding / 2;
            let right_padding = padding - left_padding;

            println!("\n\x1b[1;33m╔{}╗\x1b[0m", "═".repeat(box_width - 2));
            println!(
                "\x1b[1;33m║\x1b[0m{}\x1b[1;36m{}\x1b[0m{}\x1b[1;33m║\x1b[0m",
                " ".repeat(left_padding),
                test_info,
                " ".repeat(right_padding),
            );
            println!("\x1b[1;33m╚{}╝\x1b[0m", "═".repeat(box_width - 2));
        }

        println!("🏗️  \x1b[1;34mBuilding...\x1b[0m");
        let build_result = run_command(test.build_command, &Default::default())?;
        if !build_result.0 {
            eprintln!("❌ \x1b[1;31mBuild failed. Exiting tests.\x1b[0m");
            std::process::exit(1);
        }

        println!("▶️  \x1b[1;32mRunning...\x1b[0m");
        let profile = if test.build_command.contains(&"--release") {
            "release"
        } else {
            "debug"
        };
        let additional_path = git_root
            .join("test-crates")
            .join("samplebin")
            .join("target")
            .join(profile);
        let env_vars =
            env_vars.with_additional_library_path(additional_path.to_string_lossy().into_owned());

        let (success, output) = run_command(test.run_command, &env_vars)?;

        match (test.expected_result, success) {
            ("success", true) => println!("✅ \x1b[1;32mTest passed as expected.\x1b[0m"),
            ("fail", false) if test.check_feature_mismatch && check_feature_mismatch(&output) => {
                println!("✅ \x1b[1;33mTest failed with feature mismatch as expected.\x1b[0m")
            }
            ("fail", false) if test.check_feature_mismatch => {
                eprintln!("❌ \x1b[1;31mTest failed, but not with the expected feature mismatch error.\x1b[0m");
                if test.allowed_to_fail || cfg!(windows) {
                    println!("⚠️ \x1b[1;33mTest was allowed to fail.\x1b[0m");
                } else {
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!(
                    "❌ \x1b[1;31mTest result unexpected. Expected {}, but got {}.\x1b[0m",
                    test.expected_result,
                    if success { "success" } else { "failure" }
                );
                if test.allowed_to_fail {
                    println!("⚠️ \x1b[1;33mTest was allowed to fail.\x1b[0m");
                } else {
                    std::process::exit(1);
                }
            }
        }
    }

    println!("\n🎉 \x1b[1;32mAll tests passed successfully.\x1b[0m");
    Ok(())
}

fn main() -> io::Result<()> {
    run_tests()
}
