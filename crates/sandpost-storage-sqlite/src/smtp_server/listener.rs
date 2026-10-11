//! Persistence of the single global SMTP listener address.
use crate::SqliteStorage;
use crate::error::StorageResult;
use sandpost_storage::StorageError;
use sqlx::Row;
use std::net::{IpAddr, SocketAddr};

/// Load the global listener address, combining the stored host and port.
pub(super) async fn listen_address(
    storage: &SqliteStorage,
) -> Result<Option<SocketAddr>, StorageError> {
    let row = sqlx::query("SELECT smtp_host, smtp_port FROM smtp_server WHERE singleton = 1")
        .fetch_one(&storage.read_pool)
        .await
        .storage()?;
    let host: Option<String> = row.try_get("smtp_host").storage()?;
    let port: Option<i64> = row.try_get("smtp_port").storage()?;
    match (host, port) {
        (Some(host), Some(port)) => Ok(Some(SocketAddr::new(
            host.parse::<IpAddr>()
                .map_err(|_| StorageError::InvalidData(format!("invalid SMTP host {host}")))?,
            u16::try_from(port).map_err(|_| StorageError::IntegerRange)?,
        ))),
        (None, None) => Ok(None),
        _ => Err(StorageError::InvalidData(
            "SMTP host and port must be stored together".into(),
        )),
    }
}

/// Replace the global listener address as a textual IP host and numeric port.
pub(super) async fn save_listen_address(
    storage: &SqliteStorage,
    address: Option<SocketAddr>,
) -> Result<(), StorageError> {
    sqlx::query("UPDATE smtp_server SET smtp_host = ?1, smtp_port = ?2 WHERE singleton = 1")
        .bind(address.map(|address| address.ip().to_string()))
        .bind(address.map(|address| address.port()))
        .execute(&storage.write_pool)
        .await
        .storage()?;
    Ok(())
}
