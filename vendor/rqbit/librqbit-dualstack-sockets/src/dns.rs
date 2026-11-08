// Download Engine: bounded DNS over the selected interface. Never falls back to
// the system resolver while bound. Only A/AAAA answers for the requested name or
// its CNAME chain are used; additional-section glue is not trusted.
use crate::{BindDevice, BindOpts, ConnectOpts, UdpSocket, tcp_connect};
use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

pub async fn lookup(
    device: Option<&BindDevice>,
    host: &str,
    port: u16,
) -> io::Result<Vec<SocketAddr>> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if device.is_some_and(|device| device.address(0).is_ipv6() != ip.is_ipv6()) {
            return Err(other(
                "Address family is not available on the bound torrent address",
            ));
        }
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let Some(device) = device else {
        return Ok(tokio::net::lookup_host((host, port)).await?.collect());
    };
    device.check().map_err(other)?;
    let server = device
        .dns
        .ok_or_else(|| other("Set a DNS server for the bound torrent interface"))?;
    let kind = if device.address(0).is_ipv6() { 28 } else { 1 };
    let lookup = async {
        let mut host = normalized(host)?;
        let mut visited = Vec::new();
        for _ in 0..8 {
            if visited.contains(&host) {
                return Err(invalid("DNS alias loop"));
            }
            visited.push(host.clone());
            let mut random = [0; 2];
            getrandom::fill(&mut random).map_err(other)?;
            let id = u16::from_ne_bytes(random);
            let query = question(&host, kind, id)?;
            let udp = UdpSocket::bind_udp(
                device.address(0),
                BindOpts {
                    request_dualstack: false,
                    device: Some(device),
                    ..Default::default()
                },
            )
            .map_err(other)?;
            udp.socket().connect(server).await?;
            udp.socket().send(&query).await?;
            let mut packet = vec![0; 65_536];
            let size = udp.socket().recv(&mut packet).await?;
            packet.truncate(size);
            validate_header(&packet, id)?;
            if packet[2] & 2 != 0 {
                let mut tcp = tcp_connect(
                    server,
                    ConnectOpts {
                        bind_device: Some(device),
                        ..Default::default()
                    },
                )
                .await
                .map_err(other)?;
                tcp.write_u16(query.len() as u16).await?;
                tcp.write_all(&query).await?;
                let size = usize::from(tcp.read_u16().await?);
                packet.resize(size, 0);
                tcp.read_exact(&mut packet).await?;
            }
            let (addresses, alias) = answer(&packet, &host, kind, id)?;
            if !addresses.is_empty() {
                return Ok(addresses
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, port))
                    .collect());
            }
            match alias {
                Some(next) if next != host => host = next,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "DNS returned no usable address",
                    ));
                }
            }
        }
        Err(invalid("DNS alias chain is too long"))
    };
    tokio::time::timeout(Duration::from_secs(8), lookup)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Bound DNS lookup timed out"))?
}

