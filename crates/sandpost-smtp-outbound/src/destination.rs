//! Resolution of an SMTP server's host name under the destination policy.
use crate::failure::failure;
use sandpost_core::{OutboundFailure, OutboundFailureKind};
use sandpost_link_check::is_public_address;
use std::net::SocketAddr;
use std::time::Duration;

/// Which network destinations outbound SMTP connections may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DestinationPolicy {
    /// Permit loopback, private, link-local and other non-public addresses. Off by default, so
    /// configuring an SMTP Outbound cannot be used to probe internal services.
    pub allow_internal_addresses: bool,
}

/**
 * Resolve `host` once and return every address to try, in resolver order.
 *
 * Unless the policy allows internal addresses, the whole answer is refused when any address is
 * not public (loopback, private, link-local including cloud metadata, carrier-grade NAT,
 * documentation, multicast, reserved, or an IPv4-mapped, NAT64 or 6to4 form of one), so a
 * mixed DNS answer cannot smuggle an internal destination in. IP literals are checked the same
 * way. Resolution is bounded by `limit`.
 */
pub async fn resolve(
    host: &str,
    port: u16,
    policy: &DestinationPolicy,
    limit: Duration,
) -> Result<Vec<SocketAddr>, OutboundFailure> {
    let lookup = tokio::net::lookup_host((host, port));
    let addresses: Vec<SocketAddr> = match tokio::time::timeout(limit, lookup).await {
        Err(_) => {
            return Err(failure(
                OutboundFailureKind::Connection,
                format!(
                    "resolving {host} did not finish within {} s",
                    limit.as_secs()
                ),
            ));
        }
        Ok(Err(error)) => {
            return Err(failure(
                OutboundFailureKind::Connection,
                format!("{host} could not be resolved: {error}"),
            ));
        }
        Ok(Ok(addresses)) => addresses.collect(),
    };
    if addresses.is_empty() {
        return Err(failure(
            OutboundFailureKind::Connection,
            format!("{host} has no address"),
        ));
    }
    if !policy.allow_internal_addresses
        && let Some(internal) = addresses
            .iter()
            .find(|address| !is_public_address(address.ip()))
    {
        return Err(failure(
            OutboundFailureKind::Policy,
            format!(
                "{host} resolves to the non-public address {}; internal destinations are refused unless the [outbound] allow_internal_addresses setting is enabled",
                internal.ip()
            ),
        ));
    }
    Ok(addresses)
}
