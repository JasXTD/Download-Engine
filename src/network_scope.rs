//! Shared destination checks for FTP, website discovery and video manifests.
use anyhow::{Context, Result, ensure};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkScope {
    Public,
    LocalOrSpecial,
}

fn ipv4_scope(ip: Ipv4Addr) -> Result<NetworkScope> {
    let [a, b, c, _] = ip.octets();
    ensure!(
        !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast(),
        "The server address resolved to an unusable network destination"
    );
    let special = ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240;
    Ok(if special {
        NetworkScope::LocalOrSpecial
    } else {
        NetworkScope::Public
    })
}

fn ipv6_scope(ip: Ipv6Addr) -> Result<NetworkScope> {
    if let Some(ip) = ip.to_ipv4_mapped() {
        return ipv4_scope(ip);
    }
    ensure!(
        !ip.is_unspecified() && !ip.is_multicast(),
        "The server address resolved to an unusable network destination"
    );
    let segments = ip.segments();
    let special = ip.is_loopback()
        || segments[0] & 0xfe00 == 0xfc00 // unique-local fc00::/7
        || segments[0] & 0xffc0 == 0xfe80 // link-local fe80::/10
        || segments[0] & 0xffc0 == 0xfec0 // deprecated site-local fec0::/10
        // RFC 8215 local-use translation can reach private IPv4 destinations.
        || (segments[0] == 0x64 && segments[1] == 0xff9b && segments[2] == 1)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8); // documentation
    Ok(if special {
        NetworkScope::LocalOrSpecial
    } else {
        NetworkScope::Public
    })
}

pub(crate) fn network_scope(ip: IpAddr) -> Result<NetworkScope> {
    match ip {
        IpAddr::V4(ip) => ipv4_scope(ip),
        IpAddr::V6(ip) => ipv6_scope(ip),
    }
}

/// Reader-controlled requests must use our checked addresses, never proxy DNS.
/// Build a new client for each destination so redirects cannot escape the check.
pub(crate) async fn public_client(
    url: &reqwest::Url,
    route: &crate::proxy::Route,
) -> Result<reqwest::Client> {
    route.require_public_pinning()?;
    let mut builder = crate::engine::client_builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    let host = url
        .host_str()
        .context("A site reader URL needs a host")?
        .trim_matches(['[', ']']);
    let allowed = |ip| -> Result<()> {
        ensure!(
            network_scope(ip)? == NetworkScope::Public,
            "Site readers can only reach the public internet"
        );
        Ok(())
    };
    if let Ok(ip) = host.parse::<IpAddr>() {
        allowed(ip)?;
    } else if let Some((host, addresses)) = crate::website_grabber::pinned_resolution(url).await? {
        for address in &addresses {
            allowed(address.ip())?;
        }
        builder = builder.resolve_to_addrs(&host, &addresses);
    }
    Ok(builder.build()?)
}

/// Validate the entire resolution before any connection attempt. Checking each
/// IP alone would let a public connection failure fall back to a local service.
pub(crate) fn validate_resolution(
    addresses: impl IntoIterator<Item = IpAddr>,
) -> Result<NetworkScope> {
    let mut addresses = addresses.into_iter();
    let first = addresses
        .next()
        .context("The server host did not resolve to an address")?;
    let scope = network_scope(first)?;
    for address in addresses {
        ensure!(
            network_scope(address)? == scope,
            "The server host resolves across public and local networks; connection is blocked"
        );
    }
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_cannot_cross_public_and_local_networks() {
        for (addresses, expected) in [
            (["8.8.8.8", "1.1.1.1"], NetworkScope::Public),
            (["127.0.0.1", "::1"], NetworkScope::LocalOrSpecial),
            (
                ["127.0.0.1", "::ffff:127.0.0.1"],
                NetworkScope::LocalOrSpecial,
            ),
        ] {
            assert_eq!(
                validate_resolution(addresses.map(|address| address.parse().unwrap())).unwrap(),
                expected
            );
        }
        for addresses in [
            ["8.8.8.8", "127.0.0.1"],
            ["127.0.0.1", "8.8.8.8"],
            ["8.8.8.8", "::ffff:127.0.0.1"],
            ["2606:4700::1111", "fc00::1"],
        ] {
            assert!(
                validate_resolution(addresses.map(|address| address.parse().unwrap()))
                    .unwrap_err()
                    .to_string()
                    .contains("public and local")
            );
        }
    }

    #[test]
    fn resolution_rejects_unusable_destinations() {
        assert!(validate_resolution([]).is_err());
        for address in ["0.0.0.0", "255.255.255.255", "224.0.0.1", "::", "ff02::1"] {
            assert!(validate_resolution([address.parse().unwrap()]).is_err());
        }
    }

    #[test]
    fn local_use_translation_is_not_public() {
        for address in [
            "64:ff9b:1::",
            "64:ff9b:1::c0a8:101",
            "64:ff9b:1::a00:1",
            "64:ff9b:1:ffff:ffff:ffff:ffff:ffff",
        ] {
            let ip = address.parse().unwrap();
            assert_eq!(network_scope(ip).unwrap(), NetworkScope::LocalOrSpecial);
            for addresses in [
                ["2606:4700::1111".parse().unwrap(), ip],
                [ip, "8.8.8.8".parse().unwrap()],
            ] {
                assert!(validate_resolution(addresses).is_err());
            }
        }
        // Neither adjacent prefixes nor the well-known translation prefix are
        // part of the local-use /48; normal public IPv6 must remain available.
        for address in [
            "64:ff9b::808:808",
            "64:ff9b:0:ffff:ffff:ffff:ffff:ffff",
            "64:ff9b:2::",
            "2606:4700::1111",
        ] {
            assert_eq!(
                network_scope(address.parse().unwrap()).unwrap(),
                NetworkScope::Public
            );
        }
    }

    #[tokio::test]
    async fn public_reader_rejects_local_use_translation_before_io() {
        // Only construct a checked client. Never resolve a host or send a request.
        for address in [
            "64:ff9b:1::",
            "64:ff9b:1::c0a8:101",
            "64:ff9b:1:ffff:ffff:ffff:ffff:ffff",
        ] {
            let url = reqwest::Url::parse(&format!("https://[{address}]/")).unwrap();
            let error = public_client(&url, &crate::proxy::Route::direct())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("only reach the public internet"));
        }
    }
}