fn normalized(name: &str) -> io::Result<String> {
    let name = name.trim_end_matches('.');
    if name.is_empty()
        || name.len() > 253
        || !name.is_ascii()
        || name.split('.').any(|part| {
            part.is_empty()
                || part.len() > 63
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
    {
        return Err(invalid("Invalid DNS name"));
    }
    Ok(name.to_ascii_lowercase())
}
fn question(name: &str, kind: u16, id: u16) -> io::Result<Vec<u8>> {
    let name = normalized(name)?;
    let mut data = vec![0; 12];
    data[..2].copy_from_slice(&id.to_be_bytes());
    data[2] = 1; // Recursion desired, one question.
    data[5] = 1;
    for part in name.split('.') {
        data.push(part.len() as u8);
        data.extend_from_slice(part.as_bytes());
    }
    data.push(0);
    data.extend_from_slice(&kind.to_be_bytes());
    data.extend_from_slice(&1u16.to_be_bytes());
    Ok(data)
}
fn number(data: &[u8], pos: &mut usize) -> io::Result<u16> {
    let bytes: [u8; 2] = data
        .get(*pos..*pos + 2)
        .ok_or_else(|| invalid("Truncated DNS number"))?
        .try_into()
        .unwrap();
    *pos += 2;
    Ok(u16::from_be_bytes(bytes))
}
fn name(data: &[u8], pos: &mut usize) -> io::Result<String> {
    let mut cursor = *pos;
    let mut jumped = false;
    let mut result = String::new();
    for _ in 0..128 {
        let size = *data
            .get(cursor)
            .ok_or_else(|| invalid("Truncated DNS name"))?;
        cursor += 1;
        if size & 0xc0 == 0xc0 {
            let low = *data
                .get(cursor)
                .ok_or_else(|| invalid("Truncated DNS pointer"))?;
            if !jumped {
                *pos = cursor + 1;
                jumped = true;
            }
            let target = (usize::from(size & 0x3f) << 8) | usize::from(low);
            // Compression can only refer backwards, preventing cycles and forward references.
            if target >= cursor - 1 {
                return Err(invalid("Invalid DNS compression pointer"));
            }
            cursor = target;
            continue;
        }
        if size & 0xc0 != 0 {
            return Err(invalid("Invalid DNS label"));
        }
        if size == 0 {
            if !jumped {
                *pos = cursor;
            }
            return normalized(&result);
        }
        let bytes = data
            .get(cursor..cursor + usize::from(size))
            .ok_or_else(|| invalid("Truncated DNS label"))?;
        if !result.is_empty() {
            result.push('.');
        }
        result.push_str(std::str::from_utf8(bytes).map_err(other)?);
        if result.len() > 253 {
            return Err(invalid("DNS name is too long"));
        }
        cursor += usize::from(size);
    }
    Err(invalid("DNS name has too many labels"))
}
fn validate_header(data: &[u8], id: u16) -> io::Result<()> {
    if data.len() < 12
        || data[..2] != id.to_be_bytes()
        || data[2] & 0xf8 != 0x80
        || data[3] & 0x0f != 0
        || data[4..6] != [0, 1]
    {
        return Err(invalid("Invalid or unsuccessful DNS response"));
    }
    Ok(())
}
fn answer(
    data: &[u8],
    host: &str,
    kind: u16,
    id: u16,
) -> io::Result<(Vec<IpAddr>, Option<String>)> {
    validate_header(data, id)?;
    if data[2] & 2 != 0 {
        return Err(invalid("Truncated DNS response"));
    }
    let count = u16::from_be_bytes([data[6], data[7]]);
    if count > 256 {
        return Err(invalid("Too many DNS answers"));
    }
    let mut pos = 12;
    if name(data, &mut pos)? != normalized(host)?
        || number(data, &mut pos)? != kind
        || number(data, &mut pos)? != 1
    {
        return Err(invalid("DNS question mismatch"));
    }
    let mut records = Vec::new();
    for _ in 0..count {
        let owner = name(data, &mut pos)?;
        let typ = number(data, &mut pos)?;
        let class = number(data, &mut pos)?;
        pos += 4; // TTL: no cache, so no stale address can outlive the interface.
        let len = usize::from(number(data, &mut pos)?);
        let end = pos
            .checked_add(len)
            .filter(|end| *end <= data.len())
            .ok_or_else(|| invalid("Truncated DNS record"))?;
        let bytes = &data[pos..end];
        let ip = match (class, typ, len) {
            (1, 1, 4) => Some(IpAddr::from(<[u8; 4]>::try_from(bytes).unwrap())),
            (1, 28, 16) => Some(IpAddr::from(<[u8; 16]>::try_from(bytes).unwrap())),
            _ => None,
        };
        let alias = if class == 1 && typ == 5 {
            let alias = name(data, &mut pos)?;
            if pos != end {
                return Err(invalid("Invalid DNS alias length"));
            }
            Some(alias)
        } else {
            None
        };
        records.push((owner, typ, ip, alias));
        pos = end;
    }
    let mut current = normalized(host)?;
    let mut visited = Vec::new();
    for _ in 0..8 {
        if visited.contains(&current) {
            return Err(invalid("DNS alias loop"));
        }
        visited.push(current.clone());
        let ips: Vec<_> = records
            .iter()
            .filter(|(owner, typ, _, _)| owner == &current && *typ == kind)
            .filter_map(|(_, _, ip, _)| *ip)
            .take(32)
            .collect();
        if !ips.is_empty() {
            return Ok((ips, None));
        }
        if let Some(next) = records.iter().find_map(|(owner, _, _, alias)| {
            (owner == &current).then_some(alias.as_ref()).flatten()
        }) {
            current = next.clone();
        } else {
            return Ok((Vec::new(), (current != host).then_some(current)));
        }
    }
    Err(invalid("DNS alias chain is too long"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_ipv6_and_unrelated_records_are_bounded_and_matched() {
        fn wire(value: &str) -> Vec<u8> {
            let question = question(value, 1, 1).unwrap();
            question[12..question.len() - 4].to_vec()
        }
        fn record(data: &mut Vec<u8>, owner: &str, typ: u16, value: &[u8]) {
            data.extend(wire(owner));
            data.extend(typ.to_be_bytes());
            data.extend([0, 1, 0, 0, 0, 30]);
            data.extend((value.len() as u16).to_be_bytes());
            data.extend(value);
        }
        let mut data = question("sample.test", 28, 42).unwrap();
        data[2] = 0x81;
        data[3] = 0x80;
        data[7] = 3;
        record(&mut data, "unrelated.test", 28, &[0; 16]);
        record(&mut data, "sample.test", 5, &wire("alias.test"));
        let ip: std::net::Ipv6Addr = "2001:db8::1".parse().unwrap();
        record(&mut data, "alias.test", 28, &ip.octets());
        assert_eq!(
            answer(&data, "sample.test", 28, 42).unwrap().0,
            vec![IpAddr::V6(ip)]
        );
        data[7] = 2;
        assert_eq!(
            answer(&data, "sample.test", 28, 42).unwrap(),
            (Vec::new(), Some("alias.test".into())),
            "Ignore records outside the answer section"
        );
        data[6] = 1;
        data[7] = 1;
        assert!(answer(&data, "sample.test", 28, 42).is_err());
    }
    #[test]
    fn strict_answers_reject_spoofs_truncation_and_pointer_cycles() {
        let mut data = question("sample.test", 1, 12).unwrap();
        data[2] = 0x81;
        data[3] = 0x80;
        data[7] = 1;
        data.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 192, 0, 2, 1]);
        assert_eq!(
            answer(&data, "sample.test", 1, 12).unwrap().0,
            vec!["192.0.2.1".parse::<IpAddr>().unwrap()]
        );
        assert!(answer(&data, "other.test", 1, 12).is_err());
        assert!(answer(&data, "sample.test", 1, 13).is_err());
        for end in 0..data.len() {
            assert!(answer(&data[..end], "sample.test", 1, 12).is_err());
        }
        assert!(name(&[0xc0, 0], &mut 0).is_err());
        data[2] |= 2;
        assert!(answer(&data, "sample.test", 1, 12).is_err());
    }
}
