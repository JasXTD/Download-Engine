// Download Engine: local adapter inventory only; never opens a network connection.
use std::{io, net::IpAddr};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// Stable adapter name (Windows adapter GUID; Unix device name).
    pub name: String,
    pub label: String,
    pub index: u32,
    pub index_v6: u32,
    pub addresses: Vec<IpAddr>,
    pub up: bool,
}

#[cfg(unix)]
pub fn interfaces() -> io::Result<Vec<Interface>> {
    use std::{ffi::CStr, ptr};
    let mut first = ptr::null_mut();
    // getifaddrs owns the linked list until freeifaddrs; copy all fields before releasing it.
    if unsafe { libc::getifaddrs(&mut first) } != 0 {
        return Err(io::Error::last_os_error());
    }
    struct Free(*mut libc::ifaddrs);
    impl Drop for Free {
        fn drop(&mut self) {
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let _free = Free(first);
    let mut result: Vec<Interface> = Vec::new();
    let mut cursor = first;
    while let Some(item) = unsafe { cursor.as_ref() } {
        cursor = item.ifa_next;
        if item.ifa_name.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr(item.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let index = unsafe { libc::if_nametoindex(item.ifa_name) };
        let slot = if let Some(slot) = result.iter().position(|entry| entry.name == name) {
            slot
        } else {
            result.push(Interface {
                label: name.clone(),
                name,
                index,
                index_v6: index,
                addresses: Vec::new(),
                up: item.ifa_flags & libc::IFF_UP as u32 != 0,
            });
            result.len() - 1
        };
        let Some(addr) = (unsafe { item.ifa_addr.as_ref() }) else {
            continue;
        };
        let ip = match i32::from(addr.sa_family) {
            libc::AF_INET => {
                let addr = unsafe { &*item.ifa_addr.cast::<libc::sockaddr_in>() };
                Some(IpAddr::from(addr.sin_addr.s_addr.to_ne_bytes()))
            }
            libc::AF_INET6 => {
                let addr = unsafe { &*item.ifa_addr.cast::<libc::sockaddr_in6>() };
                Some(IpAddr::from(addr.sin6_addr.s6_addr))
            }
            _ => None,
        };
        if let Some(ip) = ip {
            result[slot].addresses.push(ip);
        }
    }
    finish(&mut result);
    Ok(result)
}

#[cfg(windows)]
pub fn interfaces() -> io::Result<Vec<Interface>> {
    use std::{ffi::CStr, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, NO_ERROR},
        NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
        Networking::WinSock::*,
    };
    // Aligned storage, bounded retries if an adapter changes while taking the snapshot.
    let mut len = 16_384u32;
    for _ in 0..4 {
        if len > 4 * 1024 * 1024 {
            return Err(io::Error::other("Adapter inventory is too large"));
        }
        let mut memory = vec![0u64; (len as usize).div_ceil(8)];
        let first = memory.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let code = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                ptr::null(),
                first,
                &mut len,
            )
        };
        if code == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if code == ERROR_NO_DATA {
            return Ok(Vec::new());
        }
        if code != NO_ERROR {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let mut result = Vec::new();
        let mut cursor = first;
        while let Some(item) = unsafe { cursor.as_ref() } {
            cursor = item.Next;
            if item.AdapterName.is_null() {
                continue;
            }
            let name = unsafe { CStr::from_ptr(item.AdapterName.cast()) }
                .to_string_lossy()
                .into_owned();
            let mut label = String::new();
            if !item.FriendlyName.is_null() {
                let mut count = 0;
                while count < 1024 && unsafe { *item.FriendlyName.add(count) } != 0 {
                    count += 1;
                }
                label = String::from_utf16_lossy(unsafe {
                    std::slice::from_raw_parts(item.FriendlyName, count)
                });
            }
            let mut addresses = Vec::new();
            let mut address = item.FirstUnicastAddress;
            while let Some(entry) = unsafe { address.as_ref() } {
                address = entry.Next;
                let Some(addr) = (unsafe { entry.Address.lpSockaddr.as_ref() }) else {
                    continue;
                };
                match addr.sa_family {
                    AF_INET
                        if entry.Address.iSockaddrLength as usize
                            >= std::mem::size_of::<SOCKADDR_IN>() =>
                    {
                        let addr = unsafe { &*entry.Address.lpSockaddr.cast::<SOCKADDR_IN>() };
                        addresses.push(IpAddr::from(
                            unsafe { addr.sin_addr.S_un.S_addr }.to_ne_bytes(),
                        ));
                    }
                    AF_INET6
                        if entry.Address.iSockaddrLength as usize
                            >= std::mem::size_of::<SOCKADDR_IN6>() =>
                    {
                        let addr = unsafe { &*entry.Address.lpSockaddr.cast::<SOCKADDR_IN6>() };
                        addresses.push(IpAddr::from(unsafe { addr.sin6_addr.u.Byte }));
                    }
                    _ => {}
                }
            }
            result.push(Interface {
                name,
                label,
                index: unsafe { item.Anonymous1.Anonymous.IfIndex },
                index_v6: item.Ipv6IfIndex,
                addresses,
                up: item.OperStatus == IfOperStatusUp,
            });
        }
        finish(&mut result);
        return Ok(result);
    }
    Err(io::Error::other(
        "Network adapters changed while reading them",
    ))
}

fn finish(items: &mut Vec<Interface>) {
    for item in items.iter_mut() {
        item.addresses.retain(|ip| {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && !matches!(ip, IpAddr::V6(v6) if v6.is_unicast_link_local())
        });
        item.addresses.sort();
        item.addresses.dedup();
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
}
