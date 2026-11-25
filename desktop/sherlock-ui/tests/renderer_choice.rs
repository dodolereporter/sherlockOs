//! Every program of the workspace that draws with iced chooses its renderer first thing in
//! `main` (`gpu::Choice`, ADR 0018). sherlock-ui compiles iced with wgpu, and Cargo turns that
//! on for every program that depends on it: without the choice, iced starts wgpu on the most
//! powerful adapter and wakes a dedicated NVIDIA card. A new program that forgets it fails here.

use std::fs;
use std::path::{Path, PathBuf};

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

/// The crate draws with iced: iced itself, the layer-shell crate, or sherlock-ui with its
/// default `iced` feature.
fn draws_with_iced(manifest: &str) -> bool {
    manifest.lines().map(str::trim).any(|line| {
        line.starts_with("iced =")
            || line.starts_with("iced_exwlshell =")
            || (line.starts_with("sherlock-ui =") && !line.contains("default-features = false"))
    })
}

/// The files holding a `main`: `src/main.rs`, `src/bin/*.rs`, `examples/*.rs` and
/// `examples/*/main.rs`.
fn programs(member: &Path) -> Vec<PathBuf> {
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
    found
}

/// The first statement of `main` is the choice, and `apply` follows it.
fn chooses_first(source: &str) -> bool {
    let Some((_, body)) = source.split_once("fn main(") else {
        return false;
    };
    let mut statements = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"));
    statements
        .next()
        .is_some_and(|line| line.contains("gpu::Choice::detect()"))
        && body.contains(".apply()")
}

#[test]
fn every_iced_program_chooses_its_renderer_first() {
    let root = root();
    let workspace = fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");
    let mut checked = Vec::new();
    for member in members(&root, &workspace) {
        let manifest = fs::read_to_string(member.join("Cargo.toml")).expect("member Cargo.toml");
        if !draws_with_iced(&manifest) {
            continue;
        }
        for program in programs(&member) {
            let source = fs::read_to_string(&program).expect("program source");
            assert!(
                chooses_first(&source),
                "{} must start `main` with `gpu::Choice::detect()` and `apply()` (ADR 0018)",
                program.display()
            );
            checked.push(program);
        }
    }
    // The bar, its backdrop, the dock, Evidence and the gallery at least: the walk found them.
    assert!(checked.len() >= 5, "only {checked:?}");
}

#[test]
fn the_check_catches_a_program_that_forgets() {
    let forgets =
        "fn main() -> iced::Result {\n    let app = iced::application(new, update, view);\n}\n";
    assert!(!chooses_first(forgets));
    let late = "fn main() {\n    tracing_subscriber::fmt().init();\n    let c = sherlock_ui::gpu::Choice::detect();\n    unsafe { c.apply() };\n}\n";
    assert!(!chooses_first(late));
    let good = "fn main() {\n    // Before anything else.\n    let c = sherlock_ui::gpu::Choice::detect();\n    // SAFETY: first.\n    unsafe { c.apply() };\n}\n";
    assert!(chooses_first(good));
    assert!(draws_with_iced(
        "sherlock-ui = { path = \"../sherlock-ui\" }"
    ));
    assert!(!draws_with_iced(
        "sherlock-ui = { path = \"../sherlock-ui\", default-features = false }"
    ));
}
