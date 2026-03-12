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
        "fmt" => task_fmt(),
        "clippy" => task_clippy(),
        _ => {
            eprintln!("Unknown task: `{task}`");
            eprintln!("Run with --list to see available tasks.");
            std::process::exit(1);
        }
    }
}

fn print_task_list() {
    // format: target|task_id|description
    println!("cargo-xtask-runner|fmt|Format all code");
    println!("cargo-xtask-runner|clippy|Run clippy lints");
    println!("cargo-xtask-runner|build|Build release binary with x64 windows target");
    println!("cargo-xtask-runner|test|Run all tests");
}

fn task_build_release_x64_msvc(target: &str) {
    run(
        "cargo",
        &["build", "--target=x86_64-pc-windows-msvc", "-p", target],
    );
}

fn task_test(target: &str) {
    run("cargo", &["test", "-p", target]);
}

fn task_fmt() {
    run("cargo", &["fmt", "--all"]);
}

fn task_clippy() {
    run(
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    );
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
