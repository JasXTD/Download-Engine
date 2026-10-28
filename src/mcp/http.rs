//! A bounded loopback HTTP listener. Client lifetimes never own the server process.
use super::*;
use anyhow::{Context as _, ensure};
use base64::Engine as _;
use http_body_util::BodyExt;
use hyper::{
    Request, Response, StatusCode,
    body::{Body, Bytes, Frame, Incoming},
    header::HeaderMap,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    collections::HashMap,
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub const DEFAULT_PORT: u16 = 17842;
const MAX_CONNECTIONS: usize = 32;
const MAX_SESSIONS: usize = 32;
const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);

struct Session {
    protocol: Protocol,
    version: String,
    used: Instant,
}
#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Session>,
    active: HashMap<(String, String), Arc<AtomicBool>>,
}
impl Registry {
    fn expire(&mut self) {
        self.sessions
            .retain(|_, session| session.used.elapsed() < SESSION_IDLE);
        for ((session, _), token) in &self.active {
            if !session.is_empty() && !self.sessions.contains_key(session) {
                token.store(true, Ordering::Release);
            }
        }
    }
    fn cancel_session(&mut self, id: &str) {
        self.sessions.remove(id);
        for ((session, _), token) in &self.active {
            if session == id {
                token.store(true, Ordering::Release);
            }
        }
    }
}
struct State {
    context: Context,
    port: u16,
    token: Zeroizing<String>,
    registry: Mutex<Registry>,
    slots: Arc<Semaphore>,
}

