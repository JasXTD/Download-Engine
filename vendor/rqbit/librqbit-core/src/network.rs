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
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80
                || first & 0xffc0 == 0xfec0
        }
    }
}

pub fn is_public_endpoint(addr: SocketAddr) -> bool {
    addr.port() != 0 && !is_local_address(addr.ip())
}
