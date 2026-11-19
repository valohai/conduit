use std::path::Path;
use std::process::{Command, Output};

use conduit_core::config::{CONFIG_ENV_VAR, DATABASE_ENV_VAR};

/// Runs `conduit` in `dir`, ignoring any config or database set in the developer's shell.
fn conduit_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_conduit"))
        .args(args)
        .current_dir(dir)
        .env_remove(CONFIG_ENV_VAR)
        .env_remove(DATABASE_ENV_VAR)
        .output()
        .expect("failed to run conduit")
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .expect("failed to list directory")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn proxy_without_a_config_leaves_the_directory_empty() {
    let dir = tempfile::tempdir().unwrap();

    let output = conduit_in(dir.path(), &[]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No providers"),
        "{output:?}"
    );
    assert_eq!(files_in(dir.path()), Vec::<String>::new());
}

#[test]
fn proxy_with_an_empty_config_creates_no_database() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("conduit.toml"), "").unwrap();

    let output = conduit_in(dir.path(), &["proxy"]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No providers"),
        "{output:?}"
    );
    assert_eq!(files_in(dir.path()), ["conduit.toml"]);
}

#[test]
fn dashboard_does_not_require_providers() {
    let dir = tempfile::tempdir().unwrap();

    let output = conduit_in(dir.path(), &["dashboard"]);

    // it gets as far as the database, which doesn't exist yet
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to open database"), "{output:?}");
    assert!(!stderr.contains("No providers"), "{output:?}");
    assert_eq!(files_in(dir.path()), Vec::<String>::new());
}
