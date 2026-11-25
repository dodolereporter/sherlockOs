//! Every program of the workspace that draws with iced chooses its renderer first thing in
//! `main` (`gpu::Choice`, ADR 0018). sherlock-ui compiles iced with wgpu, and Cargo turns that
//! on for every program that depends on it: without the choice, iced starts wgpu on the most
//! powerful adapter and wakes a dedicated NVIDIA card. A new program that forgets it fails here.
//!
//! The crates are found by Cargo itself: a member whose graph (normal and dev dependencies,
//! with its own features) holds `iced_wgpu`, directly or through another crate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The members of the workspace, from the root `Cargo.toml` (`manifest`).
fn members(root: &Path, manifest: &str) -> Vec<PathBuf> {
    let list = manifest
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map_or("", |(list, _)| list);
    list.split('"')
        .skip(1)
        .step_by(2)
        .map(|member| root.join(member))
        .collect()
}

/// The `name` of the package, in its `Cargo.toml`.
fn package_name(manifest: &str) -> Option<&str> {
    manifest
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("name = \""))
        .and_then(|rest| rest.split('"').next())
}

/// Whether `iced_wgpu` is in the dependency graph of `package`, resolved for that package
/// alone (`cargo tree --invert` fails when the crate is not in the graph).
fn reaches_iced_wgpu(root: &Path, package: &str) -> Result<bool, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["tree", "--offline", "--locked", "--edges", "normal,dev"])
        .args([
            "--package",
            package,
            "--invert",
            "iced_wgpu",
            "--prefix",
            "none",
        ])
        .output()
        .map_err(|error| format!("cargo tree: {error}"))?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("did not match any packages") {
        Ok(false)
    } else {
        Err(format!("cargo tree -p {package}: {stderr}"))
    }
}

/// The files holding a `main`: `src/main.rs`, `src/bin/*.rs`, `examples/*.rs`,
/// `examples/*/main.rs`, and the `path` of `[[bin]]` and `[[example]]` sections.
fn programs(member: &Path, manifest: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let main = member.join("src/main.rs");
    if main.exists() {
        found.push(main);
    }
    for dir in ["src/bin", "examples"] {
        let Ok(entries) = fs::read_dir(member.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            } else if path.join("main.rs").exists() {
                found.push(path.join("main.rs"));
            }
        }
    }
    let mut section = "";
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if (section == "[[bin]]" || section == "[[example]]")
            && let Some(path) = line.strip_prefix("path = \"")
            && let Some(path) = path.split('"').next()
        {
            let path = member.join(path);
            if !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// Code lines, without blank lines and comments.
fn statements(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
}

/// `main` carries no attribute (no `#[tokio::main]`), its first statement is the choice and
/// the next one applies it.
fn chooses_first(source: &str) -> Result<(), &'static str> {
    let lines: Vec<&str> = source.lines().collect();
    let at = lines
        .iter()
        .position(|line| {
            let line = line.trim_start();
            line.starts_with("fn main(") || line.starts_with("pub fn main(")
        })
        .ok_or("no `fn main`")?;
    let before = lines[..at].join("\n");
    if statements(&before)
        .last()
        .is_some_and(|line| line.starts_with("#["))
    {
        return Err("`main` carries an attribute (`#[tokio::main]` starts threads before it)");
    }
    let body = lines[at + 1..].join("\n");
    let mut body = statements(&body);
    if !body
        .next()
        .is_some_and(|line| line.contains("gpu::Choice::detect()"))
    {
        return Err("the first statement is not `gpu::Choice::detect()`");
    }
    if !body.next().is_some_and(|line| line.contains(".apply()")) {
        return Err("the statement after `detect()` is not `apply()`");
    }
    Ok(())
}

#[test]
fn every_iced_program_chooses_its_renderer_first() {
    let root = root();
    let workspace = fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");
    let mut checked = Vec::new();
    for member in members(&root, &workspace) {
        let manifest = fs::read_to_string(member.join("Cargo.toml")).expect("member Cargo.toml");
        let package = package_name(&manifest).expect("package name");
        if !reaches_iced_wgpu(&root, package).expect("cargo tree") {
            continue;
        }
        for program in programs(&member, &manifest) {
            let source = fs::read_to_string(&program).expect("program source");
            if let Err(why) = chooses_first(&source) {
                panic!("{}: {why} (ADR 0018)", program.display());
            }
            checked.push(program);
        }
    }
    // The bar, its backdrop, the dock, Evidence and the gallery at least.
    assert!(checked.len() >= 5, "only {checked:?}");
}

/// Source text from its lines.
fn code(lines: &[&str]) -> String {
    lines.join("\n")
}

#[test]
fn the_check_catches_a_program_that_forgets() {
    let detect = "    let c = sherlock_ui::gpu::Choice::detect();";
    let apply = "    unsafe { c.apply() };";
    let good = code(&[
        "fn main() {",
        "    // Before anything else.",
        detect,
        "    // SAFETY: first.",
        apply,
        "    run();",
        "}",
    ]);
    assert_eq!(chooses_first(&good), Ok(()));
    let forgets = code(&[
        "fn main() -> iced::Result {",
        "    iced::application(a, b, c)",
        "}",
    ]);
    assert!(chooses_first(&forgets).is_err());
    let late = code(&["fn main() {", "    init_logs();", detect, apply, "}"]);
    assert!(chooses_first(&late).is_err());
    // A thread started between the choice and `apply`.
    let thread = code(&[
        "fn main() {",
        detect,
        "    std::thread::spawn(work);",
        apply,
        "}",
    ]);
    assert!(chooses_first(&thread).is_err());
    // No `apply` in `main`, one further down the file.
    let elsewhere = code(&[
        "fn main() {",
        detect,
        "    run(c);",
        "}",
        "fn run(c: Choice) {",
        apply,
        "}",
    ]);
    assert!(chooses_first(&elsewhere).is_err());
    let tokio = code(&["#[tokio::main]", "async fn main() {}"]);
    assert!(chooses_first(&tokio).is_err());
    let tokio = code(&["#[tokio::main]", "fn main() {", detect, apply, "}"]);
    assert!(chooses_first(&tokio).is_err());
}

#[test]
fn bins_and_examples_declared_in_the_manifest_are_found() {
    let manifest = code(&[
        "[package]",
        "name = \"x\"",
        "[lib]",
        "path = \"src/lib.rs\"",
        "[[bin]]",
        "name = \"tool\"",
        "path = \"src/tool.rs\"",
    ]);
    let found = programs(Path::new("/nowhere"), &manifest);
    assert_eq!(found, vec![PathBuf::from("/nowhere/src/tool.rs")]);
    assert_eq!(package_name(&manifest), Some("x"));
}
