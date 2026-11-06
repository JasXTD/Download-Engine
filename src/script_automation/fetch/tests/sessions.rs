use super::*;
use crate::script_automation::{HttpSession, execute_reader_job};

fn session(origin: &str, value: &str) -> HttpSession {
    HttpSession::new(
        origin,
        BTreeMap::from([("login".into(), value.into())]),
        BTreeMap::from([("Authorization".into(), "Bearer harmless-fixture".into())]),
    )
    .unwrap()
}
#[test]
fn explicit_sessions_isolate_cookies_and_headers_and_share_the_request_budget() {
    let server = Server::start();
    let a = session(&server.base, "first");
    let b = session(&server.base, "second");
    let mut client = Fetcher::new(crate::proxy::Route::direct())
        .unwrap()
        .allowing_local();
    let request = |handle: Option<HttpSession>, path: &str| FetchRequest {
        url: server.url(path),
        session: handle,
        ..Default::default()
    };
    client.fetch(request(Some(a.clone()), "/page")).unwrap();
    assert!(server.last().contains("cookie: login=first"));
    assert!(
        server
            .last()
            .contains("authorization: bearer harmless-fixture")
    );
    client.fetch(request(Some(a), "/other")).unwrap();
    assert!(server.last().contains("session=abc123"));
    client.fetch(request(Some(b.clone()), "/other")).unwrap();
    assert!(server.last().contains("cookie: login=second"));
    assert!(!server.last().contains("abc123"));
    client.fetch(request(None, "/other")).unwrap();
    assert!(!server.last().contains("cookie:") && !server.last().contains("authorization:"));
    for _ in 4..MAX_REQUESTS {
        client.fetch(request(Some(b.clone()), "/other")).unwrap();
    }
    assert!(client.fetch(request(Some(b), "/other")).is_err());
    assert_eq!(server.requests.lock().unwrap().len(), MAX_REQUESTS);
}
#[test]
fn authenticated_redirects_and_origin_changes_never_send_credentials() {
    let victim = Server::start();
    let server = Server::redirecting_to(Some(victim.url("/other")));
    let mut client = Fetcher::new(crate::proxy::Route::direct())
        .unwrap()
        .allowing_local();
    for path in ["/cross-origin", "/cross307", "/cross308"] {
        let result = client.fetch(FetchRequest {
            url: server.url(path),
            session: Some(session(&server.base, "private-fixture")),
            ..Default::default()
        });
        assert!(result.is_err());
    }
    assert!(victim.requests.lock().unwrap().is_empty());
    assert!(
        client
            .fetch(FetchRequest {
                url: victim.url("/other"),
                session: Some(session(&server.base, "private-fixture")),
                ..Default::default()
            })
            .is_err()
    );
    let mut restricted = Fetcher::new(crate::proxy::Route::direct())
        .unwrap()
        .allowing_local()
        .limit_hosts(vec!["example.invalid".into()]);
    assert!(
        restricted
            .fetch(FetchRequest {
                url: server.url("/other"),
                session: Some(session(&server.base, "private-fixture")),
                ..Default::default()
            })
            .is_err()
    );
}
#[test]
fn session_construction_rejects_unsafe_or_unbounded_credentials() {
    for origin in [
        "http://example.com",
        "https://example.com/path",
        "https://example.com/?key=fixture",
        "https://user:pass@example.com",
        "file:///tmp/a",
    ] {
        assert!(HttpSession::new(origin, Default::default(), Default::default()).is_err());
    }
    for (name, value) in [
        ("bad;name", "a"),
        ("login", "value\r\nInjected: 1"),
        ("login", "a;b"),
    ] {
        assert!(
            HttpSession::new(
                "https://example.com",
                BTreeMap::from([(name.into(), value.into())]),
                Default::default()
            )
            .is_err()
        );
    }
    for name in [
        "Cookie",
        "Host",
        "Proxy-Authorization",
        "Content-Length",
        "Accept-Encoding",
    ] {
        assert!(
            HttpSession::new(
                "https://example.com",
                Default::default(),
                BTreeMap::from([(name.into(), "x".into())])
            )
            .is_err()
        );
    }
    assert!(
        HttpSession::new(
            "https://example.com",
            Default::default(),
            BTreeMap::from([("X-Key".into(), "x".repeat(16385))])
        )
        .is_err()
    );
    assert!(
        HttpSession::new(
            "https://example.com",
            Default::default(),
            BTreeMap::from([("X-Key".into(), "a".into()), ("x-key".into(), "b".into())])
        )
        .is_err()
    );
}
#[test]
fn lua_session_permission_and_opaque_handles_are_enforced() {
    let server = Server::start();
    let denied = crate::extensions::Context::default();
    execute_reader_job(
        "assert(xd.http_session==nil and not xd.supports('xd.http_session')); return {}",
        &serde_json::json!({}),
        Some(&denied),
        |_| panic!("No network needed"),
    )
    .unwrap();
    let allowed = crate::extensions::Context {
        http_sessions: true,
        ..Default::default()
    };
    let mut fetcher = Fetcher::new(crate::proxy::Route::direct())
        .unwrap()
        .allowing_local();
    let value=execute_reader_job(r#"
        assert(xd.supports('xd.http_session'))
        local s=xd.http_session(event.url,{cookies={login='local-test'},headers={Authorization='Bearer harmless-fixture'}})
        assert(type(s)=='userdata' and not pcall(xd.to_json,s))
        assert(not pcall(xd.fetch,event.url,{session='fake'}))
        local reply=xd.fetch_bytes(event.url..'/other',{session=s})
        assert(reply.status==200)
        return {ok=true}
    "#,&serde_json::json!({"url":server.base}),Some(&allowed),move|r|fetcher.fetch(r)).unwrap();
    assert_eq!(value["ok"], true);
    assert!(server.last().contains("cookie: login=local-test"));
}
#[test]
fn cookie_path_expiry_and_origin_scopes_are_observed() {
    let mut jar = cookies::Jar::default();
    let url = Url::parse("https://example.com/account/login").unwrap();
    jar.keep(
        &url,
        &[
            "root=one; Path=/".into(),
            "account=two".into(),
            "secure=three; Path=/; Secure".into(),
            "foreign=no; Domain=other.example".into(),
            "expired=no; Max-Age=0".into(),
        ],
    );
    let account = jar.header(&url).unwrap();
    assert!(
        account.starts_with("account=two")
            && account.contains("root=one")
            && !account.contains("foreign")
            && !account.contains("expired")
    );
    assert!(
        !jar.header(&Url::parse("https://example.com/accounts").unwrap())
            .unwrap()
            .contains("account=")
    );
    assert!(
        !jar.header(&Url::parse("http://example.com/").unwrap())
            .unwrap()
            .contains("secure=")
    );
    jar.keep(
        &url,
        &[
            "account=; Max-Age=0".into(),
            "root=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT".into(),
        ],
    );
    assert_eq!(jar.header(&url).unwrap(), "secure=three");
}
