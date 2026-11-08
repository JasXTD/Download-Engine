// Modified by Download Engine: explicit adapter/address binding on every supported OS.
// A missing adapter/address is an error, never a request to use the default route.
use crate::{Error, interfaces::interfaces};
use std::{
    ffi::CString,
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    str::FromStr,
};

#[derive(Debug, Clone)]
pub struct BindDevice {
    index: NonZeroU32,
    name: CString,
    address: IpAddr,
    pub dns: Option<SocketAddr>,
}

impl BindDevice {
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        let adapter = interfaces()
            .map_err(Error::BindDeviceInvalidError)?
            .into_iter()
            .find(|adapter| adapter.name == name && adapter.up)
            .ok_or(Error::BindDeviceInvalid)?;
        let address = *adapter.addresses.first().ok_or(Error::BindDeviceInvalid)?;
        Self::new(name, address, None)
    }

    pub fn new(name: &str, address: IpAddr, dns: Option<SocketAddr>) -> crate::Result<Self> {
        let adapter = interfaces()
            .map_err(Error::BindDeviceInvalidError)?
            .into_iter()
            .find(|adapter| {
                adapter.name == name && adapter.up && adapter.addresses.contains(&address)
            })
            .ok_or(Error::BindDeviceInvalid)?;
        let index = NonZeroU32::new(if address.is_ipv6() {
            adapter.index_v6
        } else {
            adapter.index
        })
        .ok_or(Error::BindDeviceInvalid)?;
        if let Some(dns) = dns {
            if dns.ip().is_unspecified()
                || dns.ip().is_multicast()
                || dns.port() == 0
                || dns.is_ipv6() != address.is_ipv6()
            {
                return Err(Error::BindDeviceInvalid);
            }
        }
        Ok(Self {
            index,
            name: CString::new(name).map_err(|_| Error::BindDeviceInvalid)?,
            address,
            dns,
        })
    }

    pub fn check(&self) -> crate::Result<()> {
        let current = Self::new(self.name(), self.address, self.dns)?;
        if current.index != self.index {
            return Err(Error::BindDeviceInvalid);
        }
        Ok(())
    }
    pub fn index(&self) -> NonZeroU32 {
        self.index
    }
    pub fn name(&self) -> &str {
        self.name.to_str().expect("constructed from UTF-8")
    }
    pub fn address(&self, port: u16) -> SocketAddr {
        SocketAddr::new(self.address, port)
    }

    pub fn bind_sref(&self, socket: &socket2::Socket, is_v6: bool) -> crate::Result<()> {
        self.check()?;
        if self.address.is_ipv6() != is_v6 {
            return Err(Error::BindDeviceInvalid);
        }
        self.bind_os(socket, is_v6)
            .map_err(Error::BindDeviceSetDeviceError)
    }

    #[cfg(target_os = "macos")]
    fn bind_os(&self, socket: &socket2::Socket, is_v6: bool) -> std::io::Result<()> {
        if is_v6 {
            socket.bind_device_by_index_v6(Some(self.index))
        } else {
            socket.bind_device_by_index_v4(Some(self.index))
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    fn bind_os(&self, socket: &socket2::Socket, _is_v6: bool) -> std::io::Result<()> {
        socket.bind_device(Some(self.name.as_bytes_with_nul()))
    }

    #[cfg(windows)]
    fn bind_os(&self, socket: &socket2::Socket, is_v6: bool) -> std::io::Result<()> {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            IP_ADD_IFLIST, IP_IFLIST, IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_ADD_IFLIST,
            IPV6_IFLIST, IPV6_UNICAST_IF, SOCKET, SOCKET_ERROR, WSAGetLastError, setsockopt,
        };
        let set = |level, option, value: u32| {
            let result = unsafe {
                setsockopt(
                    socket.as_raw_socket() as SOCKET,
                    level,
                    option,
                    (&value as *const u32).cast(),
                    4,
                )
            };
            if result == SOCKET_ERROR {
                return Err(std::io::Error::from_raw_os_error(unsafe {
                    WSAGetLastError()
                }));
            }
            Ok(())
        };
        // Windows specifies network byte order for IPv4, host byte order for IPv6.
        let (level, option, index) = if is_v6 {
            (IPPROTO_IPV6, IPV6_UNICAST_IF, self.index.get())
        } else {
            (IPPROTO_IP, IP_UNICAST_IF, self.index.get().to_be())
        };
        set(level, option, index)?;
        // UNICAST_IF controls sending only. Restrict UDP reception too, including
        // machines whose administrator enabled Windows weak-host receive mode.
        if socket.r#type()? == socket2::Type::DGRAM {
            let (enable, add) = if is_v6 {
                (IPV6_IFLIST, IPV6_ADD_IFLIST)
            } else {
                (IP_IFLIST, IP_ADD_IFLIST)
            };
            set(level, enable, 1)?;
            set(level, add, self.index.get())?;
        }
        Ok(())
    }
}
impl FromStr for BindDevice {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Self::new_from_name(s)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::*;

    #[test]
    fn windows_adapter_binding_sets_socket_policy_without_sending_packets() {
        let mut checked = 0;
        for adapter in interfaces().unwrap().into_iter().filter(|item| item.up) {
            for address in adapter
                .addresses
                .iter()
                .filter(|address| address.is_loopback())
            {
                let device = BindDevice::new(&adapter.name, *address, None).unwrap();
                let v6 = address.is_ipv6();
                for typ in [socket2::Type::STREAM, socket2::Type::DGRAM] {
                    let socket = socket2::Socket::new(
                        if v6 {
                            socket2::Domain::IPV6
                        } else {
                            socket2::Domain::IPV4
                        },
                        typ,
                        None,
                    )
                    .unwrap();
                    device.bind_sref(&socket, v6).unwrap();
                    socket.bind(&device.address(0).into()).unwrap();
                    assert_eq!(
                        socket.local_addr().unwrap().as_socket().unwrap().ip(),
                        *address
                    );
                    let level = if v6 { IPPROTO_IPV6 } else { IPPROTO_IP };
                    let get = |option| {
                        let mut value = 0u32;
                        let mut size = 4;
                        assert_eq!(
                            unsafe {
                                getsockopt(
                                    socket.as_raw_socket() as SOCKET,
                                    level,
                                    option,
                                    (&mut value as *mut u32).cast(),
                                    &mut size,
                                )
                            },
                            0
                        );
                        value
                    };
                    assert_eq!(
                        get(if v6 { IPV6_UNICAST_IF } else { IP_UNICAST_IF }),
                        device.index.get()
                    );
                    if typ == socket2::Type::DGRAM {
                        assert_eq!(get(if v6 { IPV6_IFLIST } else { IP_IFLIST }), 1);
                        assert_eq!(
                            get(if v6 { IPV6_GET_IFLIST } else { IP_GET_IFLIST }),
                            device.index.get()
                        );
                    }
                    assert!(device.bind_sref(&socket, !v6).is_err());
                    checked += 1;
                }
            }
        }
        assert!(
            checked >= 2,
            "No usable Windows loopback adapter was enumerated"
        );
    }
}
