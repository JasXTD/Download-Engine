//! Asks the home router to forward the incoming port while torrents run (UPnP IGD).
//!
//! Off unless the person turns on both incoming connections and router setup. This is the
//! only engine code that may reach this computer's own network, and it reaches only the
//! default gateway named by the system's routing table. UPnP has no passwords, so any device
//! on the network could answer pretending to be the router. Everything read here is
//! checked: who answered, where its addresses lead, how large each answer is and which few
//! fields are used. It asks for one rule, for this computer only, that expires by itself, and
//! it never replaces or removes a rule that belongs to another device or to another run.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use librqbit_core::network::is_local_address;
use parking_lot::Mutex;
use quick_xml::Reader;
use quick_xml::events::{BytesRef, Event};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

/// How long a rule lasts unless it is renewed. After a crash, the router drops it by itself.
pub const LEASE: Duration = Duration::from_secs(60 * 60);
pub const RENEW_EVERY: Duration = Duration::from_secs(30 * 60);
/// Each request to the router, and the search for it, gives up after this.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);
/// No router description or answer comes anywhere near this size.
const MAX_ANSWER: usize = 64 * 1024;
/// A search answer is one small packet.
const MAX_SEARCH_ANSWER: usize = 2048;
const MAX_URL: usize = 1024;
const MAX_FIELD: usize = 1024;
const MAX_DEPTH: usize = 32;
const MAX_SERVICES: usize = 64;
const MAX_LEAVES: usize = 512;
/// Bad packets put up with during one search, such as port-unreachable replies.
const MAX_BAD_PACKETS: usize = 16;
/// After an attempt that could work later, the next ones wait this long. Then router setup
/// rests until the helper starts again.
const RETRIES: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(300),
    Duration::from_secs(900),
];
const SSDP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
const SEARCH_TARGETS: [&str; 2] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
];
/// Services that forward ports, best first.
const SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];
// The UPnP error codes acted on.
const NOT_AUTHORIZED: u32 = 606;
const NO_SUCH_ENTRY: u32 = 714;
const CONFLICT: u32 = 718;
const ONLY_PERMANENT: u32 = 725;

/// What router setup has come to, for the Preferences window. It carries only addresses and
/// a port, never text from the router.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RouterState {
    /// Looking for the router, or asking it.
    Looking,
    /// The router forwards `port` to this computer.
    Working { gateway: Ipv4Addr, port: u16 },
    /// No router answered, or it can't forward ports.
    NoRouter,
    /// Another device already has this port.
    Taken { port: u16, other: Option<Ipv4Addr> },
    /// The router only keeps rules forever, which would stay open after the app closes.
    PermanentOnly,
    /// The router's own outside address is shared or private, so a rule can't help.
    Shared { external: Ipv4Addr },
    /// The router said no.
    Refused,
    /// There is no home-network gateway to ask, or this system isn't supported.
    Unavailable,
    /// Something else went wrong while asking the router.
    Failed,
}

impl RouterState {
    /// Whether asking again later could help.
    fn worth_retrying(self) -> bool {
        matches!(self, Self::NoRouter | Self::Failed)
    }
}

/// The router to ask, and this computer's own address toward it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gateway {
    pub router: Ipv4Addr,
    pub me: Ipv4Addr,
}

impl Gateway {
    /// Both must be on a home network (RFC 1918). Anything else is never asked.
    pub fn checked(router: Ipv4Addr, me: Ipv4Addr) -> Option<Self> {
        (router.is_private() && me.is_private() && router != me).then_some(Self { router, me })
    }
}

/// A router address, only as plain http to the gateway's own IP. A relative address is
/// read against `base`. Anything else is refused.
pub(crate) fn checked_url(text: &str, base: Option<&Url>, gateway: Gateway) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_URL || text.chars().any(char::is_control) {
        return None;
    }
    let mut url = match base {
        Some(base) => base.join(text).ok()?,
        None => Url::parse(text).ok()?,
    };
    url.set_fragment(None);
    (url.scheme() == "http"
        && url.host() == Some(Host::Ipv4(gateway.router))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default().is_some_and(|port| port != 0)
        && url.as_str().len() <= MAX_URL)
        .then_some(url)
}

fn search_request(target: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n"
    )
}

/// The description address in a search answer. Only an answer from the gateway, naming a
/// gateway device and an address on the gateway itself, counts.
pub(crate) fn location_in_answer(answer: &[u8], from: SocketAddr, gateway: Gateway) -> Option<Url> {
    if from.ip().to_canonical() != IpAddr::V4(gateway.router) || answer.len() > MAX_SEARCH_ANSWER {
        return None;
    }
    let text = std::str::from_utf8(answer).ok()?;
    let mut lines = text.lines();
    let status = lines.next()?;
    if !(status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200")) {
        return None;
    }
    let (mut location, mut device) = (None, false);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "location" => location = Some(value),
            "st" | "nt" => {
                device |= value.contains(":device:InternetGatewayDevice:")
                    || SERVICES.contains(&value);
            }
            _ => {}
        }
    }
    if !device {
        return None;
    }
    checked_url(location?, None, gateway)
}

/// One element's text, with its local name and the service it belongs to, if any.
struct Leaf {
    name: String,
    service: Option<usize>,
    text: String,
}

