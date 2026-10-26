use super::{Endpoint, Metadata, Security, valid_timestamp};
use crate::{auth::Credentials, retry};
use anyhow::{Context, Result, bail, ensure};
use std::{future::Future, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(super) trait Transport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Transport for T {}
pub(super) type Stream = Box<dyn Transport>;
pub(super) struct Client {
    stream: Option<Stream>,
    peer: SocketAddr,
    endpoint: Endpoint,
    tls: tokio_native_tls::TlsConnector,
    certificate: Option<Vec<u8>>,
}
struct Reply {
    code: u16,
    text: String,
}

pub(super) fn connector() -> Result<tokio_native_tls::TlsConnector> {
    Ok(native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()?
        .into())
}
pub(super) async fn wait<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("FTP transfer cancelled"),
        value = tokio::time::timeout(Duration::from_secs(30), future) =>
            value.map_err(|_| retry::incomplete("The FTP server connection timed out."))?,
    }
}
fn network(error: std::io::Error) -> anyhow::Error {
    use std::io::ErrorKind::*;
    if matches!(
        error.kind(),
        ConnectionReset
            | ConnectionAborted
            | ConnectionRefused
            | NotConnected
            | TimedOut
            | Interrupted
            | UnexpectedEof
            | BrokenPipe
            | NetworkUnreachable
            | HostUnreachable
    ) {
        retry::incomplete(
            "The FTP server connection was interrupted or is temporarily unavailable.",
        )
    } else {
        anyhow::anyhow!("FTP network operation failed ({:?})", error.kind())
    }
}
async fn reply(stream: &mut Stream) -> Result<Reply> {
    let mut first = None;
    let mut total = 0;
    // Bound both each line and an entire multiline greeting/reply. Reading one
    // byte at a time avoids consuming TLS bytes before an AUTH TLS upgrade.
    for _ in 0..64 {
        let mut line = Vec::new();
        loop {
            let mut byte = [0];
            let n = stream.read(&mut byte).await.map_err(network)?;
            if n == 0 {
                return Err(retry::incomplete(
                    "FTP server closed before a complete reply.",
                ));
            }
            line.push(byte[0]);
            total += 1;
            ensure!(
                line.len() <= 4096 && total <= 32768,
                "FTP reply exceeds its size limit"
            );
            if byte[0] == b'\n' {
                break;
            }
        }
        ensure!(line.ends_with(b"\r\n"), "Malformed FTP reply");
        line.truncate(line.len() - 2);
        ensure!(
            !line.iter().any(|b| b.is_ascii_control()),
            "Invalid FTP reply characters"
        );
        let header = if line.len() >= 4
            && line[..3].iter().all(u8::is_ascii_digit)
            && matches!(line[3], b' ' | b'-')
        {
            Some((
                (line[0] - b'0') as u16 * 100
                    + (line[1] - b'0') as u16 * 10
                    + (line[2] - b'0') as u16,
                line[3],
            ))
        } else {
            None
        };
        if let Some(code) = first {
            if header != Some((code, b' ')) {
                continue;
            }
        } else {
            let (code, separator) = header.context("Malformed FTP reply code")?;
            ensure!((100..600).contains(&code), "Invalid FTP reply code");
            if separator == b'-' {
                first = Some(code);
                continue;
            }
        }
        let (code, _) = header.context("Malformed FTP reply")?;
        // Never report untrusted server text: it may echo a username/password.
        return Ok(Reply {
            code,
            text: String::from_utf8(line[4..].to_vec()).context("Invalid FTP response encoding")?,
        });
    }
    bail!("FTP reply has too many lines")
}
fn unexpected(code: u16, operation: &str) -> anyhow::Error {
    if matches!(code, 421 | 425 | 426 | 450 | 451 | 452) {
        retry::incomplete(&format!("FTP {operation} temporarily failed ({code})."))
    } else if code == 530 || code == 532 {
        anyhow::anyhow!(
            "FTP login was required or rejected ({code}). Check Authentication; saved logins require FTPS."
        )
    } else {
        anyhow::anyhow!("FTP {operation} failed ({code}).")
    }
}
fn expect(reply: Reply, codes: &[u16], operation: &str) -> Result<Reply> {
    if codes.contains(&reply.code) {
        Ok(reply)
    } else {
        Err(unexpected(reply.code, operation))
    }
}
async fn secure(
    stream: Stream,
    tls: &tokio_native_tls::TlsConnector,
    host: &str,
) -> Result<(Stream, Vec<u8>)> {
    let socket = tls.connect(host, stream).await.map_err(|_| anyhow::anyhow!(
        "FTPS TLS handshake failed. Check the server certificate, hostname and TLS mode; certificate checks cannot be bypassed."
    ))?;
    let certificate = socket
        .get_ref()
        .peer_certificate()?
        .context("FTPS server did not provide a certificate")?
        .to_der()?;
    Ok((Box::new(socket), certificate))
}

