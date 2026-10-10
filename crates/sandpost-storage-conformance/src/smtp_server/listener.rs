//! Global SMTP listener address conformance check.
use sandpost_storage::Storage;

use crate::ConformanceFailure;
use crate::support::{unwrap_storage, verify_equal};

/// Verify the global listener address round-trips, including the disabled state.
pub async fn smtp_server_address(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "smtp_server_address";
    let original = unwrap_storage(
        CHECK,
        "smtp_listen_address",
        storage.smtp_listen_address().await,
    )?;
    for address in [
        Some("127.0.0.1:2526".parse().expect("valid IPv4 socket address")),
        Some("[::1]:2527".parse().expect("valid IPv6 socket address")),
        None,
    ] {
        unwrap_storage(
            CHECK,
            "save_smtp_listen_address",
            storage.save_smtp_listen_address(address).await,
        )?;
        verify_equal(
            CHECK,
            "stored SMTP listen address",
            unwrap_storage(
                CHECK,
                "smtp_listen_address after save",
                storage.smtp_listen_address().await,
            )?,
            address,
        )?;
    }
    unwrap_storage(
        CHECK,
        "restore smtp_listen_address",
        storage.save_smtp_listen_address(original).await,
    )
}