/// The five entities XML defines, and character numbers. A document can't define its own,
/// because one with a DOCTYPE is refused.
fn entity(reference: &BytesRef<'_>) -> Option<char> {
    if reference.is_char_ref() {
        return reference.resolve_char_ref().ok().flatten();
    }
    match reference.decode().ok()?.as_ref() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

fn add_text(open: &mut [String], piece: &str) -> Option<()> {
    let text = open.last_mut()?;
    if text.len() + piece.len() > MAX_FIELD {
        return None;
    }
    text.push_str(piece);
    Some(())
}

/// Every element's text in a router's XML. A DOCTYPE, an unknown entity, deep nesting,
/// long text, too many parts or broken XML refuses the whole document.
fn leaves(xml: &[u8]) -> Option<Vec<Leaf>> {
    if xml.len() > MAX_ANSWER {
        return None;
    }
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let (mut names, mut open): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    // Which service is open, and how deep it started.
    let mut service: Option<(usize, usize)> = None;
    let mut services = 0;
    let mut found = Vec::new();
    loop {
        match reader.read_event().ok()? {
            Event::DocType(_) => return None,
            Event::Start(element) => {
                if names.len() >= MAX_DEPTH {
                    return None;
                }
                let name = std::str::from_utf8(element.local_name().as_ref())
                    .ok()?
                    .to_owned();
                if name == "service" && service.is_none() {
                    if services >= MAX_SERVICES {
                        return None;
                    }
                    service = Some((services, names.len()));
                    services += 1;
                }
                names.push(name);
                open.push(String::new());
            }
            Event::End(_) => {
                let (name, text) = (names.pop()?, open.pop()?);
                if !text.is_empty() {
                    if found.len() >= MAX_LEAVES {
                        return None;
                    }
                    found.push(Leaf {
                        name,
                        service: service.map(|(index, _)| index),
                        text,
                    });
                }
                if service.is_some_and(|(_, depth)| depth == names.len()) {
                    service = None;
                }
            }
            Event::Text(text) => add_text(&mut open, &text.decode().ok()?)?,
            Event::CData(text) => add_text(&mut open, &text.decode().ok()?)?,
            Event::GeneralRef(reference) => {
                add_text(&mut open, entity(&reference)?.encode_utf8(&mut [0; 4]))?;
            }
            Event::Eof => return names.is_empty().then_some(found),
            _ => {}
        }
    }
}

/// The router's port-forwarding service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Service {
    pub control: Url,
    pub kind: &'static str,
}

/// The best port-forwarding service in a router's description. A base address that leads
/// anywhere but the gateway refuses the whole description; a service that does is skipped.
pub(crate) fn find_service(xml: &[u8], location: &Url, gateway: Gateway) -> Option<Service> {
    let leaves = leaves(xml)?;
    let base = match leaves
        .iter()
        .find(|leaf| leaf.service.is_none() && leaf.name == "URLBase")
    {
        Some(leaf) => checked_url(&leaf.text, None, gateway)?,
        None => location.clone(),
    };
    let mut best: Option<(usize, Url)> = None;
    for index in 0..MAX_SERVICES {
        let field = |name: &str| {
            leaves
                .iter()
                .find(|leaf| leaf.service == Some(index) && leaf.name == name)
                .map(|leaf| leaf.text.as_str())
        };
        let (Some(kind), Some(control)) = (field("serviceType"), field("controlURL")) else {
            continue;
        };
        let Some(rank) = SERVICES.iter().position(|known| *known == kind) else {
            continue;
        };
        if best.as_ref().is_some_and(|(have, _)| *have <= rank) {
            continue;
        }
        if let Some(control) = checked_url(control, Some(&base), gateway) {
            best = Some((rank, control));
        }
    }
    best.map(|(rank, control)| Service {
        control,
        kind: SERVICES[rank],
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// A SOAP request. The values are numbers, this computer's address and a fixed name,
/// escaped all the same.
fn envelope(service: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut body = format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">"
    );
    for (name, value) in args {
        body.push_str(&format!("<{name}>{}</{name}>", escape(value)));
    }
    body.push_str(&format!("</u:{action}></s:Body></s:Envelope>"));
    body
}

/// What a SOAP answer said: the asked-for fields, or the router's error code.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Fields(Vec<(String, String)>),
    Fault(Option<u32>),
}

pub(crate) fn read_answer(status: u16, xml: &[u8], wanted: &[&str]) -> Option<Answer> {
    let leaves = leaves(xml)?;
    match status {
        200 => Some(Answer::Fields(
            leaves
                .into_iter()
                .filter(|leaf| wanted.contains(&leaf.name.as_str()))
                .map(|leaf| (leaf.name, leaf.text))
                .collect(),
        )),
        500 => Some(Answer::Fault(
            leaves
                .iter()
                .find(|leaf| leaf.name == "errorCode")
                .and_then(|leaf| leaf.text.trim().parse().ok()),
        )),
        _ => None,
    }
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.as_str())
}

/// How router setup reaches the network. Tests use a scripted router instead.
pub trait Transport: Send + Sync {
    /// Looks for the gateway device and returns its checked description address.
    fn search(&self, gateway: Gateway) -> impl Future<Output = Option<Url>> + Send;
    /// The body of a 200 answer, at most `MAX_ANSWER` bytes.
    fn get(&self, url: &Url) -> impl Future<Output = Result<Vec<u8>>> + Send;
    /// Sends a SOAP action, and returns the status and at most `MAX_ANSWER` bytes.
    fn soap(
        &self,
        url: &Url,
        action: &str,
        body: String,
    ) -> impl Future<Output = Result<(u16, Vec<u8>)>> + Send;
}