async fn connect_addresses(addresses: &[SocketAddr]) -> Result<TcpStream> {
    ensure!(
        !addresses.is_empty() && addresses.len() <= 32,
        "FTP server has no usable address or too many addresses"
    );
    crate::network_scope::validate_resolution(addresses.iter().map(|address| address.ip()))?;
    TcpStream::connect(addresses).await.map_err(network)
}

impl Client {
    pub(super) async fn connect(
        endpoint: Endpoint,
        credentials: Option<&Credentials>,
        tls: tokio_native_tls::TlsConnector,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        ensure!(
            credentials.is_none() || endpoint.security != Security::Plain,
            "Saved logins require FTPS. Plain FTP is anonymous only."
        );
        wait(cancel, async {
            let addresses: Vec<_> =
                tokio::net::lookup_host((endpoint.host.as_str(), endpoint.port))
                    .await
                    .map_err(network)?
                    .take(33)
                    .collect();
            let socket = connect_addresses(&addresses).await?;
            let peer = socket.peer_addr().map_err(network)?;
            let mut client = Self {
                stream: Some(Box::new(socket)),
                peer,
                endpoint,
                tls,
                certificate: None,
            };
            if client.endpoint.security == Security::Implicit {
                client.upgrade().await?;
            }
            let mut greeting = reply(client.socket()?).await?;
            if greeting.code == 120 {
                greeting = reply(client.socket()?).await?;
            }
            expect(greeting, &[220], "greeting")?;
            if client.endpoint.security == Security::Explicit {
                expect(
                    client.command("AUTH", "TLS").await?,
                    &[234],
                    "TLS upgrade (no plaintext fallback)",
                )?;
                client.upgrade().await?;
            }
            if client.endpoint.security != Security::Plain {
                expect(
                    client.command("PBSZ", "0").await?,
                    &[200],
                    "TLS buffer negotiation",
                )?;
                expect(
                    client.command("PROT", "P").await?,
                    &[200],
                    "private data-channel protection",
                )?;
            }
            let (user, password) = credentials.map_or(
                Ok(("anonymous", "download-engine@")),
                Credentials::ftp_login,
            )?;
            let login = client.command("USER", user).await?;
            if login.code == 331 {
                expect(client.command("PASS", password).await?, &[230], "login")?;
            } else {
                expect(login, &[230], "login")?;
            }
            expect(client.command("TYPE", "I").await?, &[200], "binary mode")?;
            let utf8 = client.command("OPTS", "UTF8 ON").await?;
            expect(utf8, &[200, 202, 500, 501, 502, 504], "UTF-8 negotiation")?;
            for directory in client.endpoint.directories.clone() {
                expect(
                    client.command("CWD", &directory).await?,
                    &[250],
                    "file directory",
                )?;
            }
            Ok(client)
        })
        .await
    }
    fn socket(&mut self) -> Result<&mut Stream> {
        self.stream.as_mut().context("FTP connection is closed")
    }
    async fn upgrade(&mut self) -> Result<()> {
        let (stream, certificate) = secure(
            self.stream.take().context("FTP connection is closed")?,
            &self.tls,
            &self.endpoint.host,
        )
        .await?;
        self.stream = Some(stream);
        self.certificate = Some(certificate);
        Ok(())
    }
    async fn command(&mut self, verb: &str, argument: &str) -> Result<Reply> {
        ensure!(
            argument.len() <= 4096 && !argument.chars().any(char::is_control),
            "Invalid FTP command argument"
        );
        let command = Zeroizing::new(if argument.is_empty() {
            format!("{verb}\r\n")
        } else {
            format!("{verb} {argument}\r\n")
        });
        self.socket()?
            .write_all(command.as_bytes())
            .await
            .map_err(network)?;
        self.socket()?.flush().await.map_err(network)?;
        reply(self.socket()?).await
    }
    pub(super) async fn metadata(&mut self, cancel: &CancellationToken) -> Result<Metadata> {
        wait(cancel, async {
            let path = self.endpoint.path.clone();
            let size = match self.command("SIZE", &path).await? {
                Reply { code: 213, text } => {
                    ensure!(
                        !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()),
                        "Invalid FTP file size"
                    );
                    let value = text.parse::<u64>().context("Invalid FTP file size")?;
                    ensure!(value <= i64::MAX as u64, "FTP file is too large");
                    Some(value)
                }
                Reply {
                    code: 500 | 502 | 504,
                    ..
                } => None,
                r => return Err(unexpected(r.code, "file size")),
            };
            let modified = match self.command("MDTM", &path).await? {
                Reply { code: 213, text } => {
                    ensure!(valid_timestamp(&text), "Invalid FTP modification time");
                    Some(text)
                }
                Reply {
                    code: 500 | 502 | 504 | 550,
                    ..
                } => None,
                r => return Err(unexpected(r.code, "modification time")),
            };
            Ok(Metadata { size, modified })
        })
        .await
    }
    pub(super) async fn retrieve(
        &mut self,
        requested_offset: u64,
        cancel: &CancellationToken,
    ) -> Result<(Stream, u64)> {
        wait(cancel, async {
            let port = match self.command("EPSV", "").await? {
                Reply { code: 229, text } => epsv_port(&text)?,
                Reply {
                    code: 500 | 501 | 502 | 504 | 522,
                    ..
                } if self.peer.is_ipv4() => {
                    let response = expect(
                        self.command("PASV", "").await?,
                        &[227],
                        "passive connection",
                    )?;
                    pasv_port(&response.text)?
                }
                r => return Err(unexpected(r.code, "passive connection")),
            };
            // Never follow a server-supplied PASV IP or resolve DNS again. Only
            // connect back to the actual control peer, on a non-privileged port.
            ensure!(
                port >= 1024 && port != self.peer.port(),
                "FTP passive data port is unsafe"
            );
            let data = TcpStream::connect(SocketAddr::new(self.peer.ip(), port))
                .await
                .map_err(network)?;
            let mut offset = 0;
            if requested_offset > 0 {
                match self.command("REST", &requested_offset.to_string()).await? {
                    Reply { code: 350, .. } => offset = requested_offset,
                    Reply {
                        code: 500 | 501 | 502 | 504,
                        ..
                    } => {}
                    r => return Err(unexpected(r.code, "resume")),
                }
            }
            let path = self.endpoint.path.clone();
            expect(
                self.command("RETR", &path).await?,
                &[125, 150],
                "file transfer",
            )?;
            let data: Stream = if self.endpoint.security == Security::Plain {
                Box::new(data)
            } else {
                let (stream, certificate) =
                    secure(Box::new(data), &self.tls, &self.endpoint.host).await?;
                ensure!(
                    self.certificate.as_ref() == Some(&certificate),
                    "FTPS data connection presented a different server certificate"
                );
                stream
            };
            Ok((data, offset))
        })
        .await
    }
    pub(super) async fn finish(&mut self, cancel: &CancellationToken) -> Result<()> {
        wait(cancel, async {
            expect(
                reply(self.socket()?).await?,
                &[226, 250],
                "transfer completion",
            )?;
            Ok(())
        })
        .await
    }
}
pub(super) async fn read(
    stream: &mut Stream,
    buffer: &mut [u8],
    cancel: &CancellationToken,
) -> Result<usize> {
    wait(cancel, async { stream.read(buffer).await.map_err(network) }).await
}
fn epsv_port(text: &str) -> Result<u16> {
    let (_, value) = text.split_once('(').context("Invalid FTP EPSV response")?;
    let (value, _) = value.split_once(')').context("Invalid FTP EPSV response")?;
    let delimiter = *value
        .as_bytes()
        .first()
        .context("Invalid FTP EPSV response")?;
    ensure!(
        delimiter.is_ascii_graphic() && !delimiter.is_ascii_digit(),
        "Invalid FTP EPSV delimiter"
    );
    let fields: Vec<_> = value.split(delimiter as char).collect();
    ensure!(
        fields.len() == 5
            && fields[0].is_empty()
            && fields[1].is_empty()
            && fields[2].is_empty()
            && fields[4].is_empty(),
        "Invalid FTP EPSV response"
    );
    port(fields[3])
}
fn pasv_port(text: &str) -> Result<u16> {
    let (_, value) = text.split_once('(').context("Invalid FTP PASV response")?;
    let (value, _) = value.split_once(')').context("Invalid FTP PASV response")?;
    let fields = value
        .split(',')
        .map(|v| {
            ensure!(
                !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()),
                "Invalid FTP PASV response"
            );
            Ok(v.parse::<u8>()?)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(fields.len() == 6, "Invalid FTP PASV response");
    Ok(u16::from(fields[4]) * 256 + u16::from(fields[5]))
}
fn port(value: &str) -> Result<u16> {
    ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "Invalid FTP passive port"
    );
    value.parse().context("Invalid FTP passive port")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mixed_resolution_is_rejected_before_connecting_but_local_ftp_still_works() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();
        let socket = connect_addresses(&[local]).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        drop((socket, accepted));

        let public = "8.8.8.8:21".parse().unwrap();
        // Check the loopback-first ordering first: the old implementation fails
        // here without ever attempting a connection to an external address.
        for addresses in [[local, public], [public, local]] {
            let error = connect_addresses(&addresses).await.unwrap_err();
            assert!(error.to_string().contains("public and local"));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "Rejected resolution still connected to the local service"
        );
    }

    #[tokio::test]
    async fn invalid_resolutions_fail_before_connecting() {
        let local: SocketAddr = "127.0.0.1:21".parse().unwrap();
        for addresses in [vec![], vec![local; 33], vec!["0.0.0.0:21".parse().unwrap()]] {
            assert!(connect_addresses(&addresses).await.is_err());
        }
    }

    #[test]
    fn passive_reply_parsers_are_bounded_and_ignore_the_advertised_host() {
        assert_eq!(
            epsv_port("Entering Extended Passive Mode (|||45000|)").unwrap(),
            45000
        );
        assert_eq!(
            pasv_port("Entering Passive Mode (10,0,0,1,175,200)").unwrap(),
            45000
        );
        for text in [
            "(|||65536|)",
            "(|||-1|)",
            "(|1||4000|)",
            "(|||4000|extra)",
            "(11140001)",
        ] {
            assert!(epsv_port(text).is_err());
        }
        for text in ["(1,2,3,4,5)", "(1,2,3,4,256,0)", "(1,2,3,4,-1,0)"] {
            assert!(pasv_port(text).is_err());
        }
    }
}
