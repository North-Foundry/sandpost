//! Parse and run offline maintenance commands without binding listeners.
//!
//! `search status` reads SQL without the writer lock. `search rebuild` and the mutating
//! `migrate` operations take the installation lock that a live runtime holds through its search
//! coordinator, so they refuse to run beside it.
use crate::bootstrap;
use crate::configuration::Configuration;
use crate::lifecycle::RuntimeError;
use sandpost_application::SearchCoordinator;
use sandpost_storage_sqlite::SqliteStorage;

/// Complete command-line usage of the `sandpost` binary.
const USAGE: &str = "usage: sandpost [search rebuild|search status|migrate [up|status|rollback]]";
/// Usage of the `migrate` command and its operations.
const MIGRATION_USAGE: &str = "usage: sandpost migrate [up|status|rollback]";

/// One offline maintenance command selected by command-line arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceCommand {
    /// Report the durable search watermarks.
    SearchStatus,
    /// Rebuild the derived search index from SQL.
    SearchRebuild,
    /// Apply, report, or revert schema migrations.
    Migrate(MigrationOperation),
}

/// The schema migration operation of `sandpost migrate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOperation {
    /// Apply pending scripts as one new batch.
    Up,
    /// Report applied scripts grouped by batch.
    Status,
    /// Atomically revert only the latest batch, without first upgrading.
    Rollback,
}

impl MaintenanceCommand {
    /// Parse arguments after the program name; no arguments select serving the runtime (`None`).
    pub fn parse(arguments: &[String]) -> Result<Option<Self>, RuntimeError> {
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let command = match arguments.as_slice() {
            [] => return Ok(None),
            ["search", "status"] => Self::SearchStatus,
            ["search", "rebuild"] => Self::SearchRebuild,
            ["migrate"] | ["migrate", "up"] => Self::Migrate(MigrationOperation::Up),
            ["migrate", "status"] => Self::Migrate(MigrationOperation::Status),
            ["migrate", "rollback"] => Self::Migrate(MigrationOperation::Rollback),
            ["migrate", ..] => return Err(std::io::Error::other(MIGRATION_USAGE).into()),
            _ => return Err(std::io::Error::other(USAGE).into()),
        };
        Ok(Some(command))
    }

    /// Run the command against the configured installation and print its result to stdout.
    pub async fn run(self, configuration: &Configuration) -> Result<(), RuntimeError> {
        match self {
            Self::SearchStatus | Self::SearchRebuild => {
                run_search_command(configuration, self == Self::SearchRebuild).await
            }
            Self::Migrate(operation) => run_migration_command(configuration, operation).await,
        }
    }
}

/// Report search watermarks, rebuilding the index first under the installation lock if requested.
async fn run_search_command(
    configuration: &Configuration,
    rebuild: bool,
) -> Result<(), RuntimeError> {
    let storage = bootstrap::open_storage(configuration).await?;
    let status = if rebuild {
        SearchCoordinator::open(storage, configuration.data_directory.join("search"))
            .await?
            .rebuild()
            .await?
    } else {
        storage.search_status().await?
    };
    println!(
        "search: latest={} indexed={} pending={}",
        status.latest_sequence, status.indexed_sequence, status.pending_operations
    );
    Ok(())
}

/// Run migration maintenance; mutating operations require the instance to be stopped.
async fn run_migration_command(
    configuration: &Configuration,
    operation: MigrationOperation,
) -> Result<(), RuntimeError> {
    if operation == MigrationOperation::Status {
        let history = SqliteStorage::migration_history(&configuration.database_path).await?;
        let batches = history
            .chunk_by(|left, right| left.batch == right.batch)
            .count();
        println!("migrations: applied={} batches={batches}", history.len());
        for record in history {
            println!("{}\t{}", record.batch, record.migration);
        }
        return Ok(());
    }
    // Held until this function returns, so no runtime can start during the operation.
    let installation_lock =
        SearchCoordinator::open_installation_lock(&configuration.data_directory.join("search"))?;
    installation_lock.try_lock().map_err(|error| {
        std::io::Error::other(format!(
            "stop the running Sandpost instance before migrating: {error}"
        ))
    })?;
    if operation == MigrationOperation::Rollback {
        let reverted =
            SqliteStorage::rollback_last_migration_batch(&configuration.database_path).await?;
        for record in &reverted {
            println!("reverted\t{}\t{}", record.batch, record.migration);
        }
        println!("migrations: reverted={}", reverted.len());
    } else {
        bootstrap::create_storage_directories(configuration)?;
        SqliteStorage::open(&configuration.database_path)
            .await?
            .close()
            .await;
        println!("migrations: up to date");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
