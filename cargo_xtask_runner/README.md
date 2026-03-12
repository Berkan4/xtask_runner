# cargo-xtask-runner

A graphical task runner for [cargo xtask](https://github.com/matklad/cargo-xtask) workflows.

Instead of typing `cargo xtask <task> <target>` in the terminal, open a GUI that lists all your tasks, lets you check which ones to run, and streams the output to a built-in console — all without leaving your project.

---

## Installation

```bash
cargo install cargo-xtask-runner
```

## Usage

Navigate to any Rust project that has an xtask runner and run:

```bash
cd my-rust-project
cargo xtask-runner
```

The GUI will open. The terminal is free to use for anything else while it runs.

## Requirements

Your project's xtask runner must support a `--list` flag that outputs tasks in this format:

```
target|task_id|description
```

For example:
```
workspace|fmt|Format all code
package|test|Run unit tests
package|build|Build release binary
```

Each line is one task. The `target` field groups tasks in the dropdown — use `workspace` for tasks that apply globally and don't need a target argument.

### Example xtask `--list` implementation

```rust
// in xtask/src/main.rs
if args.contains(&"--list") {
    println!("workspace|fmt|Format all code");
    println!("workspace|clippy|Run clippy lints");
    println!("package|test|Run unit tests");
    println!("package|build|Build release binary");
    return;
}
```

## Features

- **Target selector** — switch between targets via dropdown or arrow buttons
- **Task pipeline** — check multiple tasks and run them in sequence; stops on first failure
- **Per-task Run button** — run a single task without touching the selection
- **Live console** — stdout and stderr streamed in real time with syntax coloring
- **Stop button** — cancel a running pipeline at any time
- **No terminal flicker** — fully windowless on Windows

## How it works

`cargo xtask-runner` runs `cargo xtask --list` to discover available tasks, then invokes each selected task as:

```
cargo xtask <task_id> <target>
```

For workspace-scoped tasks the target argument is omitted.

## Platform support

| Platform | Status |
|----------|--------|
| Windows  | ✅ Full support, no console window |
| Linux    | ✅ Supported |
| macOS    | ✅ Supported |

On Linux and macOS the terminal remains usable after launch. If you want to explicitly background the process:
```bash
cargo xtask-runner &
```

## License

MIT — see [LICENSE](../LICENSE)