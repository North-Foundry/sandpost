//! Derive the addresses local clients connect to from the addresses listeners are bound to.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Replace wildcard binds with matching-family loopback addresses for local clients.
pub(crate) fn connection_address(listen_address: SocketAddr) -> SocketAddr {
    match listen_address.ip() {
        IpAddr::V4(address) if address.is_unspecified() => {
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), listen_address.port())
        }
        IpAddr::V6(address) if address.is_unspecified() => {
            SocketAddr::new(Ipv6Addr::LOCALHOST.into(), listen_address.port())
        }
        _ => listen_address,
    }
}

/// Format a connection host without URL brackets, retaining an IPv6 scope if present.
pub(crate) fn connection_host(connection_address: SocketAddr) -> String {
    match connection_address {
        SocketAddr::V6(address) if address.scope_id() != 0 => {
            format!("{}%{}", address.ip(), address.scope_id())
        }
        _ => connection_address.ip().to_string(),
    }
}

#[cfg(test)]
mod tests;
