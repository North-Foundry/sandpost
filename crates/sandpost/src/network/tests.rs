use super::*;

/// Wildcards become same-family loopback hosts, while concrete and scoped addresses survive.
#[test]
fn connection_hosts_replace_wildcards_and_keep_scopes() {
    for (listen_address, expected_address, expected_host) in [
        ("0.0.0.0:1143", "127.0.0.1:1143", "127.0.0.1"),
        ("[::]:1143", "[::1]:1143", "::1"),
        ("192.0.2.4:993", "192.0.2.4:993", "192.0.2.4"),
        ("[fe80::2%3]:143", "[fe80::2%3]:143", "fe80::2%3"),
    ] {
        let address = connection_address(listen_address.parse().unwrap());
        assert_eq!(
            address,
            expected_address.parse().unwrap(),
            "{listen_address}"
        );
        assert_eq!(connection_host(address), expected_host, "{listen_address}");
    }
}
