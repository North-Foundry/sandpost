//! SQLite pool configuration, asynchronous initialization, and lifecycle.
use crate::{
    AppliedMigration, StorageError,
    error::StorageResult,
    migrations::{self, migrate},
};
use sandpost_storage::StorageHealth;
use sqlx::{
    ConnectOptions, Connection, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous},
};
use std::{path::Path, time::Duration};
use tokio::time::Instant;

/// Bounded default read concurrency; SQLite still permits only one active writer.
const READ_CONNECTIONS: u32 = 4;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// SQLite storage with concurrent WAL readers and one serialized asynchronous writer.
/// In-memory handles share one pool and never acquire a second connection within an operation.
#[derive(Clone)]
pub struct SqliteStorage {
    pub(crate) read_pool: SqlitePool,
    pub(crate) write_pool: SqlitePool,
}

impl SqliteStorage {
    /// Inspect applied batches without running pending migrations or creating a database.
    pub async fn migration_history(
        path: impl AsRef<Path>,
    ) -> Result<Vec<AppliedMigration>, StorageError> {
        let mut connection = connection_options()
            .filename(path)
            .read_only(true)
            .connect()
            .await
            .storage()?;
        let result = migrations::status(&mut connection).await;
        connection.close().await.storage()?;
        result
    }

    /// Revert the last batch without first applying pending scripts. Stop all database users first.
    /// Reversing table or column creation removes their data, as with ordinary down migrations.
    pub async fn rollback_last_migration_batch(
        path: impl AsRef<Path>,
    ) -> Result<Vec<AppliedMigration>, StorageError> {
        let mut connection = connection_options()
            .filename(path)
            .connect()
            .await
            .storage()?;
        let result = migrations::rollback(&mut connection).await;
        connection.close().await.storage()?;
        result
    }

    /// Open or create a file database, validating and migrating it before opening readers.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let options = connection_options().filename(path).create_if_missing(true);
        let write_pool = pool_options(1)
            .connect_with(options.clone())
            .await
            .storage()?;
        let initialize = async {
            let mut connection = write_pool.acquire().await.storage()?;
            enable_write_ahead_logging(&mut connection).await?;
            migrate(&mut connection).await
        }
        .await;
        if let Err(error) = initialize {
            write_pool.close().await;
            return Err(error);
        }
        let read_pool = match pool_options(READ_CONNECTIONS)
            .connect_with(options.create_if_missing(false).read_only(true))
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                write_pool.close().await;
                return Err(crate::error::sqlite_error(error));
            }
        };
        Ok(Self {
            read_pool,
            write_pool,
        })
    }

    /// Create an isolated memory database whose read and write handles share one connection.
    pub async fn memory() -> Result<Self, StorageError> {
        let pool = pool_options(1)
            // Retain the sole connection for the lifetime of the pool. Skipping acquire checks
            // also prevents cancellation during those checks from dropping the memory database.
            .idle_timeout(None)
            .max_lifetime(None)
            .test_before_acquire(false)
            .connect_with(connection_options().in_memory(true))
            .await
            .storage()?;
        let initialize = async {
            let mut connection = pool.acquire().await.storage()?;
            migrate(&mut connection).await
        }
        .await;
        if let Err(error) = initialize {
            pool.close().await;
            return Err(error);
        }
        Ok(Self {
            read_pool: pool.clone(),
            write_pool: pool,
        })
    }

    /// Close both pools, waking waiting callers and waiting for checked-out operations to finish.
    /// Closing any cloned storage handle closes the shared pools for every handle.
    pub async fn close(&self) {
        tokio::join!(self.read_pool.close(), self.write_pool.close());
    }
}

/// Configure every connection with foreign keys, existing durability, and bounded busy handling.
fn connection_options() -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
        .synchronous(SqliteSynchronous::Full)
}

/// Bound connection count and acquisition waiting without adding a custom write scheduler.
fn pool_options(connections: u32) -> SqlitePoolOptions {
    SqlitePoolOptions::new()
        .max_connections(connections)
        .acquire_timeout(BUSY_TIMEOUT)
}

#[async_trait::async_trait]
impl StorageHealth for SqliteStorage {
    /// Probe the read pool without serializing against the writer.
    async fn health(&self) -> Result<(), StorageError> {
        sqlx::query("SELECT 1")
            .execute(&self.read_pool)
            .await
            .storage()?;
        Ok(())
    }
}

/// Initialize WAL once before exposing readers, retrying journal lock upgrades for five seconds.
async fn enable_write_ahead_logging(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    sqlx::query("PRAGMA busy_timeout=0")
        .execute(&mut *connection)
        .await
        .storage()?;
    let deadline = Instant::now() + BUSY_TIMEOUT;
    let result = loop {
        match sqlx::query_scalar::<_, String>("PRAGMA journal_mode=WAL")
            .fetch_one(&mut *connection)
            .await
        {
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => break Ok(()),
            Ok(_) => break Err(StorageError::Unavailable),
            Err(error)
                if error
                    .as_database_error()
                    .and_then(|database| database.code())
                    .and_then(|code| code.parse::<i32>().ok())
                    .is_some_and(|code| code & 255 == 5)
                    && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(error) => break Err(crate::error::sqlite_error(error)),
        }
    };
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(connection)
        .await
        .storage()?;
    result
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
