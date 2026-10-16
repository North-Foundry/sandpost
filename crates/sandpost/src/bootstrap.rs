//! Open authoritative SQL and validate persisted security query definitions.
use crate::configuration::Configuration;
use crate::lifecycle::RuntimeError;
use sandpost_core::ScopeTree;
use sandpost_storage::Storage;
use sandpost_storage_sqlite::SqliteStorage;
use std::sync::Arc;

/// Open storage without evaluating scopes or constructing message-membership materialization.
pub(crate) async fn initialize(
    configuration: &Configuration,
) -> Result<Arc<dyn Storage>, RuntimeError> {
    let storage = open_storage(configuration).await?;
    let tree = ScopeTree::new(
        storage.list_scopes().await?,
        configuration.maximum_scope_depth,
    )?;
    for scope in tree.scopes() {
        sandpost_query::compile(&scope.filter)?;
    }
    Ok(storage)
}

/// Create the storage directories, then open and migrate the selected SQLite backend.
pub(crate) async fn open_storage(
    configuration: &Configuration,
) -> Result<Arc<dyn Storage>, RuntimeError> {
    create_storage_directories(configuration)?;
    Ok(Arc::new(
        SqliteStorage::open(&configuration.database_path).await?,
    ))
}

/// Create the data directory and the database file's parent directory when they are missing.
pub(crate) fn create_storage_directories(
    configuration: &Configuration,
) -> Result<(), RuntimeError> {
    std::fs::create_dir_all(&configuration.data_directory)
        .map_err(sandpost_application::InstanceError::CreateDirectory)?;
    if let Some(parent) = configuration
        .database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
