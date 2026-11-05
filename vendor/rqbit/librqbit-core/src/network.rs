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

/// Well-known ports of services that are not torrent clients. Strangers' peer and node lists
/// can name them to turn torrent clients against someone else's server, such as an
/// internet-phone server on 5060, so they are never contacted.
const SERVICE_PORTS: [u16; 12] = [
    1433,  // SQL Server
    1900,  // SSDP device discovery
    3306,  // MySQL
    3389,  // Remote Desktop
    5060,  // SIP, internet phones
    5061,  // SIP over TLS
    5353,  // mDNS
    5432,  // PostgreSQL
    5900,  // VNC remote screens
    6379,  // Redis
    11211, // memcached
    27017, // MongoDB
];

/// Ports a torrent client never listens on: everything below 1024, which belongs to system
/// services such as web, mail and name servers, and the well-known ports above.
pub fn is_service_port(port: u16) -> bool {
    port < 1024 || SERVICE_PORTS.contains(&port)
}

/// Whether a peer or DHT node a stranger named may be contacted: a public address, on a port
/// that no other kind of service uses.
pub fn is_public_endpoint(addr: SocketAddr) -> bool {
    !is_service_port(addr.port()) && !is_local_address(addr.ip())
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
    fn service_ports_are_never_contacted_even_on_public_addresses() {
        let public: IpAddr = "93.184.216.34".parse().unwrap();
        for port in [0, 1, 22, 25, 53, 80, 123, 443, 1023, 1900, 3389, 5060, 5061, 11211] {
            assert!(is_service_port(port), "{port}");
            assert!(!is_public_endpoint(SocketAddr::new(public, port)), "{port}");
        }
        // Ordinary peer ports stay usable, including the ones of the blocked popups.
        for port in [1024, 3491, 3751, 6881, 6889, 25401, 51413, 65535] {
            assert!(!is_service_port(port), "{port}");
            assert!(is_public_endpoint(SocketAddr::new(public, port)), "{port}");
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
