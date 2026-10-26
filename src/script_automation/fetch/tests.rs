use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
};

/// A loopback server that records each raw request and answers by path.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}
impl Server {
    fn start() -> Self {
        Self::redirecting_to(None)
    }
    fn redirecting_to(destination: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut data = Vec::new();
                let mut buffer = [0; 4096];
                while !data.windows(4).any(|w| w == b"\r\n\r\n") {
                    let Ok(n) = stream.read(&mut buffer) else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..n]);
                }
                let text = String::from_utf8_lossy(&data).into_owned();
                let head_end = text.find("\r\n\r\n").map_or(text.len(), |i| i + 4);
                let length = text
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while data.len() < head_end + length {
                    let Ok(n) = stream.read(&mut buffer) else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8_lossy(&data).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                log.lock().unwrap().push(request);
                let reply: String = match path.as_str() {
                    "/cross307" | "/cross308" => format!("HTTP/1.1 {} Redirect\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", &path[6..], destination.as_deref().unwrap()),
                    "/same307" => "HTTP/1.1 307 Redirect\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    "/cross-origin" => format!("HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", destination.as_deref().unwrap()),
                    "/page" => "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Test: one\r\nX-Test: two\r\nSet-Cookie: session=abc123; Path=/; HttpOnly\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".into(),
                    "/moved" => "HTTP/1.1 302 Found\r\nLocation: /page\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    "/form" => "HTTP/1.1 303 See Other\r\nLocation: /page\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    "/loop" => "HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    "/huge" => "HTTP/1.1 200 OK\r\nContent-Length: 9437184\r\nConnection: close\r\n\r\n".into(),
                    "/missing" => "HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\ngone".into(),
                    _ => "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into(),
                };
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Self { base, requests }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
    fn last(&self) -> String {
        self.requests
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }
}

#[test]
fn reader_security_redirects_do_not_forward_caller_secrets() {
    let target = Server::start();
    let origin = Server::redirecting_to(Some(target.url("/other")));
    let mut fetch = fetcher().allowing_local();
    let headers = vec![
        ("Authorization".into(), "Bearer fixture-secret".into()),
        ("X-API-Key".into(), "fixture-api-key".into()),
        ("X-Custom-Login".into(), "fixture-custom-secret".into()),
    ];
    fetch
        .fetch(FetchRequest {
            headers: headers.clone(),
            ..get(&origin.url("/moved"))
        })
        .unwrap();
    assert!(
        origin
            .last()
            .contains("authorization: bearer fixture-secret")
    );
    fetch
        .fetch(FetchRequest {
            headers,
            ..get(&origin.url("/cross-origin"))
        })
        .unwrap();
    let received = target.last();
    assert!(received.starts_with("get /other"));
    assert!(
        !received.contains("fixture-"),
        "Cross-origin redirect leaked caller headers"
    );
    assert!(
        !received.contains("cookie:"),
        "An origin's cookies must stay there"
    );
}

#[test]
fn reader_security_post_bodies_are_not_forwarded_to_another_origin() {
    let target = Server::start();
    let origin = Server::redirecting_to(Some(target.url("/other")));
    let mut fetch = fetcher().allowing_local();
    let post = |path| FetchRequest {
        method: Some("POST".into()),
        body: Some("fixture-body-secret".into()),
        headers: vec![("Authorization".into(), "Bearer fixture-secret".into())],
        ..get(&origin.url(path))
    };
    fetch.fetch(post("/same307")).unwrap();
    let same = origin.last();
    assert!(same.starts_with("post /other") && same.ends_with("fixture-body-secret"));
    assert!(same.contains("authorization: bearer fixture-secret"));
    for path in ["/cross307", "/cross308"] {
        let error = fetch.fetch(post(path)).unwrap_err().to_string();
        assert!(error.contains("POST body"), "{error}");
    }
    assert!(target.requests.lock().unwrap().is_empty());
    fetch.fetch(post("/cross-origin")).unwrap();
    assert!(target.last().starts_with("get /other"));
    assert!(!target.last().contains("fixture-"));
}