// Dropping a modern request's response stream cancels its work. Legacy streams
// instead require an explicit cancellation notification, as that protocol specifies.
struct ResponseBody {
    data: Option<Bytes>,
    stream: Option<mpsc::Receiver<Bytes>>,
    cancel: Option<Arc<AtomicBool>>,
}
impl Body for ResponseBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Bytes>, Infallible>>> {
        if let Some(data) = self.data.take() {
            return Poll::Ready(Some(Ok(Frame::data(data))));
        }
        match &mut self.stream {
            Some(rx) => rx
                .poll_recv(cx)
                .map(|v| v.map(|data| Ok(Frame::data(data)))),
            None => Poll::Ready(None),
        }
    }
    fn is_end_stream(&self) -> bool {
        self.data.is_none() && self.stream.is_none()
    }
}
impl Drop for ResponseBody {
    fn drop(&mut self) {
        if let Some(token) = &self.cancel {
            token.store(true, Ordering::Release);
        }
    }
}
type HttpResponse = Response<ResponseBody>;
fn reply(status: StatusCode, value: Option<Value>) -> HttpResponse {
    let mut response = Response::new(ResponseBody {
        data: value.map(|v| Bytes::from(serde_json::to_vec(&v).expect("JSON value"))),
        stream: None,
        cancel: None,
    });
    *response.status_mut() = status;
    let h = response.headers_mut();
    h.insert("content-type", "application/json".parse().unwrap());
    h.insert("cache-control", "no-store".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    // One request per connection makes disconnect cancellation unambiguous and
    // bounds idle sockets. Clients reconnect without restarting this server.
    h.insert("connection", "close".parse().unwrap());
    response
}
fn error(status: StatusCode, id: Value, code: i32, message: &str) -> HttpResponse {
    reply(status, Some(rpc_error(id, code, message)))
}
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}
fn decoded_header(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = header(headers, name)?;
    if let Some(encoded) = value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
    {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?,
        )
        .ok()
    } else if value.trim() == value && value.bytes().all(|v| (32..=126).contains(&v) || v == b'\t')
    {
        Some(value.into())
    } else {
        None
    }
}
fn local_headers(
    headers: &HeaderMap,
    port: u16,
    token: &str,
) -> std::result::Result<(), StatusCode> {
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !header(headers, "host").is_some_and(|v| hosts.iter().any(|h| h == v)) {
        return Err(StatusCode::FORBIDDEN);
    }
    if headers.contains_key("origin")
        && !header(headers, "origin")
            .is_some_and(|v| hosts.iter().any(|h| v == format!("http://{h}")))
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let supplied = header(headers, "authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if supplied.len() != token.len()
        || supplied
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            != 0
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

fn dispatch(
    state: &State,
    headers: &HeaderMap,
    message: Value,
) -> std::result::Result<(Dispatch, String, Option<String>), Box<HttpResponse>> {
    let id = message
        .get("id")
        .filter(|v| valid_id(v))
        .cloned()
        .unwrap_or(Value::Null);
    let mismatch = || {
        Box::new(error(
            StatusCode::BAD_REQUEST,
            id.clone(),
            -32020,
            "Required MCP headers are missing, malformed or do not match the request",
        ))
    };
    let version = header(headers, "mcp-protocol-version");
    let body_version = message["params"]["_meta"].get(VERSION_META);
    let modern = version == Some(MODERN) || body_version.is_some();
    if modern {
        let requested = body_version.and_then(Value::as_str);
        if version.is_none() || version != requested {
            return Err(mismatch());
        }
        if version != Some(MODERN) {
            let mut value = rpc_error(id, -32022, "Unsupported protocol version");
            value["error"]["data"] = json!({"supported":supported(),"requested":requested});
            return Err(Box::new(reply(StatusCode::BAD_REQUEST, Some(value))));
        }
        if message.get("id").is_some() {
            if header(headers, "mcp-method") != message["method"].as_str()
                || header(headers, "mcp-method").is_none()
            {
                return Err(mismatch());
            }
            if matches!(
                message["method"].as_str(),
                Some("tools/call" | "resources/read" | "prompts/get")
            ) {
                let expected = if message["method"] == "resources/read" {
                    message["params"]["uri"].as_str()
                } else {
                    message["params"]["name"].as_str()
                };
                if expected.is_none() || decoded_header(headers, "mcp-name").as_deref() != expected
                {
                    return Err(mismatch());
                }
            }
        }
        if message["method"] == "initialize" {
            return Err(Box::new(error(
                StatusCode::NOT_FOUND,
                id,
                -32601,
                "Modern requests use server/discover",
            )));
        }
        let dispatch = Protocol::default().accept(message, state.context.access());
        // Modern requests are independent; identical JSON-RPC IDs from different
        // clients must not collide or cancel each other.
        return Ok((dispatch, String::new(), None));
    }
    if let Some(version) = version
        && !LEGACY.contains(&version)
    {
        let mut value = rpc_error(id, -32022, "Unsupported protocol version");
        value["error"]["data"] = json!({"supported":supported(),"requested":version});
        return Err(Box::new(reply(StatusCode::BAD_REQUEST, Some(value))));
    }
    let mut registry = state.registry.lock().unwrap();
    registry.expire();
    if message["method"] == "initialize" {
        if headers.contains_key("mcp-session-id") {
            return Err(Box::new(error(
                StatusCode::BAD_REQUEST,
                id,
                -32600,
                "Initialize without a session ID",
            )));
        }
        if registry.sessions.len() >= MAX_SESSIONS {
            return Err(Box::new(error(
                StatusCode::SERVICE_UNAVAILABLE,
                id,
                -32603,
                "Too many sessions; close an idle client and retry",
            )));
        }
        let mut protocol = Protocol::default();
        let dispatch = protocol.accept(message, state.context.access());
        if let Dispatch::Reply(value) = &dispatch
            && let Some(version) = value["result"]["protocolVersion"].as_str()
        {
            let session = uuid::Uuid::new_v4().to_string();
            registry.sessions.insert(
                session.clone(),
                Session {
                    protocol,
                    version: version.into(),
                    used: Instant::now(),
                },
            );
            return Ok((dispatch, session.clone(), Some(session)));
        }
        return Ok((dispatch, String::new(), None));
    }
    let Some(session_id) = header(headers, "mcp-session-id") else {
        return Err(mismatch());
    };
    let Some(session) = registry.sessions.get_mut(session_id) else {
        return Err(Box::new(error(
            StatusCode::NOT_FOUND,
            id,
            -32600,
            "Session expired; initialize again",
        )));
    };
    if version != Some(session.version.as_str()) {
        return Err(mismatch());
    }
    session.used = Instant::now();
    Ok((
        session.protocol.accept(message, state.context.access()),
        session_id.into(),
        None,
    ))
}

async fn handle(request: Request<Incoming>, state: Arc<State>) -> HttpResponse {
    let (parts, body) = request.into_parts();
    if let Err(status) = local_headers(&parts.headers, state.port, &state.token) {
        let mut response = error(
            status,
            Value::Null,
            -32600,
            "Local host, origin or connection token rejected",
        );
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                "www-authenticate",
                "Bearer realm=\"Download Engine\"".parse().unwrap(),
            );
        }
        return response;
    }
    if parts.uri.path() != "/mcp" || parts.uri.query().is_some() || parts.uri.authority().is_some()
    {
        return error(
            StatusCode::NOT_FOUND,
            Value::Null,
            -32601,
            "Use the /mcp endpoint",
        );
    }
    if parts.method == hyper::Method::DELETE
        && header(&parts.headers, "mcp-protocol-version") != Some(MODERN)
    {
        let mut registry = state.registry.lock().unwrap();
        if let Some(id) = header(&parts.headers, "mcp-session-id")
            && registry.sessions.contains_key(id)
        {
            registry.cancel_session(id);
            return reply(StatusCode::NO_CONTENT, None);
        }
        return error(
            StatusCode::NOT_FOUND,
            Value::Null,
            -32600,
            "Session not found",
        );
    }
    if parts.method != hyper::Method::POST {
        let mut response = error(
            StatusCode::METHOD_NOT_ALLOWED,
            Value::Null,
            -32600,
            "Use POST for MCP requests",
        );
        response
            .headers_mut()
            .insert("allow", "POST, DELETE".parse().unwrap());
        return response;
    }
    if !header(&parts.headers, "content-type").is_some_and(|v| {
        v.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    }) {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Value::Null,
            -32600,
            "Use application/json",
        );
    }
    let accept: Vec<_> = parts
        .headers
        .get_all("accept")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|v| v.split(';').next().unwrap_or("").trim())
        .collect();
    if !accept.contains(&"application/json") || !accept.contains(&"text/event-stream") {
        return error(
            StatusCode::NOT_ACCEPTABLE,
            Value::Null,
            -32600,
            "Accept application/json and text/event-stream",
        );
    }
    let bytes = match tokio::time::timeout(
        Duration::from_secs(8),
        http_body_util::Limited::new(body, MAX_FRAME).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                Value::Null,
                -32600,
                "Invalid body or body exceeds 256 KiB",
            );
        }
        Err(_) => {
            return error(
                StatusCode::REQUEST_TIMEOUT,
                Value::Null,
                -32600,
                "Request body timed out",
            );
        }
    };
    let message: Value = match serde_json::from_slice(&bytes) {
        Ok(message) => message,
        Err(_) => return error(StatusCode::BAD_REQUEST, Value::Null, -32700, "Parse error"),
    };
    if !message.is_object() || message["jsonrpc"] != "2.0" || !message["method"].is_string() {
        return error(
            StatusCode::BAD_REQUEST,
            Value::Null,
            -32600,
            "Invalid JSON-RPC request",
        );
    }
    let (dispatch, session, new_session) = match dispatch(&state, &parts.headers, message) {
        Ok(dispatch) => dispatch,
        Err(response) => return *response,
    };
    match dispatch {
        Dispatch::Ignore => reply(StatusCode::ACCEPTED, None),
        Dispatch::Cancel(id) => {
            // Modern cancellation is the response stream closing. Never allow a
            // modern notification to cancel another client's same-numbered call.
            if !session.is_empty()
                && let Some(token) = state.registry.lock().unwrap().active.get(&(session, id))
            {
                token.store(true, Ordering::Release);
            }
            reply(StatusCode::ACCEPTED, None)
        }
        Dispatch::Reply(value) => {
            let status = match value["error"]["code"].as_i64() {
                Some(-32601) => StatusCode::NOT_FOUND,
                Some(_) => StatusCode::BAD_REQUEST,
                None => StatusCode::OK,
            };
            let mut response = reply(status, Some(value));
            if let Some(session) = new_session {
                response
                    .headers_mut()
                    .insert("mcp-session-id", session.parse().unwrap());
            }
            response
        }
        Dispatch::Call {
            id,
            modern,
            name,
            arguments,
        } => {
            let Ok(permit) = state.slots.clone().try_acquire_owned() else {
                let mut response = error(
                    StatusCode::TOO_MANY_REQUESTS,
                    id,
                    -32603,
                    "Four calls are active; retry later using the same durable request_id",
                );
                response
                    .headers_mut()
                    .insert("retry-after", "1".parse().unwrap());
                return response;
            };
            let request_key = (
                session,
                if modern {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    key(&id)
                },
            );
            let cancelled = Arc::new(AtomicBool::new(false));
            {
                let mut registry = state.registry.lock().unwrap();
                if !request_key.0.is_empty() && !registry.sessions.contains_key(&request_key.0) {
                    return error(
                        StatusCode::NOT_FOUND,
                        id,
                        -32600,
                        "Session ended before execution",
                    );
                }
                if registry.active.contains_key(&request_key) {
                    return error(
                        StatusCode::BAD_REQUEST,
                        id,
                        -32600,
                        "Request ID is already in flight",
                    );
                }
                registry
                    .active
                    .insert(request_key.clone(), cancelled.clone());
            }
            let (tx, rx) = mpsc::channel(2);
            tx.try_send(Bytes::from_static(b": ready\n\n"))
                .expect("empty channel");
            let token = cancelled.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tools::call(&state.context, &name, &arguments, &token)
                }))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "Internal MCP worker panicked; inspect the server console"
                    ))
                });
                state.registry.lock().unwrap().active.remove(&request_key);
                if !token.load(Ordering::Acquire) {
                    let value = response(id, tool_result(result), modern);
                    let _ = tx.blocking_send(Bytes::from(format!(
                        "event: message\ndata: {}\n\n",
                        serde_json::to_string(&value).expect("JSON value")
                    )));
                }
            });
            let mut response = reply(StatusCode::OK, None);
            *response.body_mut() = ResponseBody {
                data: None,
                stream: Some(rx),
                cancel: modern.then_some(cancelled),
            };
            response
                .headers_mut()
                .insert("content-type", "text/event-stream".parse().unwrap());
            response
                .headers_mut()
                .insert("x-accel-buffering", "no".parse().unwrap());
            response
        }
    }
}

