//! Open authoritative SQL and validate persisted security query definitions.
use crate::configuration::Configuration;
use sandpost_core::ScopeTree;
use sandpost_storage::Storage;
use sandpost_storage_sqlite::SqliteStorage;
use std::error::Error;
use std::sync::Arc;

/// Open storage without evaluating scopes or constructing message-membership materialization.
pub(crate) async fn initialize(
    configuration: &Configuration,
) -> Result<Arc<dyn Storage>, Box<dyn Error + Send + Sync>> {
    std::fs::create_dir_all(&configuration.data_directory)
        .map_err(sandpost_application::InstanceError::CreateDirectory)?;
    if let Some(parent) = configuration
        .database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let storage: Arc<dyn Storage> =
        Arc::new(SqliteStorage::open(&configuration.database_path).await?);
    let tree = ScopeTree::new(
        storage.list_scopes().await?,
        configuration.maximum_scope_depth,
    )?;
    for scope in tree.scopes() {
        sandpost_query::compile(&scope.filter)?;
    }
    Ok(storage)
}
