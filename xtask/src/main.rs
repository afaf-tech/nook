#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! `cargo xtask <task>`: the gates. Nothing here is user-facing; it exists so the checks are one
//! command, like WordCraft's xtask.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    let task = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());
    let code = match task.as_str() {
        "parity" => parity(),
        "parity-check" => parity_check(),
        "layers" => layers(),
        "test" => run("cargo", &["test", "--workspace"]),
        "clippy" => run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "fmt" => run("cargo", &["fmt", "--all"]),
        "fmt-check" => run("cargo", &["fmt", "--all", "--check"]),
        "build" => run("cargo", &["build", "--workspace"]),
        "ci" => {
            let mut status = 0;
            for args in [
                vec!["fmt", "--all", "--check"],
                vec![
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
                vec!["test", "--workspace"],
            ] {
                if run("cargo", &args) != 0 {
                    status = 1;
                }
            }
            if layers() != 0 {
                status = 1;
            }
            if parity_check() != 0 {
                status = 1;
            }
            status
        }
        _ => {
            println!(
                "cargo xtask <task>\n\n  ci           fmt --check, clippy -D warnings, tests, layers, parity freshness\n  test         the whole test suite\n  clippy       clippy with warnings denied\n  fmt          format every crate\n  parity       regenerate docs/parity-checklist.md
  parity-check fail when the checked-in checklist differs from the generated one\n  layers       enforce the crate layering\n  build        build the workspace"
            );
            0
        }
    };
    std::process::exit(code);
}

fn repo_root() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    while !dir.join("Cargo.toml").is_file() && dir.pop() {}
    dir
}

fn run(program: &str, args: &[&str]) -> i32 {
    println!("$ {program} {}", args.join(" "));
    match Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("could not run {program}: {e}");
            1
        }
    }
}

fn parity() -> i32 {
    let markdown = nook_engine::parity::measure().markdown();
    let path = repo_root().join("docs/parity-checklist.md");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &markdown) {
        Ok(()) => {
            println!("wrote {}", path.display());
            println!("{}", nook_engine::parity::measure().summary());
            0
        }
        Err(e) => {
            eprintln!("could not write {}: {e}", path.display());
            1
        }
    }
}

/// The generated checklist has to match the code, or a feature quietly slipped out.
fn parity_check() -> i32 {
    let markdown = nook_engine::parity::measure().markdown();
    let path = repo_root().join("docs/parity-checklist.md");
    match std::fs::read_to_string(&path) {
        Ok(current) if current == markdown => {
            println!("parity checklist is up to date");
            0
        }
        Ok(_) => {
            eprintln!("docs/parity-checklist.md is stale; run `cargo xtask parity`");
            1
        }
        Err(_) => {
            eprintln!("docs/parity-checklist.md is missing; run `cargo xtask parity`");
            1
        }
    }
}

/// Layering: a crate may only depend on crates below it.
fn layers() -> i32 {
    const ORDER: &[(&str, usize)] = &[
        ("nook-core", 0),
        ("nook-model", 1),
        ("nook-ai", 2),
        ("nook-drivers", 2),
        ("nook-engine", 3),
        ("nook-mcp", 4),
        ("nook-ui", 5),
        ("nook", 6),
        ("nook-cli", 6),
        ("xtask", 7),
    ];
    let mut problems: Vec<String> = Vec::new();
    let mut checked = 0;
    for entry in walk(&repo_root()) {
        let text = match std::fs::read_to_string(&entry) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let package = text
            .lines()
            .find_map(|line| line.strip_prefix("name = "))
            .map(|name| name.trim().trim_matches('"').to_string());
        let Some(package) = package else { continue };
        let Some(level) = ORDER
            .iter()
            .find(|(name, _)| name == &package.as_str())
            .map(|(_, level)| *level)
        else {
            continue;
        };
        checked += 1;
        for line in text.lines() {
            let Some((dependency, _)) = line.split_once(" = {") else {
                continue;
            };
            let dependency = dependency.trim();
            if let Some((name, dep_level)) = ORDER.iter().find(|(name, _)| name == &dependency) {
                if dep_level >= &level && *name != package {
                    problems.push(format!(
                        "{package} (L{level}) depends on {name} (L{dep_level}); dependencies go downward only"
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        println!("layers ok ({checked} crates checked)");
        0
    } else {
        for problem in &problems {
            eprintln!("layer violation: {problem}");
        }
        1
    }
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if matches!(name.as_str(), "target" | ".git" | "plan") {
                    continue;
                }
                stack.push(path);
            } else if path.file_name().map(|n| n == "Cargo.toml").unwrap_or(false) {
                out.push(path);
            }
        }
    }
    out
}
