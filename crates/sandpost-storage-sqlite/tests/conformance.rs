//! Run the reusable backend conformance suite against the SQLite backend.
use sandpost_storage::Storage;
use sandpost_storage_sqlite::SqliteStorage;
use std::sync::Arc;

/// The SQLite backend must satisfy every storage conformance check.
#[tokio::test]
async fn sqlite_satisfies_the_storage_conformance_suite()
-> Result<(), sandpost_storage_conformance::ConformanceFailure> {
    let storage: Arc<dyn Storage> =
        Arc::new(SqliteStorage::memory().expect("create in-memory storage"));
    sandpost_storage_conformance::run_all(storage.as_ref()).await
}
