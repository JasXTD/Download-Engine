//! Windows owns connection-bound NTLM/Negotiate handshakes. Every request has
//! its own session: no shared authentication or cookie state. Handles stay on
//! one blocking worker, including cancellation cleanup (WinHTTP sync contract).
#[cfg(test)]
mod tests;
use crate::{
    auth::{Credentials, Kind},
    proxy::{Authentication, Mode, Route},
};
use anyhow::{Context, Result, ensure};
use hyper::{
    body::{Body, Bytes, Frame, SizeHint},
    http,
};
use reqwest::{Request, Response, ResponseBuilderExt};
use std::{
    ffi::c_void,
    io,
    pin::Pin,
    ptr,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};
use tokio::sync::{Semaphore, mpsc, oneshot};
use windows_sys::Win32::{Foundation::GetLastError, Networking::WinHttp::*};
use zeroize::Zeroizing;

// Cover every scheduler slot at its supported per-file connection limit, plus
// one file's worth of previews/link checks. A smaller pool can strand admitted
// range requests behind long-lived bodies until their connection deadline.
static WORKERS: Semaphore = Semaphore::const_new(
    (crate::queue::MAX_ACTIVE + 1) * crate::connections::MAX_CONNECTIONS as usize,
);
type Chunk = std::result::Result<Frame<Bytes>, io::Error>;
struct Stream {
    receiver: mpsc::Receiver<Chunk>,
    remaining: Option<u64>,
}
impl Body for Stream {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Chunk>> {
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    self.remaining = self.remaining.map(|n| n.saturating_sub(data.len() as u64));
                }
                Poll::Ready(Some(Ok(frame)))
            }
            other => other,
        }
    }
    fn size_hint(&self) -> SizeHint {
        self.remaining
            .map_or_else(SizeHint::default, SizeHint::with_exact)
    }
}
struct Handle(*mut c_void);
impl Handle {
    fn new(raw: *mut c_void, operation: &str) -> Result<Self> {
        ensure!(!raw.is_null(), "{}", failure(operation));
        Ok(Self(raw))
    }
    fn option(&self, option: u32, value: u32) -> Result<()> {
        // The buffer is valid for the duration of this synchronous call.
        check(
            unsafe { WinHttpSetOption(self.0, option, (&value as *const u32).cast(), 4) },
            "setting request options",
        )
        .with_context(|| format!("Windows HTTP option {option}"))
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn wide(value: &str) -> Zeroizing<Vec<u16>> {
    Zeroizing::new(value.encode_utf16().chain(Some(0)).collect())
}
fn failure(operation: &str) -> io::Error {
    let code = unsafe { GetLastError() };
    let kind = match code {
        ERROR_WINHTTP_TIMEOUT => io::ErrorKind::TimedOut,
        ERROR_WINHTTP_CANNOT_CONNECT => io::ErrorKind::ConnectionRefused,
        ERROR_WINHTTP_CONNECTION_ERROR => io::ErrorKind::ConnectionReset,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(
        kind,
        format!("Windows HTTP failed while {operation} (code {code})"),
    )
}
fn check(ok: i32, operation: &str) -> Result<()> {
    if ok == 0 {
        Err(failure(operation).into())
    } else {
        Ok(())
    }
}
fn scheme(kind: Authentication) -> u32 {
    match kind {
        Authentication::Basic => WINHTTP_AUTH_SCHEME_BASIC,
        Authentication::Ntlm => WINHTTP_AUTH_SCHEME_NTLM,
        Authentication::Negotiate => WINHTTP_AUTH_SCHEME_NEGOTIATE,
    }
}
fn set_credentials(
    request: &Handle,
    target: u32,
    scheme: u32,
    user: &str,
    password: &str,
) -> Result<()> {
    let user = wide(user);
    let password = wide(password);
    check(
        unsafe {
            WinHttpSetCredentials(
                request.0,
                target,
                scheme,
                user.as_ptr(),
                password.as_ptr(),
                ptr::null_mut(),
            )
        },
        "setting explicit credentials",
    )
}

pub(crate) async fn send(
    route: Route,
    request: Request,
    server: Option<Arc<Credentials>>,
) -> Result<Response> {
    let permit = WORKERS
        .acquire()
        .await
        .context("Windows HTTP transport stopped")?;
    let (head_tx, head_rx) = oneshot::channel();
    let (body_tx, body_rx) = mpsc::channel(1);
    // This separate OS thread is bounded by WORKERS and does not hold a Tokio
    // runtime open after cancellation. All native waits have finite timeouts.
    let worker = std::thread::Builder::new()
        .name("enterprise-http".into())
        .spawn(move || {
            let _permit = permit;
            match open(&route, request, server.as_deref(), &head_tx) {
                Ok((session, connection, request, response, expected)) => {
                    if head_tx.send(Ok(response)).is_ok()
                        && let Err(error) = read(&request, expected, &body_tx)
                    {
                        let kind = error
                            .downcast_ref::<io::Error>()
                            .map_or(io::ErrorKind::Other, io::Error::kind);
                        let _ = body_tx.blocking_send(Err(io::Error::new(kind, error.to_string())));
                    }
                    drop(request);
                    drop(connection);
                    drop(session);
                }
                Err(error) => {
                    let transient = error.downcast_ref::<io::Error>().is_some_and(|e| {
                        matches!(
                            e.kind(),
                            io::ErrorKind::TimedOut
                                | io::ErrorKind::ConnectionRefused
                                | io::ErrorKind::ConnectionReset
                        )
                    });
                    let _ = head_tx.send(Err(if transient {
                        crate::retry::incomplete(&error.to_string())
                    } else {
                        error
                    }));
                }
            }
        })?;
    drop(worker); // Channel closure cancels; only its owning thread closes handles.
    let response: http::response::Builder =
        tokio::time::timeout(std::time::Duration::from_secs(30), head_rx)
            .await
            .map_err(|_| crate::retry::incomplete("Enterprise server connection timed out"))?
            .context("Enterprise HTTP worker stopped")??;
    let remaining = response
        .headers_ref()
        .and_then(|headers| headers.get(http::header::CONTENT_LENGTH))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    Ok(Response::from(response.body(reqwest::Body::wrap(
        Stream {
            receiver: body_rx,
            remaining,
        },
    ))?))
}

type Opened = (Handle, Handle, Handle, http::response::Builder, Option<u64>);
fn open(
    route: &Route,
    request: Request,
    server: Option<&Credentials>,
    pending: &oneshot::Sender<Result<http::response::Builder>>,
) -> Result<Opened> {
    let url = crate::model::validate_url(request.url().as_str())?;
    ensure!(
        route.settings.mode != Mode::System,
        "For enterprise authentication, choose Direct connection or a Custom HTTP proxy in Preferences → Connection → Proxy. Automatic proxy discovery cannot guarantee an explicit proxy login."
    );
    // WinHTTP ignores AUTOLOGON_HIGH for literal IPs and localhost. If a target
    // has no explicit account, require a hostname where that policy is enforced.
    let needs_hostname = |host: &str| {
        host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
            || host
                .trim_end_matches('.')
                .to_ascii_lowercase()
                .ends_with(".localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok()
    };
    if server.is_none() && route.login.is_some() {
        ensure!(
            !needs_hostname(url.host_str().unwrap_or_default()),
            "An enterprise proxy requires a destination hostname, not localhost or an IP address, when no file-server login is supplied."
        );
    }
    ensure!(
        matches!(request.method().as_str(), "GET" | "POST" | "HEAD"),
        "Unsupported enterprise HTTP method"
    );
    let body = request
        .body()
        .map(|b| b.as_bytes().context("Streaming uploads are not supported"))
        .transpose()?
        .unwrap_or_default();
    ensure!(
        body.len() <= 2 * 1024 * 1024,
        "Enterprise request body exceeds 2 MiB"
    );
    let proxy = if route.settings.mode == Mode::Custom {
        let proxy = crate::proxy::endpoint(&route.settings.endpoint)?;
        if route.login.is_none() {
            ensure!(
                !needs_hostname(proxy.host_str().unwrap_or_default()),
                "Use a proxy hostname or supply an explicit proxy login for enterprise downloads."
            );
        }
        ensure!(
            proxy.scheme() == "http",
            "Enterprise authentication supports Direct or HTTP proxies; select an http:// proxy endpoint."
        );
        wide(&format!(
            "{}:{}",
            proxy.host_str().context("Proxy has no host")?,
            proxy.port_or_known_default().context("Proxy has no port")?
        ))
    } else {
        wide("")
    };
    let access = match route.settings.mode {
        Mode::Direct => WINHTTP_ACCESS_TYPE_NO_PROXY,
        Mode::System => WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        Mode::Custom => WINHTTP_ACCESS_TYPE_NAMED_PROXY,
    };
    let agent = wide("Download Engine/1");
    let session = Handle::new(
        unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                access,
                if route.settings.mode == Mode::Custom {
                    proxy.as_ptr()
                } else {
                    ptr::null()
                },
                ptr::null(),
                0,
            )
        },
        "opening session",
    )?;
    check(
        unsafe { WinHttpSetTimeouts(session.0, 10_000, 15_000, 15_000, 15_000) },
        "setting timeouts",
    )?;
    session.option(WINHTTP_OPTION_CONNECT_RETRIES, 1)?;
    // Do not inherit legacy WinHTTP SSL/TLS protocol defaults. TLS 1.2 is
    // supported by every Windows version targeted by this desktop build.
    session.option(
        WINHTTP_OPTION_SECURE_PROTOCOLS,
        WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
    )?;
    session.option(WINHTTP_OPTION_USE_GLOBAL_SERVER_CREDENTIALS, 0)?;
    // Use a session-local connection limit. Unlike the newer pooling/GUID
    // options, these options are supported on Windows 10. Each session serves
    // one request (including its connection-bound authentication exchanges).
    session.option(WINHTTP_OPTION_MAX_CONNS_PER_SERVER, 1)?;
    session.option(WINHTTP_OPTION_MAX_CONNS_PER_1_0_SERVER, 1)?;
    let host = wide(
        url.host_str()
            .context("Request has no host")?
            .trim_matches(['[', ']']),
    );
    let connection = Handle::new(
        unsafe {
            WinHttpConnect(
                session.0,
                host.as_ptr(),
                url.port_or_known_default().context("Request has no port")?,
                0,
            )
        },
        "connecting",
    )?;
    let method = wide(request.method().as_str());
    let path = wide(&format!(
        "{}{}",
        url.path(),
        url.query().map_or(String::new(), |q| format!("?{q}"))
    ));
    let flags = if url.scheme() == "https" {
        WINHTTP_FLAG_SECURE
    } else {
        0
    } | WINHTTP_FLAG_ESCAPE_DISABLE
        | WINHTTP_FLAG_ESCAPE_DISABLE_QUERY;
    let handle = Handle::new(
        unsafe {
            WinHttpOpenRequest(
                connection.0,
                method.as_ptr(),
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                flags,
            )
        },
        "opening request",
    )?;
    handle.option(
        WINHTTP_OPTION_AUTOLOGON_POLICY,
        WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
    )?;
    handle.option(
        WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_DISABLE_COOKIES
            | WINHTTP_DISABLE_REDIRECTS
            | if server.is_none() && route.login.is_none() {
                WINHTTP_DISABLE_AUTHENTICATION
            } else {
                0
            },
    )?;
    handle.option(WINHTTP_OPTION_ENABLE_FEATURE, WINHTTP_ENABLE_SSL_REVOCATION)?;
    // HTTPS CONNECT is authenticated inside WinHttpSendRequest, before a
    // response is exposed to the caller. Supply the explicitly chosen proxy
    // account up front so WinHTTP can complete that tunnel handshake.
    if let Some(login) = &route.login {
        set_credentials(
            &handle,
            WINHTTP_AUTH_TARGET_PROXY,
            scheme(route.settings.authentication),
            &login.username,
            &login.password,
        )?;
    }
    if let Some(credentials) = server {
        let (kind, username, password) = credentials.enterprise_login()?;
        set_credentials(
            &handle,
            WINHTTP_AUTH_TARGET_SERVER,
            if kind == Kind::Ntlm {
                WINHTTP_AUTH_SCHEME_NTLM
            } else {
                WINHTTP_AUTH_SCHEME_NEGOTIATE
            },
            username,
            password,
        )?;
    }
    // Certificate validation is never relaxed. No default Windows credentials,
    // shared cookies or automatic cross-origin redirects are permitted.
    let mut headers = Zeroizing::new(String::new());
    for (name, value) in request.headers() {
        ensure!(
            name != http::header::PROXY_AUTHORIZATION,
            "Proxy credentials must not be origin headers"
        );
        if matches!(name.as_str(), "host" | "content-length" | "connection") {
            continue;
        }
        headers.push_str(name.as_str());
        headers.push_str(": ");
        headers.push_str(value.to_str().context("Non-text request header")?);
        headers.push_str("\r\n");
    }
    let headers = wide(&headers);
    let mut resends = 0;
    let status = loop {
        ensure!(!pending.is_closed(), "Enterprise request cancelled");
        let sent = unsafe {
            WinHttpSendRequest(
                handle.0,
                headers.as_ptr(),
                (headers.len() - 1) as u32,
                body.as_ptr().cast(),
                body.len() as u32,
                body.len() as u32,
                0,
            )
        };
        let received =
            sent != 0 && unsafe { WinHttpReceiveResponse(handle.0, ptr::null_mut()) } != 0;
        if !received {
            // WinHTTP may request a resend during an authentication exchange.
            // Bound resends; a final 401/407 is returned without another login.
            if unsafe { GetLastError() } == ERROR_WINHTTP_RESEND_REQUEST && resends < 3 {
                resends += 1;
                continue;
            }
            return Err(failure("sending or receiving response").into());
        }
        let mut status = 0u32;
        let mut size = 4;
        check(
            unsafe {
                WinHttpQueryHeaders(
                    handle.0,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    ptr::null(),
                    (&mut status as *mut u32).cast(),
                    &mut size,
                    ptr::null_mut(),
                )
            },
            "reading response status",
        )?;
        let expected = match status {
            401 if server.is_some() => Some((
                WINHTTP_AUTH_TARGET_SERVER,
                if server.unwrap().enterprise_login()?.0 == Kind::Ntlm {
                    WINHTTP_AUTH_SCHEME_NTLM
                } else {
                    WINHTTP_AUTH_SCHEME_NEGOTIATE
                },
            )),
            407 if route.login.is_some() => Some((
                WINHTTP_AUTH_TARGET_PROXY,
                scheme(route.settings.authentication),
            )),
            _ => None,
        };
        if let Some((target, desired)) = expected {
            let (mut supported, mut first, mut actual) = (0, 0, 0);
            check(
                unsafe {
                    WinHttpQueryAuthSchemes(handle.0, &mut supported, &mut first, &mut actual)
                },
                "reading authentication challenge",
            )?;
            ensure!(
                actual == target && supported & desired != 0,
                "The server or proxy does not offer the selected authentication method"
            );
        }
        break status;
    };
    let mut bytes = 0;
    unsafe {
        WinHttpQueryHeaders(
            handle.0,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            ptr::null(),
            ptr::null_mut(),
            &mut bytes,
            ptr::null_mut(),
        );
    }
    ensure!(
        bytes > 0 && bytes <= 64 * 1024 && bytes % 2 == 0,
        "Invalid or oversized enterprise response headers"
    );
    let mut raw = vec![0u16; bytes as usize / 2];
    check(
        unsafe {
            WinHttpQueryHeaders(
                handle.0,
                WINHTTP_QUERY_RAW_HEADERS_CRLF,
                ptr::null(),
                raw.as_mut_ptr().cast(),
                &mut bytes,
                ptr::null_mut(),
            )
        },
        "reading response headers",
    )?;
    let text = String::from_utf16(&raw).context("Invalid response headers")?;
    let mut response = http::Response::builder().status(status as u16).url(url);
    let mut expected = None;
    for line in text
        .trim_end_matches('\0')
        .split("\r\n")
        .skip(1)
        .filter(|s| !s.is_empty())
    {
        let (name, value) = line.split_once(':').context("Malformed response header")?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            let length: u64 = value.parse().context("Invalid response length")?;
            ensure!(expected.is_none(), "Duplicate response length");
            expected = Some(length);
        }
        response = response.header(name, value);
    }
    if request.method() == http::Method::HEAD {
        expected = Some(0);
    }
    Ok((session, connection, handle, response, expected))
}

fn read(handle: &Handle, mut remaining: Option<u64>, sender: &mpsc::Sender<Chunk>) -> Result<()> {
    while !sender.is_closed() && remaining != Some(0) {
        let mut data = vec![0u8; 64 * 1024];
        let mut count = 0;
        check(
            unsafe {
                WinHttpReadData(
                    handle.0,
                    data.as_mut_ptr().cast(),
                    data.len() as u32,
                    &mut count,
                )
            },
            "reading response body",
        )?;
        if count == 0 {
            if remaining.is_some_and(|n| n != 0) {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Enterprise response ended before its declared length",
                )
                .into());
            }
            break;
        }
        if let Some(left) = &mut remaining {
            ensure!(
                u64::from(count) <= *left,
                "Enterprise response exceeded its declared length"
            );
            *left -= u64::from(count);
        }
        data.truncate(count as usize);
        if sender
            .blocking_send(Ok(Frame::data(Bytes::from(data))))
            .is_err()
        {
            break;
        }
    }
    Ok(())
}
