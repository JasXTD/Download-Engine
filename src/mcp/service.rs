//! One owned, cancellable HTTP worker shared by GUI and console hosts.
use super::*;
use anyhow::{Context as _, ensure};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ServerStatus {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed(String),
}

impl ServerStatus {
    pub fn active(&self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}

#[derive(Default)]
pub struct Server {
    status: Arc<Mutex<ServerStatus>>,
    shutdown: CancellationToken,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Server {
    pub fn status(&mut self) -> ServerStatus {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            *self.status.lock().unwrap() = match self.worker.take().unwrap().join() {
                Ok(Ok(())) => ServerStatus::Stopped,
                Ok(Err(error)) => ServerStatus::Failed(format!("{error:#}")),
                Err(_) => {
                    ServerStatus::Failed("MCP server stopped unexpectedly. Start it again.".into())
                }
            };
        }
        self.status.lock().unwrap().clone()
    }

    pub fn start(&mut self, context: Context, port: u16) -> Result<()> {
        self.status();
        ensure!(
            self.worker.is_none(),
            "MCP server is already starting, running or stopping"
        );
        ensure!(port != 0, "Enter a port from 1 to 65535");
        self.shutdown = CancellationToken::new();
        let shutdown = self.shutdown.clone();
        let status = self.status.clone();
        *status.lock().unwrap() = ServerStatus::Starting;
        let worker = std::thread::Builder::new()
            .name("mcp-server".into())
            .spawn(move || {
                let result = http::run_owned(context, port, shutdown.clone(), || {
                    let mut value = status.lock().unwrap();
                    if !shutdown.is_cancelled() {
                        *value = ServerStatus::Running;
                    }
                });
                // A stop during setup wins even if the token lock or listener
                // fails just as cancellation arrives.
                if shutdown.is_cancelled() {
                    Ok(())
                } else {
                    result
                }
            });
        match worker {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => {
                *self.status.lock().unwrap() =
                    ServerStatus::Failed(format!("Cannot start MCP worker: {error}"));
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        let mut status = self.status.lock().unwrap();
        if status.active() {
            self.shutdown.cancel();
            *status = ServerStatus::Stopping;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// A real authenticated HTTP check; does not submit commands or open a session.
pub fn check_connection(paths: &AppPaths, port: u16) -> Result<usize> {
    ensure!(port != 0, "Enter a port from 1 to 65535");
    let token = auth::token(paths)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3)).build()?;
        let mut response = client.post(format!("http://127.0.0.1:{port}/mcp"))
            .bearer_auth(token.as_str())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MODERN).header("Mcp-Method", "tools/list")
            .body(json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{VERSION_META:MODERN,CAPABILITIES_META:{}}}}).to_string())
            .send().await.context("Cannot reach the MCP server")?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len() + chunk.len() <= MAX_OUTPUT, "Unexpected MCP check response size");
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)?;
        // Servers from before 2.0 report the same tools under their old name.
        ensure!(value["jsonrpc"] == "2.0" && value["id"] == 1 && value["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"].as_str().is_some_and(|name| matches!(name, "download-engine" | "xdownloader")), "Unexpected MCP server response");
        let tools = value["result"]["tools"].as_array().context("MCP server did not return its tools")?;
        Ok(tools.len())
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{net::TcpListener, time::Instant};

    fn fixture() -> (tempfile::TempDir, Context) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("downloads"),
        })
        .unwrap();
        (
            root,
            Context {
                store,
                read_only: true,
                permissions: Default::default(),
                no_start: true,
                start: |_| panic!("No desktop launch for lifecycle checks"),
            },
        )
    }
    fn until(server: &mut Server, expected: impl Fn(&ServerStatus) -> bool) -> ServerStatus {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let status = server.status();
            if expected(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "Server did not settle: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn owned_server_handles_port_conflicts_restart_read_only_and_drop() {
        let (_root, mut context) = fixture();
        let reserved = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reserved.local_addr().unwrap().port();
        let mut server = Server::default();
        assert!(server.start(context.clone(), 0).is_err());
        assert_eq!(server.status(), ServerStatus::Stopped);
        server.start(context.clone(), port).unwrap();
        let failed = until(&mut server, |s| matches!(s, ServerStatus::Failed(_)));
        assert!(
            matches!(failed,ServerStatus::Failed(e) if e.contains("Cannot listen") && e.contains(&port.to_string()))
        );
        drop(reserved);
        let config = client_config(&context.store.paths, port).unwrap();
        server.start(context.clone(), port).unwrap();
        assert!(server.start(context.clone(), port).is_err());
        until(&mut server, |s| *s == ServerStatus::Running);
        let read_count = check_connection(&context.store.paths, port).unwrap();
        assert_eq!(read_count, tools::catalog(true).len());
        assert_eq!(client_config(&context.store.paths, port).unwrap(), config);
        server.stop();
        server.stop();
        until(&mut server, |s| *s == ServerStatus::Stopped);
        assert!(check_connection(&context.store.paths, port).is_err());
        context.read_only = false;
        server.start(context.clone(), port).unwrap();
        until(&mut server, |s| *s == ServerStatus::Running);
        // Routine controls are available, but both script tools stay hidden
        // until a script-language capability is explicitly enabled.
        assert_eq!(check_connection(&context.store.paths, port).unwrap(), 20);
        drop(server);
        let _released = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    }

    #[test]
    fn stopping_during_token_initialization_does_not_reopen_the_listener() {
        let (_root, context) = fixture();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(context.store.paths.data.join("mcp-token.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        let reserved = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let mut server = Server::default();
        server.start(context.clone(), port).unwrap();
        server.stop();
        assert_eq!(server.status(), ServerStatus::Stopping);
        assert!(server.start(context.clone(), port).is_err());
        until(&mut server, |s| *s == ServerStatus::Stopped);
        assert!(!context.store.paths.data.join("mcp-token.dpapi").exists());
        drop(lock);
        let _released = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    }
}