#[test]
fn reader_security_rejects_proxy_routes_before_connecting() {
    let proxy = Server::start();
    for scheme in ["system", "http", "https", "socks5", "socks5h"] {
        let route = crate::proxy::Route {
            settings: crate::proxy::Settings {
                mode: if scheme == "system" {
                    crate::proxy::Mode::System
                } else {
                    crate::proxy::Mode::Custom
                },
                endpoint: if scheme == "system" {
                    String::new()
                } else {
                    proxy.base.replacen("http", scheme, 1)
                },
                ..Default::default()
            },
            login: None,
        };
        let mut fetch = Fetcher::new(route).unwrap();
        let error = fetch
            .fetch(get("http://8.8.8.8/fixture"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Direct connection"), "{scheme}: {error}");
    }
    assert!(proxy.requests.lock().unwrap().is_empty());
}

fn fetcher() -> Fetcher {
    Fetcher::new(crate::proxy::Route::direct()).unwrap()
}
fn get(url: &str) -> FetchRequest {
    FetchRequest {
        url: url.into(),
        ..Default::default()
    }
}

#[test]
fn readers_cannot_reach_local_or_home_network_addresses() {
    let server = Server::start();
    let mut fetch = fetcher();
    for url in [
        server.url("/page"),
        "http://localhost/".into(),
        "http://192.168.1.1/admin".into(),
        "http://10.0.0.2/".into(),
        "http://169.254.169.254/latest/meta-data".into(),
        "http://[::1]/".into(),
    ] {
        let error = fetch.fetch(get(&url)).unwrap_err().to_string();
        assert!(error.contains("public internet"), "{url}: {error}");
    }
    assert!(server.requests.lock().unwrap().is_empty());
    for url in [
        "file:///C:/Windows/win.ini",
        "ftp://example.com/x",
        "https://user:pass@example.com/",
    ] {
        assert!(fetch.fetch(get(url)).is_err(), "{url}");
    }
}

#[test]
fn replies_carry_status_headers_and_body_without_browser_data() {
    let server = Server::start();
    let mut fetch = fetcher().allowing_local();
    let reply = fetch
        .fetch(FetchRequest {
            url: server.url("/page"),
            headers: vec![
                ("X-Requested-With".into(), "XMLHttpRequest".into()),
                ("User-Agent".into(), "Reader/1".into()),
            ],
            ..Default::default()
        })
        .unwrap();
    assert_eq!((reply.status, reply.body.as_str()), (200, "hello"));
    assert_eq!(reply.headers["x-test"], "one, two");
    assert!(!reply.headers.contains_key("set-cookie"));
    let sent = server.last();
    assert!(
        sent.contains("x-requested-with: xmlhttprequest") && sent.contains("user-agent: reader/1")
    );
    assert!(sent.contains("accept-encoding: identity") && !sent.contains("cookie:"));
    // The site's own cookie goes back to the same host only, for this run only.
    fetch.fetch(get(&server.url("/other"))).unwrap();
    assert!(server.last().contains("cookie: session=abc123"));
    let mut fresh = fetcher().allowing_local();
    fresh.fetch(get(&server.url("/other"))).unwrap();
    assert!(!server.last().contains("cookie:"));
    // Error statuses are returned for the script to decide on.
    assert_eq!(
        fetch.fetch(get(&server.url("/missing"))).unwrap().status,
        404
    );
}

#[test]
fn methods_headers_redirects_and_limits_are_enforced() {
    let server = Server::start();
    let mut fetch = fetcher().allowing_local();
    let moved = fetch.fetch(get(&server.url("/moved"))).unwrap();
    assert_eq!(moved.url, server.url("/page"));
    let posted = fetch
        .fetch(FetchRequest {
            url: server.url("/form"),
            method: Some("post".into()),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(r#"{"q":1}"#.into()),
        })
        .unwrap();
    assert_eq!(posted.body, "hello");
    let requests = server.requests.lock().unwrap().clone();
    assert!(
        requests[requests.len() - 2].starts_with("POST /form")
            && requests[requests.len() - 2].ends_with(r#"{"q":1}"#)
    );
    assert!(requests[requests.len() - 1].starts_with("GET /page"));
    for (request, expected) in [
        (
            FetchRequest {
                method: Some("PUT".into()),
                ..get(&server.url("/"))
            },
            "GET and POST",
        ),
        (
            FetchRequest {
                body: Some("x".into()),
                ..get(&server.url("/"))
            },
            "Only a POST",
        ),
        (
            FetchRequest {
                headers: vec![("Cookie".into(), "a=b".into())],
                ..get(&server.url("/"))
            },
            "cookie",
        ),
        (
            FetchRequest {
                headers: vec![("Host".into(), "evil.test".into())],
                ..get(&server.url("/"))
            },
            "host",
        ),
        (
            FetchRequest {
                headers: vec![("Accept-Encoding".into(), "gzip".into())],
                ..get(&server.url("/"))
            },
            "accept-encoding",
        ),
        (
            FetchRequest {
                headers: vec![("Proxy-Anything".into(), "x".into())],
                ..get(&server.url("/"))
            },
            "proxy-anything",
        ),
        (
            FetchRequest {
                headers: vec![("X-Bad".into(), "a\r\nb".into())],
                ..get(&server.url("/"))
            },
            "Invalid value",
        ),
        (get(&server.url("/loop")), "too many redirects"),
        (get(&server.url("/huge")), "8 MiB"),
    ] {
        let error = fetch.fetch(request).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
    let mut counted = fetcher().allowing_local();
    for _ in 0..MAX_REQUESTS {
        counted.fetch(get(&server.url("/"))).unwrap();
    }
    assert!(
        counted
            .fetch(get(&server.url("/")))
            .unwrap_err()
            .to_string()
            .contains("at most 50 requests")
    );
}

/// Live check of a real public site: cargo test --lib fetch::tests::live -- --ignored
#[test]
#[ignore = "contacts example.com"]
fn live_public_page_is_fetched() {
    let reply = fetcher().fetch(get("https://example.com/")).unwrap();
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("Example Domain"));
}
