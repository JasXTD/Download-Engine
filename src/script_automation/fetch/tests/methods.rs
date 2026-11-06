use super::*;
#[test]
fn additional_methods_preserve_bodies_and_head_does_not_download_the_resource() {
    let server = Server::start();
    let mut fetch = fetcher().allowing_local();
    for method in ["PUT", "PATCH", "DELETE", "OPTIONS"] {
        fetch
            .fetch(FetchRequest {
                method: Some(method.into()),
                body: Some(b"owned-body".to_vec()),
                ..get(&server.url("/same307"))
            })
            .unwrap();
        let request = server.last();
        assert!(request.starts_with(&format!("{} /other", method.to_ascii_lowercase())));
        assert!(request.ends_with("owned-body"));
    }
    let reply = fetch
        .fetch(FetchRequest {
            method: Some("HEAD".into()),
            ..get(&server.url("/huge"))
        })
        .unwrap();
    assert_eq!(reply.status, 200);
    assert!(reply.body.is_empty());
    fetch
        .fetch(FetchRequest {
            method: Some("HEAD".into()),
            ..get(&server.url("/form"))
        })
        .unwrap();
    assert!(server.last().starts_with("head /page"));
    assert!(
        fetch
            .fetch(FetchRequest {
                method: Some("HEAD".into()),
                body: Some(vec![1]),
                ..get(&server.url("/"))
            })
            .is_err()
    );
    assert!(
        fetch
            .fetch(FetchRequest {
                method: Some("CONNECT".into()),
                ..get(&server.url("/"))
            })
            .is_err()
    );
}
#[test]
fn supplied_user_agent_and_learned_cookies_survive_a_protected_handoff() {
    let server = Server::start();
    let session = crate::script_automation::HttpSession::new(
        &server.base,
        Default::default(),
        BTreeMap::from([
            ("User-Agent".into(), "Fixture/1".into()),
            ("Authorization".into(), "Bearer owned".into()),
        ]),
    )
    .unwrap();
    let mut fetch = fetcher().allowing_local();
    fetch
        .fetch(FetchRequest {
            session: Some(session.clone()),
            ..get(&server.url("/page"))
        })
        .unwrap();
    let scope = uuid::Uuid::new_v4();
    let sealed = session.seal(scope).unwrap();
    assert!(!sealed.contains("owned") && !sealed.contains("abc123"));
    let recovered = crate::script_automation::HttpSession::open(scope, &sealed).unwrap();
    assert!(crate::script_automation::HttpSession::open(uuid::Uuid::new_v4(), &sealed).is_err());
    assert_eq!(
        recovered
            .cookie_header(&reqwest::Url::parse(&server.url("/other")).unwrap())
            .unwrap()
            .as_deref(),
        Some("session=abc123")
    );
    fetch
        .fetch(FetchRequest {
            session: Some(recovered.clone()),
            ..get(&server.url("/other"))
        })
        .unwrap();
    assert!(
        server.last().contains("user-agent: fixture/1")
            && server.last().contains("cookie: session=abc123")
    );
    assert!(
        recovered
            .cookie_header(&reqwest::Url::parse("https://elsewhere.invalid/").unwrap())
            .is_err()
    );
}
