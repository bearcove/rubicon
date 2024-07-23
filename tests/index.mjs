import { spawn, execSync } from "child_process";
import chalk from "chalk";
import os from "os";
import { existsSync } from "fs";
import { dirname, join } from "path";

let ENV_VARS = {};

// Helper function to set environment variables
function setEnvVariables() {
  const rustSysroot = execSync("rustc --print sysroot").toString().trim();
  const rustNightlySysroot = execSync("rustc +nightly --print sysroot")
    .toString()
    .trim();

  const platform = os.platform();
  if (platform === "darwin") {
    console.log("🍎 Detected macOS");
    ENV_VARS.DYLD_LIBRARY_PATH = `${rustSysroot}/lib:${rustNightlySysroot}/lib`;
  } else if (platform === "win32") {
    console.log("🪟 Detected Windows");
    ENV_VARS.PATH += `;${process.env.PATH};${rustSysroot}/lib;${rustNightlySysroot}/lib`;
  } else if (platform === "linux") {
    console.log("🐧 Detected Linux");
    ENV_VARS.LD_LIBRARY_PATH = `${rustSysroot}/lib:${rustNightlySysroot}/lib`;
  } else {
    console.log(`❌ Unsupported platform: ${platform}`);
    process.exit(1);
  }

  console.log("\nEnvironment Variables Summary:");
  for (const [key, value] of Object.entries(ENV_VARS)) {
    console.log(`${key}: ${value}`);
  }
}

// Helper function to run a command and capture output
function runCommand(command) {
  try {
    let env = {
      SOPRINTLN: "1",
      PATH: process.env.PATH,
      ...ENV_VARS,
    };
    console.log("Running with env: ", env);
    const child = spawn(command, [], {
      shell: true,
      stdio: ["inherit", "pipe", "pipe"],
      env,
    });

    let output = "";

    child.stdout.on("data", (data) => {
      process.stdout.write(data);
      output += data;
    });

    child.stderr.on("data", (data) => {
      process.stderr.write(data);
      output += data;
    });

    return new Promise((resolve) => {
      child.on("close", (code) => {
        resolve({
          success: code === 0,
          output: output,
        });
      });
    });
  } catch (error) {
    process.stderr.write(chalk.red(error.toString()));
    return Promise.resolve({
      success: false,
      output: error.toString(),
    });
  }
}

// Helper function to check for feature mismatch
function checkFeatureMismatch(output) {
  return output.includes("feature mismatch for crate");
}

// Test cases
const testCases = [
  {
    name: "Tests pass (debug)",
    buildCommand:
      "cargo build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand: "./test-crates/samplebin/target/debug/samplebin",
    expectedResult: "success",
  },
  {
    name: "Tests pass (release)",
    buildCommand:
      "cargo build --manifest-path test-crates/samplebin/Cargo.toml --release",
    runCommand: "./test-crates/samplebin/target/release/samplebin",
    expectedResult: "success",
  },
  {
    name: "Bin stable, mod_a nightly (should fail)",
    buildCommand:
      "cargo +stable build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --channel:mod_a=nightly",
    expectedResult: "fail",
    checkFeatureMismatch: true,
  },
  {
    name: "Bin nightly, mod_a stable (should fail)",
    buildCommand:
      "cargo +nightly build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --channel:mod_a=stable",
    expectedResult: "fail",
    checkFeatureMismatch: true,
  },
  {
    name: "All nightly (should work)",
    buildCommand:
      "cargo +nightly build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --channel:mod_a=nightly --channel:mod_b=nightly",
    expectedResult: "success",
  },
  {
    name: "Bin has mokio-timer feature (should fail)",
    buildCommand:
      "cargo build --features=exports/mokio-timer --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand: "./test-crates/samplebin/target/debug/samplebin",
    expectedResult: "fail",
    checkFeatureMismatch: true,
  },
  {
    name: "mod_a has mokio-timer feature (should fail)",
    buildCommand:
      "cargo build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --features:mod_a=mokio/timer",
    expectedResult: "fail",
    checkFeatureMismatch: true,
  },
  {
    name: "mod_b has mokio-timer feature (should fail)",
    buildCommand:
      "cargo build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --features:mod_b=mokio/timer",
    expectedResult: "fail",
    checkFeatureMismatch: true,
  },
  {
    name: "all mods have mokio-timer feature (should fail)",
    buildCommand:
      "cargo build --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --features:mod_a=mokio/timer --features:mod_b=mokio/timer",
    expectedResult: "fail",
    checkFeatureMismatch: true,
  },
  {
    name: "bin and mods have mokio-timer feature (should work)",
    buildCommand:
      "cargo build --features=exports/mokio-timer --manifest-path test-crates/samplebin/Cargo.toml",
    runCommand:
      "./test-crates/samplebin/target/debug/samplebin --features:mod_a=mokio/timer --features:mod_b=mokio/timer",
    expectedResult: "success",
  },
];

// Main function to run tests
async function runTests() {
  console.log(chalk.blue("Changing working directory to Git root..."));
  let currentDir = process.cwd();

  while (!existsSync(`${currentDir}/.git`)) {
    const parentDir = dirname(currentDir);
    if (parentDir === currentDir) {
      console.log(chalk.red("Git root not found. Exiting."));
      process.exit(1);
    }
    currentDir = parentDir;
  }
  process.chdir(currentDir);
  console.log(chalk.green(`Changed working directory to: ${currentDir}`));
  console.log(chalk.blue("Checking Rust version and toolchain..."));
  console.log(chalk.yellow("rustc --version:"));
  await runCommand("rustc --version");
  console.log(chalk.yellow("\nrustup which rustc:"));
  await runCommand("rustup which rustc");
  console.log("");

  console.log(chalk.blue("Setting up environment variables..."));
  setEnvVariables();

  console.log(chalk.blue("Installing nightly Rust..."));
  await runCommand("rustup toolchain add nightly");

  console.log(chalk.blue("Running tests..."));
  for (const [index, test] of testCases.entries()) {
    console.log(chalk.yellow(`\nRunning test ${index + 1}: ${test.name}`));

    console.log(chalk.cyan("Building..."));
    const buildResult = await runCommand(test.buildCommand);
    if (!buildResult.success) {
      console.log(chalk.red("Build failed. Exiting tests."));
      process.exit(1);
    }

    console.log(chalk.cyan("Running..."));
    const { success, output } = await runCommand(test.runCommand);

    if (test.expectedResult === "success" && success) {
      console.log(chalk.green("Test passed as expected."));
    } else if (test.expectedResult === "fail" && !success) {
      if (test.checkFeatureMismatch && checkFeatureMismatch(output)) {
        console.log(
          chalk.green("Test failed with feature mismatch as expected."),
        );
      } else {
        console.log(
          chalk.red(
            "Test failed, but not with the expected feature mismatch error.",
          ),
        );
        process.exit(1);
      }
    } else {
      console.log(
        chalk.red(
          `Test result unexpected. Expected ${test.expectedResult}, but got ${success ? "success" : "failure"}.`,
        ),
      );
      process.exit(1);
    }
  }

  console.log(chalk.green("All tests passed successfully."));
}

// Run the tests
runTests().catch((error) => {
  console.error(chalk.red(`An error occurred: ${error.message}`));
  process.exit(1);
});