async fn serve(
    listener: TcpListener,
    state: Arc<State>,
    shutdown: CancellationToken,
) -> Result<()> {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let (socket,peer) = match accepted {
                    Ok(value) => value,
                    Err(error) => {
                        // Always cancel waiting tools before shutting down the runtime.
                        for token in state.registry.lock().unwrap().active.values() {
                            token.store(true, Ordering::Release);
                        }
                        return Err(error.into());
                    }
                };
                if !peer.ip().is_loopback() {continue;}
                let Ok(permit) = slots.clone().try_acquire_owned() else {continue;};
                let state = state.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    let service = hyper::service::service_fn(move |request| {
                        let state = state.clone();
                        async move { Ok::<_,Infallible>(handle(request,state).await) }
                    });
                    let mut builder = hyper::server::conn::http1::Builder::new();
                    builder.timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(5))
                        .max_headers(32).max_buf_size(16*1024).keep_alive(false);
                    // Also release a slot if an authenticated peer stops reading
                    // a large response after its tool has already completed.
                    let _ = tokio::time::timeout(Duration::from_secs(60),
                        builder.serve_connection(TokioIo::new(socket),service)).await;
                });
            }
        }
    }
    for token in state.registry.lock().unwrap().active.values() {
        token.store(true, Ordering::Release);
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

pub fn client_config(paths: &AppPaths, port: u16) -> Result<Value> {
    ensure!(port != 0, "MCP port must be 1..65535");
    let token = auth::token(paths)?;
    Ok(
        json!({"mcpServers":{"download-engine":{"url":format!("http://127.0.0.1:{port}/mcp"),"headers":{"Authorization":format!("Bearer {}",token.as_str())}}}}),
    )
}

/// Shared by the terminal and desktop owners. Never installs a console handler.
pub(super) fn run_owned(
    context: Context,
    port: u16,
    shutdown: CancellationToken,
    ready: impl FnOnce(),
) -> Result<()> {
    ensure!(port != 0, "MCP port must be 1..65535");
    if shutdown.is_cancelled() {
        return Ok(());
    }
    let token = auth::token(&context.store.paths)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        if shutdown.is_cancelled() { return Ok(()); }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
            .with_context(|| format!("Cannot listen on 127.0.0.1:{port}. Another server may be using this port; stop it or choose another port."))?;
        let state = Arc::new(State {context,port,token,registry:Mutex::new(Registry::default()),slots:Arc::new(Semaphore::new(MAX_ACTIVE))});
        ready();
        serve(listener,state,shutdown).await
    });
    runtime.shutdown_timeout(Duration::from_secs(15));
    result
}

