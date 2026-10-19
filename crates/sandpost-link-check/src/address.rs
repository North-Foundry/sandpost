//! Classification of destination addresses that link checks may contact.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/**
 * Report whether an address is globally routable public unicast.
 *
 * Loopback, private, shared (carrier-grade NAT), link-local, unspecified, broadcast, multicast,
 * documentation, benchmarking and reserved ranges are not public. IPv6 must lie in global unicast
 * `2000::/3` outside its special-purpose blocks; IPv4-mapped, NAT64 and 6to4 addresses are judged by
 * the IPv4 address they embed, so a private target cannot be reached through a translation prefix.
 */
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

/**
 * Report whether an IPv4 address is outside every special-purpose and non-routable range.
 */
fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !(first == 0
        || first == 10
        || first == 127
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 192 && second == 0 && third == 2)
        || (first == 192 && second == 88 && third == 99)
        || (first == 192 && second == 168)
        || (first == 198 && (second == 18 || second == 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113)
        || first >= 224)
}

/**
 * Report whether an IPv6 address is public global unicast, judging embedded IPv4 by its target.
 */
fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    let octets = address.octets();
    // NAT64 well-known prefix 64:ff9b::/96 translates to the embedded IPv4 destination.
    if segments[..6] == [0x0064, 0xff9b, 0, 0, 0, 0] {
        return is_public_ipv4(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ));
    }
    // 6to4 2002::/16 reaches the IPv4 address held in its next 32 bits.
    if segments[0] == 0x2002 {
        return is_public_ipv4(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5]));
    }
    let global_unicast = segments[0] & 0xe000 == 0x2000;
    global_unicast
        // IETF protocol assignments 2001::/23, including Teredo 2001::/32.
        && !(segments[0] == 0x2001 && segments[1] < 0x0200)
        // Documentation 2001:db8::/32 and 3fff::/20.
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        // Segment Routing SIDs 5f00::/16.
        && segments[0] != 0x5f00
}
