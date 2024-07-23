use std::collections::HashMap;
use std::env;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

struct EnvVars {
    vars: HashMap<String, String>,
}

impl EnvVars {
    fn new() -> Self {
        EnvVars {
            vars: HashMap::new(),
        }
    }

    fn set(&mut self, key: &str, value: String) {
        self.vars.insert(key.to_string(), value);
    }
}

fn set_env_variables(git_root: &Path) -> EnvVars {
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
    let debug_lib_path = git_root.join("test-crates/samplebin/target/debug");

    match platform {
        "macos" => {
            println!("🍎 Detected macOS");
            env_vars.set(
                "DYLD_LIBRARY_PATH",
                format!("{}/lib:{}/lib", rust_sysroot, rust_nightly_sysroot),
            );
        }
        "windows" => {
            println!("🪟 Detected Windows");
            let current_path = env::var("PATH").unwrap_or_default();
            env_vars.set(
                "PATH",
                format!(
                    "{};{}/lib;{}/lib;{}",
                    current_path,
                    rust_sysroot,
                    rust_nightly_sysroot,
                    debug_lib_path.display()
                ),
            );
        }
        "linux" => {
            println!("🐧 Detected Linux");
            env_vars.set(
                "LD_LIBRARY_PATH",
                format!(
                    "{}/lib:{}/lib:{}",
                    rust_sysroot,
                    rust_nightly_sysroot,
                    debug_lib_path.display()
                ),
            );
        }
        _ => {
            eprintln!("❌ Unsupported platform: {}", platform);
            std::process::exit(1);
        }
    }

    println!("\nEnvironment Variables Summary:");
    for (key, value) in &env_vars.vars {
        println!("{}: {}", key, value);
    }

    env_vars
}

fn run_command(command: &[&str], env_vars: &EnvVars) -> io::Result<(bool, String)> {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::thread;

    let program = command[0];
    let args = &command[1..];

    println!("Running command: {} {:?}", program, args);

    let mut child = Command::new(program)
        .args(args)
        .envs(&env_vars.vars)
        .env("PATH", std::env::var("PATH").unwrap())
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

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
    let env_vars = set_env_variables(&git_root);

    println!("🌙 \x1b[1;34mInstalling nightly Rust...\x1b[0m");
    run_command(&["rustup", "toolchain", "add", "nightly"], &env_vars)?;

    println!("\n🧪 \x1b[1;35mRunning tests...\x1b[0m");

    for (index, test) in TEST_CASES.iter().enumerate() {
        println!("\n\x1b[1;33m╔═════════════════════════════════════════════════════════════════════════════╗\x1b[0m");
        println!(
            "\x1b[1;33m║\x1b[0m 🎉🔬 \x1b[1;36mRunning test {}: {}\x1b[0m",
            index + 1,
            test.name
        );
        println!("\x1b[1;33m╚═════════════════════════════════════════════════════════════════════════════╝\x1b[0m");

        println!("🏗️  \x1b[1;34mBuilding...\x1b[0m");
        let build_result = run_command(test.build_command, &env_vars)?;
        if !build_result.0 {
            eprintln!("❌ \x1b[1;31mBuild failed. Exiting tests.\x1b[0m");
            std::process::exit(1);
        }

        println!("▶️  \x1b[1;32mRunning...\x1b[0m");
        let (success, output) = run_command(test.run_command, &env_vars)?;

        match (test.expected_result, success) {
            ("success", true) => println!("✅ \x1b[1;32mTest passed as expected.\x1b[0m"),
            ("fail", false) if test.check_feature_mismatch && check_feature_mismatch(&output) => {
                println!("✅ \x1b[1;33mTest failed with feature mismatch as expected.\x1b[0m")
            }
            ("fail", false) if test.check_feature_mismatch => {
                eprintln!("❌ \x1b[1;31mTest failed, but not with the expected feature mismatch error.\x1b[0m");
                std::process::exit(1);
            }
            _ => {
                eprintln!(
                    "❌ \x1b[1;31mTest result unexpected. Expected {}, but got {}.\x1b[0m",
                    test.expected_result,
                    if success { "success" } else { "failure" }
                );
                std::process::exit(1);
            }
        }
    }

    println!("\n🎉 \x1b[1;32mAll tests passed successfully.\x1b[0m");
    Ok(())
}

fn main() -> io::Result<()> {
    run_tests()
}