async fn ask<T: Transport>(
    transport: &T,
    service: &Service,
    action: &str,
    args: &[(&str, String)],
    wanted: &[&str],
) -> std::result::Result<Answer, RouterState> {
    let body = envelope(service.kind, action, args);
    let header = format!("\"{}#{action}\"", service.kind);
    let (status, answer) = transport
        .soap(&service.control, &header, body)
        .await
        .map_err(|_| RouterState::Failed)?;
    read_answer(status, &answer, wanted).ok_or(RouterState::Failed)
}

fn mapping_args(port: u16) -> Vec<(&'static str, String)> {
    vec![
        ("NewRemoteHost", String::new()),
        ("NewExternalPort", port.to_string()),
        ("NewProtocol", "TCP".into()),
    ]
}

/// Who holds this port on the router, and under what name. None when no one does, or when
/// the router won't say; then the router's own conflict check still guards the rule.
async fn holder<T: Transport>(
    transport: &T,
    service: &Service,
    port: u16,
) -> std::result::Result<Option<(Ipv4Addr, String)>, RouterState> {
    let wanted = ["NewInternalClient", "NewPortMappingDescription"];
    match ask(transport, service, "GetSpecificPortMappingEntry", &mapping_args(port), &wanted).await? {
        Answer::Fields(fields) => {
            // A rule that names no IPv4 address can't be this computer's.
            let client = field(&fields, "NewInternalClient")
                .and_then(|text| text.trim().parse().ok())
                .ok_or(RouterState::Taken { port, other: None })?;
            let name = field(&fields, "NewPortMappingDescription").unwrap_or_default();
            Ok(Some((client, name.to_owned())))
        }
        Answer::Fault(Some(NOT_AUTHORIZED)) => Err(RouterState::Refused),
        Answer::Fault(Some(NO_SUCH_ENTRY)) => Ok(None),
        // A router that won't say. Its own conflict check still guards the rule.
        Answer::Fault(_) => Ok(None),
    }
}

/// Adds or renews the rule, after checking that a rule here can help and that the port is
/// free or already this computer's.
async fn open<T: Transport>(
    transport: &T,
    service: &Service,
    gateway: Gateway,
    port: u16,
    name: &str,
) -> std::result::Result<(), RouterState> {
    let wanted = ["NewExternalIPAddress"];
    match ask(transport, service, "GetExternalIPAddress", &[], &wanted).await? {
        Answer::Fields(fields) => {
            let external: Ipv4Addr = field(&fields, "NewExternalIPAddress")
                .and_then(|text| text.trim().parse().ok())
                .filter(|address: &Ipv4Addr| !address.is_unspecified())
                .ok_or(RouterState::Failed)?;
            // Behind a provider's shared address or a second router, a rule can't help.
            if is_local_address(external.into()) {
                return Err(RouterState::Shared { external });
            }
        }
        Answer::Fault(Some(NOT_AUTHORIZED)) => return Err(RouterState::Refused),
        Answer::Fault(_) => return Err(RouterState::Failed),
    }
    if let Some((client, _)) = holder(transport, service, port).await?
        && client != gateway.me
    {
        return Err(RouterState::Taken {
            port,
            other: Some(client),
        });
    }
    let mut args = mapping_args(port);
    args.extend([
        ("NewInternalPort", port.to_string()),
        ("NewInternalClient", gateway.me.to_string()),
        ("NewEnabled", "1".into()),
        ("NewPortMappingDescription", name.to_owned()),
        ("NewLeaseDuration", LEASE.as_secs().to_string()),
    ]);
    match ask(transport, service, "AddPortMapping", &args, &[]).await? {
        Answer::Fields(_) => Ok(()),
        // A rule kept forever would stay open after the app closes, so none is added.
        Answer::Fault(Some(ONLY_PERMANENT)) => Err(RouterState::PermanentOnly),
        Answer::Fault(Some(CONFLICT)) => Err(RouterState::Taken { port, other: None }),
        Answer::Fault(Some(NOT_AUTHORIZED)) => Err(RouterState::Refused),
        Answer::Fault(_) => Err(RouterState::Failed),
    }
}

/// Removes the rule, only while it is still this run's own.
async fn close<T: Transport>(
    transport: &T,
    service: &Service,
    gateway: Gateway,
    port: u16,
    name: &str,
) {
    if let Ok(Some((client, held))) = holder(transport, service, port).await
        && client == gateway.me
        && held == name
    {
        let _ = ask(transport, service, "DeletePortMapping", &mapping_args(port), &[]).await;
    }
}

async fn find<T: Transport>(
    transport: &T,
    gateway: Gateway,
) -> std::result::Result<Service, RouterState> {
    let location = transport
        .search(gateway)
        .await
        .ok_or(RouterState::NoRouter)?;
    let description = transport
        .get(&location)
        .await
        .map_err(|_| RouterState::NoRouter)?;
    find_service(&description, &location, gateway).ok_or(RouterState::NoRouter)
}

/// How the rule appears in the router's own list. The process number tells this run's rule
/// apart from one an earlier run left, so that one is never removed by mistake.
fn rule_name() -> String {
    format!("Download Engine torrents {}", std::process::id())
}

