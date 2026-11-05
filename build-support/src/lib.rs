//! Enforce module source size, purpose comments and shallow control flow at build time without adding browser runtime code.
mod syntax;
use std::{
    fs,
    path::{Path, PathBuf},
};

// Inspect a project's own files and reject size, documentation, layout and guard-clause violations together.
pub fn check(root: &Path) {
    let mut paths: Vec<PathBuf> = ["src", "tests", "build.rs"]
        .map(|path| root.join(path))
        .into();
    let mut errors = Vec::new();
    while let Some(path) = paths.pop() {
        // Some SDK projects have no tests directory until their first private test module is added.
        if !path.exists() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        // Keep native UI and protocol files flat within their respective responsibility boundaries.
        if path.is_dir() {
            for owner in [root.join("src/ui"), root.join("src/developer")] {
                assert!(
                    !path.starts_with(&owner) || path == owner,
                    "{} must remain flat",
                    path.display()
                );
            }
            paths.extend(
                fs::read_dir(path)
                    .expect("Read source directory")
                    .map(|entry| entry.expect("Source entry").path()),
            );
            continue;
        }
        // Assets are owned by packaging rather than Rust source conventions.
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("Read source");
        assert!(
            source
                .lines()
                .next()
                .is_some_and(|line| line.starts_with("//! ")),
            "{} needs module documentation",
            path.display()
        );
        assert!(
            source.lines().count() <= 600,
            "{} exceeds 600 lines",
            path.display()
        );
        let violations = syntax::violations(&source);
        // Report affected owners together so structural work does not require repeated one-file rebuilds.
        if !violations.is_empty() {
            errors.push(format!("{}: {}", path.display(), violations.join("\n")));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n\n"));
}