#[cfg(windows)]
pub fn run(context: Context, port: u16) -> Result<()> {
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };
    static STOP: AtomicBool = AtomicBool::new(false);
    unsafe extern "system" fn stop(event: u32) -> i32 {
        if matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
            STOP.store(true, Ordering::Release);
            1
        } else {
            0
        }
    }
    struct Handler;
    impl Drop for Handler {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCtrlHandler(Some(stop), 0);
            }
        }
    }
    STOP.store(false, Ordering::Release);
    ensure!(
        unsafe { SetConsoleCtrlHandler(Some(stop), 1) } != 0,
        "Cannot install MCP stop handler: {}",
        std::io::Error::last_os_error()
    );
    let _handler = Handler;
    let read_only = context.read_only;
    let mut server = Server::default();
    server.start(context, port)?;
    let mut announced = false;
    loop {
        match server.status() {
            ServerStatus::Running if !announced => {
                announced = true;
                eprintln!(
                    "Download Engine MCP {} listening at http://127.0.0.1:{port}/mcp ({}). Run mcp-config to get connection settings. Ctrl+C stops this server.",
                    env!("CARGO_PKG_VERSION"),
                    if read_only {
                        "read-only"
                    } else {
                        "control enabled"
                    }
                );
            }
            ServerStatus::Failed(error) => anyhow::bail!("{error}"),
            ServerStatus::Stopped => return Ok(()),
            _ => {}
        }
        if STOP.load(Ordering::Acquire) {
            server.stop();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
#[cfg(not(windows))]
pub fn run(_context: Context, _port: u16) -> Result<()> {
    anyhow::bail!("Download Engine MCP requires Windows protected token storage")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_security_headers_and_invalid_name_encodings_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:17842".parse().unwrap());
        headers.insert("authorization", "Bearer fixture".parse().unwrap());
        assert_eq!(local_headers(&headers, 17842, "fixture"), Ok(()));
        for (name, value, status) in [
            ("host", "evil.example", StatusCode::FORBIDDEN),
            ("authorization", "Bearer fixture", StatusCode::UNAUTHORIZED),
            ("origin", "http://127.0.0.1:17842", StatusCode::FORBIDDEN),
        ] {
            let mut repeated = headers.clone();
            if name == "origin" {
                repeated.insert(name, value.parse().unwrap());
            }
            repeated.append(name, value.parse().unwrap());
            assert_eq!(local_headers(&repeated, 17842, "fixture"), Err(status));
        }
        headers.insert("mcp-name", "=?base64?%%%?=".parse().unwrap());
        assert!(decoded_header(&headers, "mcp-name").is_none());
        headers.insert("mcp-name", "=?base64?bGlzdF9xdWV1ZXM=?=".parse().unwrap());
        assert_eq!(
            decoded_header(&headers, "mcp-name").as_deref(),
            Some("list_queues")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn separate_sessions_can_reuse_rpc_ids_and_shutdown_cancels_remaining_work() {
        let (_temp, context) = super::super::tests::fixture();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(State {
            context,
            port,
            token: Zeroizing::new("fixture".into()),
            registry: Mutex::new(Registry::default()),
            slots: Arc::new(Semaphore::new(MAX_ACTIVE)),
        });
        let shutdown = CancellationToken::new();
        let server = tokio::spawn(serve(listener, state.clone(), shutdown.clone()));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let url = format!("http://127.0.0.1:{port}/mcp");
        let request = || {
            client
                .post(&url)
                .bearer_auth("fixture")
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
        };
        let mut sessions = Vec::new();
        for _ in 0..2 {
            let init = request().body(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":LEGACY[0],"capabilities":{},"clientInfo":{"name":"test","version":"1"}}}).to_string()).send().await.unwrap();
            assert_eq!(init.status(), StatusCode::OK);
            let id = init.headers()["mcp-session-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let initialized = request()
                .header("mcp-session-id", &id)
                .header("mcp-protocol-version", LEGACY[0])
                .body(json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string())
                .send()
                .await
                .unwrap();
            assert_eq!(initialized.status(), StatusCode::ACCEPTED);
            sessions.push(id);
        }
        let mut responses = Vec::new();
        let mut ids = Vec::new();
        for session in &sessions {
            let id = uuid::Uuid::new_v4().to_string();
            let response = request().header("mcp-session-id",session).header("mcp-protocol-version",LEGACY[0]).body(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"pause_all","arguments":{"request_id":id,"wait_seconds":30}}}).to_string()).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "text/event-stream");
            responses.push(response);
            ids.push(id);
        }
        async fn receipt(state: &State, id: &str, expected: &str) {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if state
                        .context
                        .store
                        .control_receipt(id)
                        .is_ok_and(|r| r.state == expected)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        for id in &ids {
            receipt(&state, id, "pending").await;
        }
        let cancel = request().header("mcp-session-id",&sessions[0]).header("mcp-protocol-version",LEGACY[0]).body(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}).to_string()).send().await.unwrap();
        assert_eq!(cancel.status(), StatusCode::ACCEPTED);
        receipt(&state, &ids[0], "cancelled").await;
        assert_eq!(
            state.context.store.control_receipt(&ids[1]).unwrap().state,
            "pending"
        );
        shutdown.cancel();
        server.await.unwrap().unwrap();
        receipt(&state, &ids[1], "cancelled").await;
        drop(responses);
        assert_eq!(state.context.store.queues().unwrap().len(), 1);
    }
}
