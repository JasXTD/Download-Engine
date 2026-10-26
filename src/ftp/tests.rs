use super::*;
use crate::{
    bandwidth::{Limiter, Limits},
    model::{Job, Status},
    paths::AppPaths,
    store::Store,
};
use tokio_util::sync::CancellationToken;
#[path = "../../tests/support/ftp_server.rs"]
mod fixture;
use fixture::{Mode, Server};

struct Cleanup(Store, String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.0.remove(&self.1);
    }
}

fn store() -> (tempfile::TempDir, Store) {
    let home = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        data: home.path().join("data"),
        downloads: home.path().join("downloads"),
    };
    let store = Store::open(paths).unwrap();
    (home, store)
}
fn trusted() -> tokio_native_tls::TlsConnector {
    let mut builder = native_tls::TlsConnector::builder();
    builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
    for bytes in [fixture::CERT, fixture::OTHER_CERT] {
        builder.add_root_certificate(native_tls::Certificate::from_der(bytes).unwrap());
    }
    builder.build().unwrap().into()
}
#[test]
fn urls_and_saved_states_reject_injection_credentials_and_ambiguous_scopes() {
    for url in [
        "ftp://example.com/file.bin",
        "ftps://example.com/file.bin",
        "ftps://example.com:21/file.bin",
    ] {
        assert!(crate::model::validate_download_url(url).is_ok());
        assert!(crate::model::validate_url(url).is_err());
    }
    for url in [
        "ftp://u:p@example.com/f",
        "ftp://example.com/",
        "ftp://example.com/a%0d%0aDELE%20x",
        "ftp://example.com/a%00b",
        "ftp://example.com/f?token=secret",
        "ftp://example.com/f#part",
        "ftp://example.com/%ff",
        "ftp://example.com:0/f",
    ] {
        assert!(crate::model::validate_download_url(url).is_err(), "{url}");
    }
    let a = Endpoint::new("ftps://example.com/f", Security::Implicit).unwrap();
    let b = Endpoint::new("ftps://other.example/f", Security::Implicit).unwrap();
    assert_eq!(a.origin(), "ftps://example.com:990");
    assert_ne!(a.origin(), b.origin());
    assert_eq!(
        Endpoint::new("ftps://example.com/f", Security::Explicit)
            .unwrap()
            .port,
        21
    );
    assert!(Endpoint::new("ftp://example.com/f", Security::Explicit).is_err());
    let mut job = Job::new(
        "ftp://example.com/f",
        None,
        std::env::temp_dir(),
        "fixture".into(),
    )
    .unwrap();
    job.ftp.as_mut().unwrap().version = 9;
    assert!(job.validate_persisted().is_err());
    job.ftp = None;
    assert!(job.validate_persisted().is_err());
}
#[tokio::test]
async fn explicit_and_implicit_ftps_protect_login_and_data_and_publish_exact_bytes() {
    for (mode, security) in [
        (Mode::Explicit, Security::Explicit),
        (Mode::Implicit, Security::Implicit),
    ] {
        let server = Server::new(mode).await;
        server.update(|c| {
            c.require_login = true;
            c.cut = Some(32768);
        });
        let (_home, store) = store();
        let mut job = Job::new(
            &server.url(true),
            None,
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.ftp.as_mut().unwrap().security = security;
        job.status = Status::Paused;
        store.enqueue(&job).unwrap();
        let _cleanup = Cleanup(store.clone(), job.id.clone());
        crate::auth::apply(
            &store,
            crate::auth::Update {
                request: uuid::Uuid::new_v4(),
                job: job.id.clone(),
                expected_url: job.url.clone(),
                credentials: Some(crate::auth::Credentials::basic(
                    "fixture-user".into(),
                    "fixture-password".into(),
                )),
                restart_confirmed: false,
                resume: true,
            },
        )
        .unwrap();
        job = store.get(&job.id).unwrap().unwrap();
        let binding = job.authentication.as_ref().unwrap().id;
        let root = store.paths.parts(&job.id).unwrap();
        let _guards = crate::download_file::prepare_parts(&store.paths.data.join("parts"), &root)
            .await
            .unwrap();
        tokio::fs::create_dir_all(&job.directory).await.unwrap();
        if security == Security::Implicit {
            use sha2::{Digest, Sha256};
            job.integrity.expected = Some(format!(
                "{:x}",
                Sha256::digest(&server.state.config.lock().unwrap().body)
            ));
        }
        assert!(
            transfer::transfer_with(
                &store,
                &mut job,
                &root,
                &CancellationToken::new(),
                &Limiter::default(),
                trusted()
            )
            .await
            .is_err()
        );
        assert_eq!(job.downloaded, 32768);
        job.status = Status::Paused;
        store.save(&job).unwrap();
        let reopened = Store::open(store.paths.clone()).unwrap();
        job = reopened.get(&job.id).unwrap().unwrap();
        assert!(crate::auth::credentials_present(binding).unwrap());
        server.update(|c| c.cut = None);
        let output = transfer::transfer_with(
            &reopened,
            &mut job,
            &root,
            &CancellationToken::new(),
            &Limiter::new(&Limits::default()).unwrap(),
            trusted(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            std::fs::read(&output.path).unwrap(),
            server.state.config.lock().unwrap().body
        );
        assert!(
            server
                .state
                .protected_data
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0
        );
        assert!(
            !serde_json::to_string(&job)
                .unwrap()
                .contains("fixture-password")
        );
        assert!(server.commands().iter().any(|c| c == "PROT P"));
        assert_eq!(
            server.commands().iter().any(|c| c == "REST 32768"),
            security == Security::Implicit
        );
        job.status = Status::Complete;
        job.saved_path = Some(output.path);
        crate::auth::save_finished(&store, &mut job).unwrap();
        crate::auth::cleanup_pending(&store).unwrap();
        assert!(!crate::auth::credentials_present(binding).unwrap());
    }
}

#[tokio::test(start_paused = true)]
async fn network_deadlines_and_cancellation_are_bounded() {
    let start = tokio::time::Instant::now();
    let error = client::wait::<()>(&CancellationToken::new(), std::future::pending())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert_eq!(start.elapsed(), std::time::Duration::from_secs(30));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let start = tokio::time::Instant::now();
    assert!(
        client::wait::<()>(&cancel, std::future::pending())
            .await
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert_eq!(start.elapsed(), std::time::Duration::ZERO);
}

#[test]
fn saved_logins_reject_plain_ftp_bearer_tokens_and_different_server_bindings() {
    let (_home, store) = store();
    for (url, credentials) in [
        (
            "ftp://127.0.0.1/file",
            crate::auth::Credentials::basic("fixture-user".into(), "fixture-password".into()),
        ),
        (
            "ftps://localhost/file",
            crate::auth::Credentials::bearer("fixture-token".into()),
        ),
        (
            "ftps://localhost/file",
            crate::auth::Credentials::basic(
                "fixture-user\r\nDELE file".into(),
                "fixture-password".into(),
            ),
        ),
    ] {
        let mut job = Job::new(
            url,
            None,
            store.paths.downloads.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        job.status = Status::Paused;
        store.enqueue(&job).unwrap();
        let _cleanup = Cleanup(store.clone(), job.id.clone());
        assert!(
            crate::auth::apply(
                &store,
                crate::auth::Update {
                    request: uuid::Uuid::new_v4(),
                    job: job.id.clone(),
                    expected_url: job.url.clone(),
                    credentials: Some(credentials),
                    restart_confirmed: false,
                    resume: false
                }
            )
            .is_err()
        );
        assert!(
            store
                .get(&job.id)
                .unwrap()
                .unwrap()
                .authentication
                .is_none()
        );
    }
    let mut job = Job::new(
        "ftps://localhost:21/file",
        None,
        store.paths.downloads.clone(),
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap();
    job.status = Status::Paused;
    store.enqueue(&job).unwrap();
    let _cleanup = Cleanup(store.clone(), job.id.clone());
    crate::auth::apply(
        &store,
        crate::auth::Update {
            request: uuid::Uuid::new_v4(),
            job: job.id.clone(),
            expected_url: job.url.clone(),
            credentials: Some(crate::auth::Credentials::basic(
                "fixture-user".into(),
                "fixture-password".into(),
            )),
            restart_confirmed: false,
            resume: false,
        },
    )
    .unwrap();
    let saved = store.get(&job.id).unwrap().unwrap();
    let binding = saved.authentication.as_ref().unwrap().id;
    assert!(crate::auth::ftp_credentials(&saved).unwrap().is_some());
    for url in [
        "ftps://other.test:21/file",
        "ftps://localhost:990/file",
        "ftp://localhost/file",
    ] {
        let mut changed = saved.clone();
        changed.url = url.into();
        changed.ftp = State::new(&reqwest::Url::parse(url).unwrap());
        assert!(crate::auth::ftp_credentials(&changed).is_err());
        assert!(changed.validate_persisted().is_err());
    }
    store.remove(&saved.id).unwrap();
    assert!(!crate::auth::credentials_present(binding).unwrap());
}
#[tokio::test]
async fn tls_errors_never_send_credentials_or_downgrade() {
    let credentials =
        crate::auth::Credentials::basic("fixture-user".into(), "fixture-password".into());
    for case in 0..4 {
        let server = Server::new(Mode::Explicit).await;
        server.update(|c| {
            c.require_login = true;
            c.reject_auth = case == 0;
            c.reject_prot = case == 3;
        });
        let mut url = server.url(true);
        if case == 2 {
            url = url.replace("localhost", "127.0.0.1");
        }
        let tls = if case == 1 {
            client::connector().unwrap()
        } else {
            trusted()
        };
        let error = client::Client::connect(
            Endpoint::new(&url, Security::Explicit).unwrap(),
            Some(&credentials),
            tls,
            &CancellationToken::new(),
        )
        .await
        .err()
        .unwrap();
        assert!(!error.to_string().contains("fixture-password"));
        assert!(
            !server
                .commands()
                .iter()
                .any(|c| c.starts_with("USER") || c.starts_with("PASS"))
        );
    }
    let server = Server::new(Mode::Explicit).await;
    server.update(|c| c.other_data_certificate = true);
    let mut client = client::Client::connect(
        Endpoint::new(&server.url(true), Security::Explicit).unwrap(),
        None,
        trusted(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let error = client
        .retrieve(0, &CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("different server certificate"));
}
