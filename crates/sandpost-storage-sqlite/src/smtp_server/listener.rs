//! Persistence of the single global SMTP listener address.
use crate::error::StorageResult;
use rusqlite::{Connection, params};
use sandpost_storage::StorageError;
use std::net::{IpAddr, SocketAddr};

/// Load the global listener address, combining the stored host and port.
pub(super) fn listen_address_blocking(
    connection: &Connection,
) -> Result<Option<SocketAddr>, StorageError> {
    let (host, port): (Option<String>, Option<i64>) = connection
        .query_row(
            "SELECT smtp_host, smtp_port FROM smtp_server WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .storage()?;
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
pub(super) fn save_listen_address_blocking(
    connection: &Connection,
    address: Option<SocketAddr>,
) -> Result<(), StorageError> {
    connection
        .execute(
            "UPDATE smtp_server SET smtp_host = ?1, smtp_port = ?2 WHERE singleton = 1",
            params![
                address.map(|address| address.ip().to_string()),
                address.map(|address| address.port())
            ],
        )
        .storage()?;
    Ok(())
}