/// Keeps `port` forwarded to this computer until `cancel`, then removes the rule. What it
/// comes to is written to `state`.
pub async fn keep_open<T: Transport>(
    transport: T,
    gateway: Gateway,
    port: u16,
    state: Arc<Mutex<RouterState>>,
    cancel: CancellationToken,
) {
    let name = rule_name();
    let set = |reached: RouterState| *state.lock() = reached;
    // The app only offers 1024 and up. Below that, ports belong to system services.
    if port < 1024 {
        set(RouterState::Unavailable);
        return;
    }
    let mut retries = RETRIES.iter();
    let service = loop {
        set(RouterState::Looking);
        let attempt = async {
            let service = find(&transport, gateway).await?;
            open(&transport, &service, gateway, port, &name).await?;
            Ok::<_, RouterState>(service)
        };
        let outcome = tokio::select! {
            _ = cancel.cancelled() => return,
            outcome = attempt => outcome,
        };
        match outcome {
            Ok(service) => break service,
            Err(problem) => {
                set(problem);
                let wait = match retries.next() {
                    Some(wait) if problem.worth_retrying() => *wait,
                    _ => return,
                };
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        }
    };
    set(RouterState::Working {
        gateway: gateway.router,
        port,
    });
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(RENEW_EVERY) => {}
        }
        match open(&transport, &service, gateway, port, &name).await {
            Ok(()) => set(RouterState::Working {
                gateway: gateway.router,
                port,
            }),
            // The port went to someone else. It isn't ours to keep or to remove.
            Err(taken @ RouterState::Taken { .. }) => {
                set(taken);
                return;
            }
            // A router that stopped answering may come back. The rule expires by itself.
            Err(problem) => set(problem),
        }
    }
    close(&transport, &service, gateway, port, &name).await;
}

/// The real network: one UDP search, then plain HTTP to the gateway's own address with no
/// proxy, no redirects, no name lookups and short timeouts.
pub struct Network {
    http: reqwest::Client,
}

impl Network {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(crate::peer::CLIENT_NAME)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(ANSWER_TIMEOUT)
            .timeout(ANSWER_TIMEOUT)
            .build()?;
        Ok(Self { http })
    }
}

async fn limited_body(mut response: reqwest::Response) -> Result<Vec<u8>> {
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_ANSWER as u64),
        "the router's answer is too large"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            body.len() + chunk.len() <= MAX_ANSWER,
            "the router's answer is too large"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl Transport for Network {
    async fn search(&self, gateway: Gateway) -> Option<Url> {
        let socket = tokio::net::UdpSocket::bind(SocketAddr::new(gateway.me.into(), 0))
            .await
            .ok()?;
        for target in SEARCH_TARGETS {
            let request = search_request(target);
            // Straight to the gateway first, so that its answer comes back the way it went.
            let direct = SocketAddr::new(gateway.router.into(), SSDP.port());
            let _ = socket.send_to(request.as_bytes(), direct).await;
            let _ = socket.send_to(request.as_bytes(), SocketAddr::V4(SSDP)).await;
        }
        let deadline = tokio::time::Instant::now() + ANSWER_TIMEOUT;
        let mut buffer = vec![0; MAX_SEARCH_ANSWER + 1];
        let mut bad = 0;
        loop {
            match tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await {
                Err(_) => return None,
                Ok(Err(_)) => {
                    bad += 1;
                    if bad > MAX_BAD_PACKETS {
                        return None;
                    }
                }
                Ok(Ok((count, from))) => {
                    if count <= MAX_SEARCH_ANSWER
                        && let Some(url) = location_in_answer(&buffer[..count], from, gateway)
                    {
                        return Some(url);
                    }
                }
            }
        }
    }

    async fn get(&self, url: &Url) -> Result<Vec<u8>> {
        let response = self.http.get(url.clone()).send().await?;
        if response.status() != reqwest::StatusCode::OK {
            bail!("the router answered {}", response.status());
        }
        limited_body(response).await
    }

    async fn soap(&self, url: &Url, action: &str, body: String) -> Result<(u16, Vec<u8>)> {
        let response = self
            .http
            .post(url.clone())
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("SOAPAction", action)
            .body(body)
            .send()
            .await?;
        let status = response.status().as_u16();
        Ok((status, limited_body(response).await?))
    }
}

/// The default gateway and this computer's own address toward it, from the routing table.
/// None when there is no home-network gateway, or on a system this doesn't read yet.
pub fn default_gateway() -> Option<Gateway> {
    platform::default_gateway()
}

#[cfg(windows)]
mod platform {
    use super::Gateway;
    use std::net::Ipv4Addr;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetBestRoute2, MIB_IPFORWARD_ROW2};
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, IN_ADDR, IN_ADDR_0, SOCKADDR_IN, SOCKADDR_INET,
    };

    pub fn default_gateway() -> Option<Gateway> {
        // Any public address works here. Windows only looks up its route and sends nothing.
        let destination = SOCKADDR_INET {
            Ipv4: SOCKADDR_IN {
                sin_family: AF_INET,
                sin_port: 0,
                sin_addr: IN_ADDR {
                    S_un: IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes([1, 1, 1, 1]),
                    },
                },
                sin_zero: [0; 8],
            },
        };
        // SAFETY: both are plain data that Windows fills in, for which all zeros is valid.
        let (mut route, mut source): (MIB_IPFORWARD_ROW2, SOCKADDR_INET) =
            unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
        // SAFETY: every pointer is to a live local of the right type; null means "any".
        let status = unsafe {
            GetBestRoute2(
                std::ptr::null(),
                0,
                std::ptr::null(),
                &destination,
                0,
                &mut route,
                &mut source,
            )
        };
        if status != 0 {
            return None;
        }
        // SAFETY: each family field says which part of the union Windows filled in.
        let (router, me) = unsafe {
            if route.NextHop.si_family != AF_INET || source.si_family != AF_INET {
                return None;
            }
            (
                Ipv4Addr::from(route.NextHop.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()),
                Ipv4Addr::from(source.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()),
            )
        };
        Gateway::checked(router, me)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Gateway, default_route};
    use std::net::{IpAddr, Ipv4Addr, UdpSocket};

    pub fn default_gateway() -> Option<Gateway> {
        let table = std::fs::read_to_string("/proc/net/route").ok()?;
        let router = default_route(&table)?;
        // Connecting a UDP socket only picks the route and the local address. It sends nothing.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.connect((router, 1900)).ok()?;
        let IpAddr::V4(me) = socket.local_addr().ok()?.ip() else {
            return None;
        };
        Gateway::checked(router, me)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    /// Not read on this system yet, so router setup reports itself unavailable.
    pub fn default_gateway() -> Option<super::Gateway> {
        None
    }
}

