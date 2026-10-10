//! Shared SQLite connection, configuration, and async execution boundary.
use crate::StorageError;
use crate::error::StorageResult;
use crate::migrations::migrate;
use rusqlite::Connection;
use sandpost_storage::StorageHealth;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ponytail: one connection-wide lock; split connections only if measured contention warrants it.
/// A fully asynchronous storage backend implemented on top of rusqlite.
///
/// Blocking SQLite work is serialized behind one connection and executed on the Tokio blocking
/// pool, so the public interface never blocks runtime worker threads. The internal lock never
/// leaks: callers only ever observe the backend-neutral storage traits.
#[derive(Clone)]
pub struct SqliteStorage(Arc<Mutex<Connection>>);

impl SqliteStorage {
    /// Open or create a database at the supplied path and apply pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path).storage()?;
        Self::initialize(connection)
    }

    /// Create an in-memory database and apply the current schema.
    pub fn memory() -> Result<Self, StorageError> {
        Self::initialize(Connection::open_in_memory().storage()?)
    }

    /// Configure the connection, migrate its schema, and wrap it for shared access.
    ///
    /// Migration runs with foreign-key enforcement off (SQLite requires that for table rebuilds
    /// and verifies references itself); enforcement is enabled for every later operation.
    fn initialize(mut connection: Connection) -> Result<Self, StorageError> {
        enable_write_ahead_logging(&connection)?;
        connection.busy_timeout(Duration::from_secs(5)).storage()?;
        migrate(&mut connection)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .storage()?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    /// Execute one blocking SQLite operation on the blocking pool.
    ///
    /// The closure runs with the connection lock held for its whole duration, preserving the
    /// single-handle serialization the SQLite schema relies on. Work is never performed on the
    /// async executor.
    pub(crate) async fn run<T, Operation>(&self, operation: Operation) -> Result<T, StorageError>
    where
        T: Send + 'static,
        Operation: FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    {
        let shared = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let mut connection = shared.lock().map_err(|_| StorageError::Unavailable)?;
            operation(&mut connection)
        })
        .await
        .map_err(|error| StorageError::Backend(Box::new(error)))?
    }
}

#[async_trait::async_trait]
impl StorageHealth for SqliteStorage {
    /// Probe the shared SQLite connection through the blocking execution boundary.
    async fn health(&self) -> Result<(), StorageError> {
        self.run(|connection| {
            connection.query_row("SELECT 1", [], |_| Ok(())).storage()?;
            Ok(())
        })
        .await
    }
}

/// Retry WAL setup for at most five seconds when concurrent openers contend.
/// SQLite can skip its busy handler when upgrading journal-mode locks; retry
/// outside any transaction so each failed attempt releases its read lock.
fn enable_write_ahead_logging(connection: &Connection) -> Result<(), StorageError> {
    connection.busy_timeout(Duration::ZERO).storage()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match connection.pragma_update(None, "journal_mode", "WAL") {
            Err(error)
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result.storage(),
        }
    }
}
