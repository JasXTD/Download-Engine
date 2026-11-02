//! Public loopback identities. macOS needs short-lived certificates; generate
//! them with its installed OpenSSL, read them into memory, and remove all files.
#![allow(dead_code)]
use std::sync::LazyLock;

struct Fixtures {
    cert: Vec<u8>,
    alternate_cert: Vec<u8>,
    identity: Vec<u8>,
    alternate_identity: Vec<u8>,
}
static FIXTURES: LazyLock<Fixtures> = LazyLock::new(|| {
    #[cfg(target_os = "macos")]
    {
        let folder = tempfile::tempdir().unwrap();
        let result = std::process::Command::new("/bin/bash")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/Create-TestTlsFixtures.sh"
            ))
            .arg(folder.path())
            .output()
            .expect("Cannot start the local TLS fixture generator");
        assert!(
            result.status.success(),
            "TLS fixture generation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let fixtures = Fixtures {
            cert: std::fs::read(folder.path().join("server.cer")).unwrap(),
            alternate_cert: std::fs::read(folder.path().join("alternate.cer")).unwrap(),
            identity: std::fs::read(folder.path().join("server.pfx")).unwrap(),
            alternate_identity: std::fs::read(folder.path().join("alternate.pfx")).unwrap(),
        };
        folder.close().unwrap();
        fixtures
    }
    #[cfg(not(target_os = "macos"))]
    Fixtures {
        cert: include_bytes!("../fixtures/ftp/server.cer").to_vec(),
        alternate_cert: include_bytes!("../fixtures/ftp/alternate.cer").to_vec(),
        identity: include_bytes!("../fixtures/ftp/server.pfx").to_vec(),
        alternate_identity: include_bytes!("../fixtures/ftp/alternate.pfx").to_vec(),
    }
});
pub fn cert() -> &'static [u8] {
    &FIXTURES.cert
}
pub fn alternate_cert() -> &'static [u8] {
    &FIXTURES.alternate_cert
}
pub fn identity() -> &'static [u8] {
    &FIXTURES.identity
}
pub fn alternate_identity() -> &'static [u8] {
    &FIXTURES.alternate_identity
}
