//! Initialize persistent storage and the immutable matching policy snapshot.

use crate::configuration::Configuration;
use sandpost_core::{Scope, ScopeIdentifier, ScopeTree};
use sandpost_match::Matcher;
use sandpost_storage::Storage;
use std::{error::Error, sync::Arc};

/// Open storage, seed an empty installation, and compile its persisted scope policies.
pub(crate) fn initialize(
    application_configuration: &Configuration,
) -> Result<(Storage, Arc<Matcher>), Box<dyn Error + Send + Sync>> {
    std::fs::create_dir_all(&application_configuration.data_directory)?;
    if let Some(parent) = application_configuration
        .database_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let storage = Storage::open(&application_configuration.database_path)?;
    let mut scopes = storage.load_scopes()?;
    if scopes.is_empty() {
        let root_scope = Scope {
            // Stable bootstrap identifier across restarts and fresh installations.
            identifier: ScopeIdentifier(uuid::Uuid::from_u128(1)),
            parent: None,
            name: "All Mail".into(),
            description: Some("Local development capture root".into()),
            filter: String::new(),
            position: 0,
            policy_version: 1,
        };
        storage.save_scope(&root_scope)?;
        scopes.push(root_scope);
    }
    let scope_tree = ScopeTree::new(scopes, application_configuration.maximum_scope_depth)?;
    let matcher = Arc::new(Matcher::new(&scope_tree)?);
    Ok((storage, matcher))
}
