//! Shared SQLite connection, configuration, and health checks.
use crate::{StorageError, migrations::migrate};
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

// ponytail: one connection-wide lock; keep calls on spawn_blocking and split connections only if contention matters.
#[derive(Clone)]
pub struct Storage(Arc<Mutex<Connection>>);

impl Storage {
    /// Open or create a database at the supplied path and apply pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    /// Create an in-memory database and apply the current schema.
    pub fn memory() -> Result<Self, StorageError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    /// Configure the connection, migrate its schema, and wrap it for shared access.
    fn initialize(mut connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&mut connection)?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    /// Lock the shared database connection, reporting a poisoned lock as a storage error.
    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.0.lock().map_err(|_| StorageError::LockPoisoned)
    }

    /// Check that the database can execute a trivial query.
    pub fn health(&self) -> Result<(), StorageError> {
        self.connection()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }
}
