//! Exercise offline migration commands through the packaged CLI entry point.
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Own a temporary installation and its explicit isolated configuration file.
struct TestInstallation(PathBuf);

impl TestInstallation {
    /// Create an installation without inheriting the developer's configuration.
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("sandpost-migration-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("config.toml"), "").unwrap();
        Self(directory)
    }
}

impl Drop for TestInstallation {
    /// Remove the isolated installation after CLI processes and lock handles exit.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Execute one CLI migration command with explicit storage paths and a clean environment.
fn run_migration_command(directory: &Path, operation: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sandpost"))
        .env_clear()
        .env("SANDPOST_CONFIG", directory.join("config.toml"))
        .env("SANDPOST_DATA_DIRECTORY", directory)
        .env("SANDPOST_DATABASE_PATH", directory.join("sandpost.sqlite3"))
        .args(["migrate", operation])
        .output()
        .unwrap()
}

/// Report batches, refuse concurrent maintenance, and undo a batch without reapplying it.
#[test]
fn migration_commands_report_batches_and_respect_the_running_instance_lock() {
    let installation = TestInstallation::new();
    let directory = &installation.0;
    assert!(!run_migration_command(directory, "invalid").status.success());
    assert!(!run_migration_command(directory, "status").status.success());
    assert!(!directory.join("sandpost.sqlite3").exists());
    let upgraded = run_migration_command(directory, "up");
    assert!(
        upgraded.status.success(),
        "{}",
        String::from_utf8_lossy(&upgraded.stderr)
    );
    let status = run_migration_command(directory, "status");
    assert!(status.status.success());
    assert!(
        String::from_utf8_lossy(&status.stdout)
            .starts_with("migrations: applied=50 batches=1\n1\t0001_")
    );
    let installation_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("search/writer.lock"))
        .unwrap();
    installation_lock.try_lock().unwrap();
    for operation in ["up", "rollback"] {
        let blocked = run_migration_command(directory, operation);
        assert!(!blocked.status.success());
        assert!(
            String::from_utf8_lossy(&blocked.stderr).contains("stop the running Sandpost instance")
        );
    }
    assert!(run_migration_command(directory, "status").status.success());
    drop(installation_lock);
    let reverted = run_migration_command(directory, "rollback");
    assert!(
        reverted.status.success(),
        "{}",
        String::from_utf8_lossy(&reverted.stderr)
    );
    assert!(String::from_utf8_lossy(&reverted.stdout).contains("migrations: reverted=50"));
    let status = run_migration_command(directory, "status");
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "migrations: applied=0 batches=0\n"
    );
    assert!(
        run_migration_command(directory, "rollback")
            .status
            .success()
    );
    assert!(run_migration_command(directory, "up").status.success());
}
