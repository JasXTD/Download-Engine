// Added by Download Engine: shared public-network policy. See ../../patch.md.
use std::net::{IpAddr, SocketAddr};

/// Addresses that stranger-supplied trackers, peers and DHT nodes must not reach.
pub fn is_local_address(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
                || first == 0
                || first >= 240
                || (first == 100 && (64..=127).contains(&second))
                || (first == 198 && (second == 18 || second == 19))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let first = segments[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
                || first & 0xffc0 == 0xfec0
                // RFC 8215 local-use translation can reach private IPv4 destinations.
                || (first == 0x64 && segments[1] == 0xff9b && segments[2] == 1)
        }
    }
}

pub fn is_public_endpoint(addr: SocketAddr) -> bool {
    addr.port() != 0 && !is_local_address(addr.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_use_translation_is_not_a_public_endpoint() {
        for address in [
            "64:ff9b:1::",
            "64:ff9b:1::c0a8:101",
            "64:ff9b:1::a00:1",
            "64:ff9b:1:ffff:ffff:ffff:ffff:ffff",
        ] {
            let ip = address.parse().unwrap();
            assert!(is_local_address(ip), "{address}");
            assert!(!is_public_endpoint(SocketAddr::new(ip, 6881)), "{address}");
        }
    }

    #[test]
    fn local_use_translation_filter_preserves_public_ipv6() {
        for address in [
            "64:ff9b::808:808",
            "64:ff9b:0:ffff:ffff:ffff:ffff:ffff",
            "64:ff9b:2::",
            "2606:4700::1111",
        ] {
            let ip = address.parse().unwrap();
            assert!(!is_local_address(ip), "{address}");
            assert!(is_public_endpoint(SocketAddr::new(ip, 6881)), "{address}");
        }
    }
}
