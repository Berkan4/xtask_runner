use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.contains(&"--list".to_string()) {
        print_task_list();
        return;
    }

    let task = args.first().map(|s| s.as_str()).unwrap_or("");
    let target = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match task {
        "build" => task_build_release_x64_msvc(target),
        "test" => task_test(target),
        "check" => task_check(),
        "clippy" => task_clippy(),
        "sleep" => task_sleep(),
        _ => {
            eprintln!("Unknown task: `{task}`");
            eprintln!("Run with --list to see available tasks.");
            std::process::exit(1);
        }
    }
}

fn print_task_list() {
    // format: target|task_id|description
    println!("cargo-xtask-runner|check|Check formatting and compilation");
    println!("cargo-xtask-runner|clippy|Run clippy lints. Abort on warnings");
    println!("cargo-xtask-runner|build|Build release binary with x64 windows target");
    println!("cargo-xtask-runner|test|Run all tests");
    println!("cargo-xtask-runner|sleep|Sleep 20 seconds to test stop button");
}

fn task_build_release_x64_msvc(target: &str) {
    println!("Starting build task for `{target}`...");
    run(
        "cargo",
        &["build", "--target=x86_64-pc-windows-msvc", "-p", target],
    );
    println!("Finished build task for `{target}`.");
}

fn task_test(target: &str) {
    println!("Starting test task for `{target}`...");
    run("cargo", &["test", "-p", target]);
    println!("Finished test task for `{target}`.");
}

fn task_check() {
    println!("Starting check task...");

    // 1️⃣ Check formatting
    println!("Checking formatting...");
    run("cargo", &["fmt", "--all", "--", "--check"]);
    println!("Formatting is OK.");

    // 2️⃣ Check compilation
    println!("Checking compilation...");
    run("cargo", &["check"]);
    println!("Compilation check passed.");

    println!("Finished check task.");
}

fn task_clippy() {
    println!("Starting clippy task...");
    run(
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    );
    println!("Finished clippy task.");
}

fn task_sleep() {
    println!("Starting sleep task (20 seconds)...");
    std::thread::sleep(Duration::new(20, 0));
    println!("Finished sleep task.");
}

// ─── Helper ───────────────────────────────────────────────────────────────────

fn run(program: &str, args: &[&str]) {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{program}`: {e}"));

    if !status.success() {
        eprintln!("command failed: {program} {}", args.join(" "));
        std::process::exit(1);
    }
}