/// The gateway of the best default route in Linux's `/proc/net/route`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn default_route(table: &str) -> Option<Ipv4Addr> {
    const UP: u32 = 0x1;
    const GATEWAY: u32 = 0x2;
    let hex = |text: &str| u32::from_str_radix(text, 16).ok();
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 8 {
                return None;
            }
            let (destination, gateway, flags) = (hex(fields[1])?, hex(fields[2])?, hex(fields[3])?);
            let (metric, mask) = (fields[6].parse::<u32>().ok()?, hex(fields[7])?);
            (destination == 0 && mask == 0 && flags & (UP | GATEWAY) == UP | GATEWAY && gateway != 0)
                .then(|| (metric, Ipv4Addr::from(gateway.to_ne_bytes())))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, gateway)| gateway)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTER: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);
    const ME: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 20);
    const OTHER: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 23);
    const IGD: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";

    fn gateway() -> Gateway {
        Gateway::checked(ROUTER, ME).unwrap()
    }

    fn location() -> Url {
        Url::parse("http://192.168.1.1:5000/rootDesc.xml").unwrap()
    }

    #[test]
    fn only_home_network_addresses_make_a_gateway() {
        assert!(Gateway::checked(ROUTER, ME).is_some());
        assert!(Gateway::checked(Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 9)).is_some());
        for (router, me) in [
            (Ipv4Addr::new(100, 64, 0, 1), ME),
            (Ipv4Addr::new(8, 8, 8, 8), ME),
            (Ipv4Addr::LOCALHOST, ME),
            (Ipv4Addr::new(169, 254, 1, 1), ME),
            (ROUTER, Ipv4Addr::new(100, 64, 0, 9)),
            (ROUTER, ROUTER),
            (Ipv4Addr::UNSPECIFIED, ME),
        ] {
            assert!(Gateway::checked(router, me).is_none(), "{router} {me}");
        }
    }

    #[test]
    fn router_addresses_must_be_plain_http_to_the_gateway_itself() {
        let gateway = gateway();
        assert!(checked_url("http://192.168.1.1:5000/rootDesc.xml", None, gateway).is_some());
        assert!(checked_url("http://192.168.1.1/ctl", None, gateway).is_some());
        assert_eq!(
            checked_url("/ctl/IPConn#here", Some(&location()), gateway).map(String::from),
            Some("http://192.168.1.1:5000/ctl/IPConn".into())
        );
        for refused in [
            "https://192.168.1.1/rootDesc.xml",
            "http://192.168.1.23/rootDesc.xml",
            "http://router.local/rootDesc.xml",
            "http://192.168.1.1.example.org/rootDesc.xml",
            "http://admin:secret@192.168.1.1/rootDesc.xml",
            "http://[fe80::1]/rootDesc.xml",
            "http://127.0.0.1/rootDesc.xml",
            "http://203.0.113.7/rootDesc.xml",
            "file:///C:/Windows/win.ini",
            "http://192.168.1.1:0/x",
            "http://192.168.1.1/\u{7}",
            "",
        ] {
            assert!(checked_url(refused, None, gateway).is_none(), "{refused}");
        }
        // A relative address can't climb out to another host either.
        for refused in ["//203.0.113.7/ctl", "http://192.168.1.23/ctl"] {
            assert!(
                checked_url(refused, Some(&location()), gateway).is_none(),
                "{refused}"
            );
        }
        let long = format!("http://192.168.1.1/{}", "a".repeat(MAX_URL));
        assert!(checked_url(&long, None, gateway).is_none());
    }

    fn search_answer(location: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: {location}\r\n\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn a_search_answer_counts_only_from_the_gateway_about_a_gateway_device() {
        let gateway = gateway();
        let from_router = SocketAddr::new(ROUTER.into(), 1900);
        let good = search_answer("http://192.168.1.1:5000/rootDesc.xml");
        assert_eq!(
            location_in_answer(&good, from_router, gateway),
            Some(location())
        );
        // From another device on the network, even naming the gateway's address.
        let from_other = SocketAddr::new(OTHER.into(), 1900);
        assert_eq!(location_in_answer(&good, from_other, gateway), None);
        // From the gateway, but pointing somewhere else.
        let elsewhere = search_answer("http://192.168.1.23:5000/rootDesc.xml");
        assert_eq!(location_in_answer(&elsewhere, from_router, gateway), None);
        // Not a gateway device, not a success, or too long.
        let printer = String::from_utf8(good.clone())
            .unwrap()
            .replace("device:InternetGatewayDevice:1", "device:Printer:1");
        assert_eq!(location_in_answer(printer.as_bytes(), from_router, gateway), None);
        let failed = String::from_utf8(good.clone())
            .unwrap()
            .replace("200 OK", "404 Not Found");
        assert_eq!(location_in_answer(failed.as_bytes(), from_router, gateway), None);
        let mut huge = good.clone();
        huge.extend(std::iter::repeat_n(b' ', MAX_SEARCH_ANSWER));
        assert_eq!(location_in_answer(&huge, from_router, gateway), None);
        assert_eq!(location_in_answer(b"\xff\xfe", from_router, gateway), None);
    }

    fn description(services: &str, base: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?><root xmlns="urn:schemas-upnp-org:device-1-0">{base}<device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType><deviceList><device><serviceList>{services}</serviceList></device></deviceList></device></root>"#
        )
        .into_bytes()
    }

    fn service(kind: &str, control: &str) -> String {
        format!("<service><serviceType>{kind}</serviceType><controlURL>{control}</controlURL></service>")
    }

    #[test]
    fn a_description_gives_the_best_service_on_the_gateway() {
        let gateway = gateway();
        let services = [
            service("urn:schemas-upnp-org:service:Layer3Forwarding:1", "/ctl/L3F"),
            service("urn:schemas-upnp-org:service:WANPPPConnection:1", "/ctl/PPP"),
            service(IGD, "/ctl/IPConn"),
        ]
        .concat();
        let found = find_service(&description(&services, ""), &location(), gateway).unwrap();
        assert_eq!(found.kind, IGD);
        assert_eq!(found.control.as_str(), "http://192.168.1.1:5000/ctl/IPConn");
        // A better service that leads elsewhere is skipped, never followed.
        let services = [
            service(IGD, "/ctl/IPConn"),
            service("urn:schemas-upnp-org:service:WANIPConnection:2", "http://203.0.113.7/ctl"),
        ]
        .concat();
        let found = find_service(&description(&services, ""), &location(), gateway).unwrap();
        assert_eq!(found.control.as_str(), "http://192.168.1.1:5000/ctl/IPConn");
        // Entities in the address are read, not executed.
        let amp = service(IGD, "/ctl?a=1&amp;b=&#50;");
        let found = find_service(&description(&amp, ""), &location(), gateway).unwrap();
        assert_eq!(found.control.as_str(), "http://192.168.1.1:5000/ctl?a=1&b=2");
        // A base address counts only on the gateway, and a bad one refuses everything.
        let based = description(&service(IGD, "ctl"), "<URLBase>http://192.168.1.1:49152/</URLBase>");
        let found = find_service(&based, &location(), gateway).unwrap();
        assert_eq!(found.control.as_str(), "http://192.168.1.1:49152/ctl");
        let moved = description(&service(IGD, "ctl"), "<URLBase>http://203.0.113.7/</URLBase>");
        assert_eq!(find_service(&moved, &location(), gateway), None);
        // No port-forwarding service at all.
        let other = service("urn:schemas-upnp-org:service:Layer3Forwarding:1", "/ctl/L3F");
        assert_eq!(find_service(&description(&other, ""), &location(), gateway), None);
    }

    #[test]
    fn booby_trapped_or_broken_xml_is_refused() {
        let gateway = gateway();
        let laughs = br#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;">]><root><URLBase>&lol2;</URLBase></root>"#;
        assert!(leaves(laughs).is_none(), "a DOCTYPE is refused");
        let unknown = description(&service(IGD, "/ctl/&secret;"), "");
        assert_eq!(find_service(&unknown, &location(), gateway), None);
        let deep = format!("{}{}", "<a>".repeat(MAX_DEPTH + 1), "</a>".repeat(MAX_DEPTH + 1));
        assert!(leaves(deep.as_bytes()).is_none());
        let long = format!("<root><x>{}</x></root>", "a".repeat(MAX_FIELD + 1));
        assert!(leaves(long.as_bytes()).is_none());
        let many = format!("<root>{}</root>", "<s><t>1</t></s>".repeat(MAX_LEAVES + 1));
        assert!(leaves(many.as_bytes()).is_none());
        let services = format!(
            "<root>{}</root>",
            service(IGD, "/c").repeat(MAX_SERVICES + 1)
        );
        assert!(leaves(services.as_bytes()).is_none());
        assert!(leaves(b"<root><a></b></root>").is_none());
        assert!(leaves(b"<root><a>").is_none());
        assert!(leaves(&vec![b' '; MAX_ANSWER + 1]).is_none());
    }

    #[test]
    fn soap_answers_and_faults_are_read() {
        let ok = br#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:GetExternalIPAddressResponse xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"><NewExternalIPAddress>203.0.113.9</NewExternalIPAddress><Other>x</Other></u:GetExternalIPAddressResponse></s:Body></s:Envelope>"#;
        assert_eq!(
            read_answer(200, ok, &["NewExternalIPAddress"]),
            Some(Answer::Fields(vec![(
                "NewExternalIPAddress".into(),
                "203.0.113.9".into()
            )]))
        );
        let fault = br#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>725</errorCode><errorDescription>OnlyPermanentLeasesSupported</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#;
        assert_eq!(read_answer(500, fault, &[]), Some(Answer::Fault(Some(725))));
        assert_eq!(read_answer(500, b"", &[]), Some(Answer::Fault(None)));
        assert_eq!(read_answer(200, b"", &[]), Some(Answer::Fields(Vec::new())));
        assert_eq!(read_answer(302, ok, &[]), None, "no redirects or odd answers");
    }

    #[test]
    fn requests_carry_only_numbers_and_escaped_text() {
        let body = envelope(
            IGD,
            "AddPortMapping",
            &[
                ("NewExternalPort", "6881".into()),
                ("NewPortMappingDescription", "a<b>&\"c'".into()),
            ],
        );
        assert!(body.contains("<NewExternalPort>6881</NewExternalPort>"));
        assert!(body.contains("a&lt;b&gt;&amp;&quot;c&apos;"));
        assert!(leaves(body.as_bytes()).is_some(), "the request itself is valid XML");
        assert!(rule_name().starts_with("Download Engine torrents "));
    }

    #[test]
    fn the_default_route_comes_from_the_route_table() {
        // 192.168.1.1 and 10.0.0.1, as Linux prints them on a little-endian computer.
        let one = Ipv4Addr::from(u32::from_str_radix("0101A8C0", 16).unwrap().to_ne_bytes());
        let table = format!(
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
             eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
             eth0\t00000000\t{:08X}\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
             wlan0\t00000000\t0100000A\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
             tun0\t00000000\t0200000A\t0001\t0\t0\t50\t00000000\t0\t0\t0\n",
            u32::from_ne_bytes(one.octets())
        );
        assert_eq!(default_route(&table), Some(one));
        assert_eq!(default_route("Iface\tDestination\n"), None);
        assert_eq!(default_route("garbage"), None);
    }

    /// A scripted router that keeps one rule, and remembers what it was asked.
    struct FakeRouter {
        search: Option<Url>,
        external: &'static str,
        add_fault: Option<u32>,
        rule: Mutex<Option<(Ipv4Addr, String)>>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeRouter {
        fn new() -> Self {
            Self {
                search: Some(location()),
                external: "203.0.113.9",
                add_fault: None,
                rule: Mutex::new(None),
                calls: Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().clone()
        }
    }

    fn between<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
        let start = text.find(&format!("<{tag}>"))? + tag.len() + 2;
        let end = text[start..].find(&format!("</{tag}>"))?;
        Some(&text[start..start + end])
    }

    fn ok(fields: &str) -> (u16, Vec<u8>) {
        (200, format!("<s:Envelope><s:Body><u:R>{fields}</u:R></s:Body></s:Envelope>").into_bytes())
    }

    fn fault(code: u32) -> (u16, Vec<u8>) {
        (500, format!("<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>{code}</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>").into_bytes())
    }

    impl Transport for FakeRouter {
        async fn search(&self, _: Gateway) -> Option<Url> {
            self.calls.lock().push("search".into());
            self.search.clone()
        }

        async fn get(&self, url: &Url) -> Result<Vec<u8>> {
            assert_eq!(url, &location());
            Ok(description(&service(IGD, "/ctl/IPConn"), ""))
        }

        async fn soap(&self, url: &Url, action: &str, body: String) -> Result<(u16, Vec<u8>)> {
            assert_eq!(url.as_str(), "http://192.168.1.1:5000/ctl/IPConn");
            let name = action.trim_matches('"').rsplit('#').next().unwrap().to_owned();
            self.calls.lock().push(name.clone());
            let mut rule = self.rule.lock();
            Ok(match name.as_str() {
                "GetExternalIPAddress" => ok(&format!(
                    "<NewExternalIPAddress>{}</NewExternalIPAddress>",
                    self.external
                )),
                "GetSpecificPortMappingEntry" => match &*rule {
                    Some((client, held)) => ok(&format!(
                        "<NewInternalClient>{client}</NewInternalClient><NewPortMappingDescription>{held}</NewPortMappingDescription>"
                    )),
                    None => fault(NO_SUCH_ENTRY),
                },
                "AddPortMapping" => match self.add_fault {
                    Some(code) => fault(code),
                    None => {
                        assert_eq!(between(&body, "NewLeaseDuration"), Some("3600"));
                        assert_eq!(between(&body, "NewProtocol"), Some("TCP"));
                        assert_eq!(between(&body, "NewExternalPort"), Some("6881"));
                        let client = between(&body, "NewInternalClient").unwrap().parse().unwrap();
                        let held = between(&body, "NewPortMappingDescription").unwrap();
                        *rule = Some((client, held.to_owned()));
                        ok("")
                    }
                },
                "DeletePortMapping" => {
                    *rule = None;
                    ok("")
                }
                other => panic!("unexpected action {other}"),
            })
        }
    }

    impl<T: Transport> Transport for Arc<T> {
        fn search(&self, gateway: Gateway) -> impl Future<Output = Option<Url>> + Send {
            self.as_ref().search(gateway)
        }
        fn get(&self, url: &Url) -> impl Future<Output = Result<Vec<u8>>> + Send {
            self.as_ref().get(url)
        }
        fn soap(
            &self,
            url: &Url,
            action: &str,
            body: String,
        ) -> impl Future<Output = Result<(u16, Vec<u8>)>> + Send {
            self.as_ref().soap(url, action, body)
        }
    }

    async fn settle(state: &Mutex<RouterState>, wanted: impl Fn(RouterState) -> bool) {
        for _ in 0..1000 {
            if wanted(*state.lock()) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("router setup stayed at {:?}", *state.lock());
    }

    fn run(
        router: &Arc<FakeRouter>,
    ) -> (Arc<Mutex<RouterState>>, CancellationToken, tokio::task::JoinHandle<()>) {
        let state = Arc::new(Mutex::new(RouterState::Looking));
        let cancel = CancellationToken::new();
        let task = tokio::spawn(keep_open(
            router.clone(),
            gateway(),
            6881,
            state.clone(),
            cancel.clone(),
        ));
        (state, cancel, task)
    }

    #[tokio::test(start_paused = true)]
    async fn a_rule_is_added_renewed_and_removed_in_order() {
        let router = Arc::new(FakeRouter::new());
        let (state, cancel, task) = run(&router);
        settle(&state, |s| matches!(s, RouterState::Working { .. })).await;
        assert_eq!(
            *state.lock(),
            RouterState::Working {
                gateway: ROUTER,
                port: 6881
            }
        );
        assert_eq!(router.rule.lock().clone(), Some((ME, rule_name())));
        tokio::time::sleep(RENEW_EVERY + Duration::from_secs(1)).await;
        cancel.cancel();
        task.await.unwrap();
        assert_eq!(
            router.calls(),
            [
                "search",
                "GetExternalIPAddress",
                "GetSpecificPortMappingEntry",
                "AddPortMapping",
                "GetExternalIPAddress",
                "GetSpecificPortMappingEntry",
                "AddPortMapping",
                "GetSpecificPortMappingEntry",
                "DeletePortMapping",
            ]
        );
        assert_eq!(*router.rule.lock(), None, "the rule is gone");
    }

    #[tokio::test(start_paused = true)]
    async fn a_port_held_by_another_device_is_never_replaced_or_removed() {
        let router = Arc::new(FakeRouter::new());
        *router.rule.lock() = Some((OTHER, "Game console".into()));
        let (state, cancel, task) = run(&router);
        settle(&state, |s| matches!(s, RouterState::Taken { .. })).await;
        task.await.unwrap();
        cancel.cancel();
        assert_eq!(
            *state.lock(),
            RouterState::Taken {
                port: 6881,
                other: Some(OTHER)
            }
        );
        assert!(!router.calls().iter().any(|call| call == "AddPortMapping" || call == "DeletePortMapping"));
        assert_eq!(router.rule.lock().clone(), Some((OTHER, "Game console".into())));
    }

    #[tokio::test(start_paused = true)]
    async fn a_router_that_only_keeps_rules_forever_gets_none() {
        let router = Arc::new(FakeRouter {
            add_fault: Some(ONLY_PERMANENT),
            ..FakeRouter::new()
        });
        let (state, _cancel, task) = run(&router);
        task.await.unwrap();
        assert_eq!(*state.lock(), RouterState::PermanentOnly);
        assert_eq!(*router.rule.lock(), None);
        assert_eq!(
            router.calls().iter().filter(|call| *call == "AddPortMapping").count(),
            1,
            "never asked again with a lease that lasts forever"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_shared_outside_address_adds_nothing() {
        let router = Arc::new(FakeRouter {
            external: "100.64.12.7",
            ..FakeRouter::new()
        });
        let (state, _cancel, task) = run(&router);
        task.await.unwrap();
        assert_eq!(
            *state.lock(),
            RouterState::Shared {
                external: Ipv4Addr::new(100, 64, 12, 7)
            }
        );
        assert_eq!(router.calls(), ["search", "GetExternalIPAddress"]);
    }

    #[tokio::test(start_paused = true)]
    async fn no_router_is_asked_four_times_then_left_alone() {
        let router = Arc::new(FakeRouter {
            search: None,
            ..FakeRouter::new()
        });
        let (state, _cancel, task) = run(&router);
        task.await.unwrap();
        assert_eq!(*state.lock(), RouterState::NoRouter);
        assert_eq!(router.calls(), ["search"; 4]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_rule_from_another_run_is_renewed_as_ours_only_by_name_and_never_removed() {
        let router = Arc::new(FakeRouter::new());
        let (state, cancel, task) = run(&router);
        settle(&state, |s| matches!(s, RouterState::Working { .. })).await;
        // Another run on this computer took the rule over in the meantime.
        *router.rule.lock() = Some((ME, "Download Engine torrents 1".into()));
        cancel.cancel();
        task.await.unwrap();
        assert_eq!(router.calls().last().map(String::as_str), Some("GetSpecificPortMappingEntry"));
        assert_eq!(
            router.rule.lock().clone(),
            Some((ME, "Download Engine torrents 1".into())),
            "only this run's own rule is removed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_system_port_is_never_forwarded() {
        let router = Arc::new(FakeRouter::new());
        let state = Arc::new(Mutex::new(RouterState::Looking));
        keep_open(router.clone(), gateway(), 445, state.clone(), CancellationToken::new()).await;
        assert_eq!(*state.lock(), RouterState::Unavailable);
        assert!(router.calls().is_empty(), "the router was never asked");
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_while_looking_leaves_the_router_alone() {
        let router = Arc::new(FakeRouter {
            search: None,
            ..FakeRouter::new()
        });
        let (state, cancel, task) = run(&router);
        settle(&state, |s| s == RouterState::NoRouter).await;
        cancel.cancel();
        task.await.unwrap();
        assert_eq!(router.calls(), ["search"]);
    }

    #[test]
    fn the_state_reaches_the_app_as_plain_fields() {
        let text = serde_json::to_value(RouterState::Working {
            gateway: ROUTER,
            port: 6881,
        })
        .unwrap();
        assert_eq!(
            text,
            serde_json::json!({"state": "working", "gateway": "192.168.1.1", "port": 6881})
        );
        assert_eq!(
            serde_json::to_value(RouterState::NoRouter).unwrap(),
            serde_json::json!({"state": "no_router"})
        );
    }
}
