use crate::store::Store;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const MAX_FRAME: usize = 1024 * 1024;

#[cfg(test)]
mod torrent_review_tests {
    use super::*;
    #[test]
    fn v2_browser_handoff_preserves_full_hash_without_starting_downloads() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("profile"),
            downloads: temp.path().join("files"),
        })
        .unwrap();
        let link = format!("magnet:?xt=urn:btmh:1220{}", "ab".repeat(32));
        let reply = handle_request(
            &store,
            Request::ReviewTorrent {
                request_id: "v2-review".into(),
                url: link,
            },
        );
        assert!(reply.ok, "{}", reply.message);
        assert_eq!(reply.preview_id.as_deref(), Some("v2-review"));
        assert!(store.list().unwrap().is_empty());
        assert!(!store.torrents.is_running());
        assert!(!store.torrent_settings().unwrap().enabled);
    }
    #[test]
    fn browser_magnets_are_only_deduplicated_review_offers_never_downloads() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(crate::paths::AppPaths {
            data: temp.path().join("profile"),
            downloads: temp.path().join("files"),
        })
        .unwrap();
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        for id in ["one", "two"] {
            let reply = handle_request(
                &store,
                Request::ReviewTorrent {
                    request_id: id.into(),
                    url: magnet.into(),
                },
            );
            assert!(reply.ok, "{}", reply.message);
            assert_eq!(reply.preview_id.as_deref(), Some(id));
        }
        let count: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM external_open_requests", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        assert!(store.list().unwrap().is_empty());
        assert!(!store.torrent_settings().unwrap().enabled);
        assert!(!store.torrents.is_running());
        for value in [
            "file:///C:/private.torrent",
            "C:\\private.torrent",
            "https://example.invalid/a.torrent",
            "magnet:?xt=urn:btih:bad",
        ] {
            assert!(
                !handle_request(
                    &store,
                    Request::ReviewTorrent {
                        request_id: "bad".into(),
                        url: value.into()
                    }
                )
                .ok
            );
        }
        assert!(
            !handle_request(
                &store,
                Request::ReviewTorrent {
                    request_id: String::new(),
                    url: magnet.into()
                }
            )
            .ok
        );
    }
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Ping,
    OpenBrowserVideoRelay {
        page: String,
        origins: Vec<String>,
    },
    ReviewTorrent {
        request_id: String,
        url: String,
    },
    BatchOptions,
    RecoveryPending,
    RecoveryStatus {
        id: uuid::Uuid,
    },
    RecoveryOffer {
        id: uuid::Uuid,
        file: crate::browser_recovery::Candidate,
    },
    RecoveryCancel {
        id: uuid::Uuid,
    },
    ReviewBrowserVideo {
        request_id: String,
        review: crate::browser_video::Review,
    },
    PreviewHls {
        request_id: String,
        url: String,
    },
    PreviewVideo {
        request_id: String,
        url: String,
    },
    AddBatch {
        request_id: String,
        links: Vec<crate::browser_batch::Link>,
        queue_id: String,
        destination: crate::browser_batch::Destination,
    },
    Add {
        request_id: String,
        url: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        source_page: Option<String>,
    },
    AddBrowserLogin {
        request_id: String,
        url: String,
        cookies: Vec<crate::browser_login::Cookie>,
    },
    CapturePrepare {
        request_id: String,
        url: String,
        filename: Option<String>,
        size: u64,
        mime: String,
        #[serde(default)]
        source_page: Option<String>,
    },
    CaptureCommit {
        request_id: String,
    },
    CaptureStatus {
        request_id: String,
    },
    CaptureAbort {
        request_id: String,
    },
}
impl Request {
    pub fn diagnostic_action(&self) -> crate::diagnostics::NativeAction {
        use crate::diagnostics::NativeAction as Action;
        match self {
            Self::Ping => Action::Ping,
            Self::ReviewTorrent { .. } => Action::Add,
            Self::BatchOptions => Action::BatchOptions,
            Self::RecoveryPending
            | Self::RecoveryStatus { .. }
            | Self::RecoveryOffer { .. }
            | Self::RecoveryCancel { .. } => Action::BrowserRecovery,
            Self::ReviewBrowserVideo { .. } | Self::OpenBrowserVideoRelay { .. } => {
                Action::ReviewVideo
            }
            Self::PreviewHls { .. } | Self::PreviewVideo { .. } => Action::PreviewVideo,
            Self::AddBatch { .. } => Action::AddBatch,
            Self::Add { .. } => Action::Add,
            Self::AddBrowserLogin { .. } => Action::AddBrowserLogin,
            Self::CapturePrepare { .. } => Action::CapturePrepare,
            Self::CaptureCommit { .. } => Action::CaptureCommit,
            Self::CaptureStatus { .. } => Action::CaptureStatus,
            Self::CaptureAbort { .. } => Action::CaptureAbort,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_relay: Option<crate::browser_relay::Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<crate::browser_recovery::Session>,
    pub ok: bool,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_state: Option<crate::store::CaptureState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub job_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_options: Option<crate::browser_batch::Options>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_id: Option<String>,
}
pub fn read_frame(reader: &mut impl Read) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>> {
    let mut length = [0; 4];
    let first = reader.read(&mut length[..1])?;
    if first == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME,
        "Invalid native message size"
    );
    let mut data = zeroize::Zeroizing::new(vec![0; length]);
    reader.read_exact(&mut data)?;
    Ok(Some(data))
}
pub fn write_frame(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_FRAME, "Reply is too large");
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}
pub fn handle(store: &Store, data: &[u8]) -> Reply {
    match serde_json::from_slice(data) {
        Ok(request) => handle_request(store, request),
        Err(_) => Reply::failure("Invalid native request"),
    }
}
pub fn handle_request(store: &Store, request: Request) -> Reply {
    let action = request.diagnostic_action();
    process(store, request).unwrap_or_else(|error| {
        use crate::diagnostics::{self, Details, Event, Failure, Level};
        diagnostics::record(
            Level::Warn,
            Event::NativeOperationFailed,
            Details {
                action: Some(action),
                failure: Some(Failure::error(&error)),
                ..Default::default()
            },
        );
        Reply::failure(error)
    })
}
fn process(store: &Store, request: Request) -> Result<Reply> {
    use crate::{
        capture::{self, ExpectedFile},
        store::CaptureState,
    };
    match request {
        Request::OpenBrowserVideoRelay { .. } => {
            anyhow::bail!("Browser video transfers require a persistent native connection")
        }
        Request::Ping => {
            let mut reply = Reply::success("Download Engine is connected");
            reply.capabilities.extend([
                "automatic_capture_v1".into(),
                "page_links_batch_v1".into(),
                "browser_login_v1".into(),
                "hls_preview_v1".into(),
                "video_preview_v1".into(),
                "browser_video_review_v1".into(),
                "browser_video_alternatives_v1".into(),
                "browser_video_user_agent_v1".into(),
                "browser_video_relay_v1".into(),
                "browser_link_recovery_v1".into(),
                "torrent_review_v1".into(),
            ]);
            Ok(reply)
        }
        Request::ReviewTorrent { request_id, url } => {
            ensure!(
                !request_id.is_empty()
                    && request_id.len() <= 128
                    && !request_id.chars().any(char::is_control),
                "Invalid request identifier"
            );
            // Browser messages may offer a magnet, never read a local path or
            // enable torrents. The desktop owns consent and metadata discovery.
            crate::torrent::info_hash(&url)?;
            store.submit_external_opens(&[crate::external_open::Target::Magnet(url)])?;
            let mut reply = Reply::success("Torrent is ready for review in Download Engine");
            reply.preview_id = Some(request_id);
            Ok(reply)
        }
        Request::BatchOptions => {
            let mut reply = Reply::success("Queues and folders loaded");
            reply.batch_options = Some(store.browser_batch_options()?);
            Ok(reply)
        }
        Request::RecoveryPending => {
            let mut reply = Reply::success("Browser recovery checked");
            reply.recovery = store.pending_browser_recovery()?;
            Ok(reply)
        }
        Request::RecoveryStatus { id } => {
            let mut reply = Reply::success("Browser recovery status");
            reply.recovery = store.browser_recovery_status(id)?;
            Ok(reply)
        }
        Request::RecoveryOffer { id, file } => {
            let mut reply =
                Reply::success("Fresh link sent. Review and confirm in Download Engine");
            reply.recovery = Some(store.offer_browser_recovery(id, file)?);
            Ok(reply)
        }
        Request::RecoveryCancel { id } => {
            let mut reply = Reply::success("Browser recovery cancelled");
            reply.recovery = store.cancel_browser_recovery(id)?;
            Ok(reply)
        }
        Request::ReviewBrowserVideo { request_id, review } => {
            let pending = store.request_browser_video_review(&request_id, &review)?;
            let mut reply = Reply::success(if pending {
                "Review the video in Download Engine. Nothing has been added to the queue."
            } else {
                "This video review was already handled or expired. Click the video button again for a new review."
            });
            reply.preview_id = Some(request_id);
            Ok(reply)
        }
        Request::PreviewHls { request_id, url } | Request::PreviewVideo { request_id, url } => {
            store.request_hls_preview(&request_id, &url, None)?;
            let mut reply = Reply::success(
                "Open Download Engine to choose video quality and audio, then review the preview. Nothing has been added to the queue.",
            );
            reply.preview_id = Some(request_id);
            Ok(reply)
        }
        Request::AddBatch {
            request_id,
            links,
            queue_id,
            destination,
        } => {
            let ids = store.enqueue_browser_batch(&request_id, &links, &queue_id, destination)?;
            let mut reply = Reply::success(format!("Saved {} files to Download Engine", ids.len()));
            reply.job_ids = ids;
            Ok(reply)
        }
        Request::Add {
            request_id,
            url,
            filename,
            source_page,
        } => {
            crate::model::validate_url(&url)?;
            ensure!(
                !crate::hls::is_playlist_url(&url),
                "Use Preview video to choose a quality first"
            );
            ensure!(
                !request_id.starts_with("capture:")
                    && !request_id.starts_with("batch:")
                    && !request_id.starts_with("login:"),
                "Reserved request identifier"
            );
            let mut job = store.new_download(&url, filename.as_deref(), None, request_id)?;
            job.source_page = source_page
                .map(|url| crate::model::validate_url(&url).map(|u| u.to_string()))
                .transpose()?;
            let saved = store.enqueue(&job)?;
            let mut reply = Reply::success("Added to the download queue");
            reply.job_id = Some(saved.id);
            Ok(reply)
        }
        Request::AddBrowserLogin {
            request_id,
            url,
            cookies,
        } => {
            let id = crate::browser_login::enqueue(store, request_id, url, cookies)?;
            let mut reply = Reply::success("Added with browser login");
            reply.job_id = Some(id);
            Ok(reply)
        }
        Request::CapturePrepare {
            request_id,
            url,
            filename,
            size,
            mime,
            source_page,
        } => {
            crate::model::validate_url(&url)?;
            capture::validate_request_id(&request_id)?;
            let mut job = store.new_download(&url, filename.as_deref(), None, request_id)?;
            job.source_page = source_page
                .map(|url| crate::model::validate_url(&url).map(|u| u.to_string()))
                .transpose()?;
            let expected = ExpectedFile::browser(size, &mime)?;
            let previous = store.capture_status(&job.request_id)?;
            if previous.state != CaptureState::Missing {
                ensure!(
                    previous.state != CaptureState::Aborted,
                    "Browser handoff was abandoned"
                );
                if previous.state == CaptureState::Committed {
                    return Ok(Reply::capture(previous));
                }
                let saved = previous.job.as_ref().context("Missing capture data")?;
                ensure!(
                    saved.url == job.url
                        && saved
                            .expected_file
                            .as_ref()
                            .is_some_and(|v| v.size == expected.size && v.mime == expected.mime),
                    "Capture identifier belongs to a different file"
                );
                return Ok(Reply::capture(previous));
            }
            // The native messaging executable is synchronous; HTTP gets its own
            // short-lived runtime and an eight-second request deadline.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(capture::preflight(store, &mut job, expected))?;
            Ok(Reply::capture(store.prepare_capture(&job)?))
        }
        Request::CaptureCommit { request_id } => {
            capture::validate_request_id(&request_id)?;
            Ok(Reply::capture(store.commit_capture(&request_id)?))
        }
        Request::CaptureStatus { request_id } => {
            capture::validate_request_id(&request_id)?;
            Ok(Reply::capture(store.capture_status(&request_id)?))
        }
        Request::CaptureAbort { request_id } => {
            capture::validate_request_id(&request_id)?;
            Ok(Reply::capture(store.abort_capture(&request_id)?))
        }
    }
}
impl Reply {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            version: 1,
            job_id: None,
            message: message.into(),
            capture_state: None,
            capabilities: Vec::new(),
            job_ids: Vec::new(),
            batch_options: None,
            preview_id: None,
            recovery: None,
            browser_relay: None,
        }
    }
    pub fn failure(error: impl std::fmt::Display) -> Self {
        Self {
            ok: false,
            ..Self::success(error.to_string())
        }
    }
    fn capture(receipt: crate::store::CaptureReceipt) -> Self {
        use crate::store::CaptureState;
        let message = match receipt.state {
            CaptureState::Missing => "No browser handoff is saved",
            CaptureState::Prepared => "File checked and ready to transfer",
            CaptureState::Committed => "Added to the download queue",
            CaptureState::Aborted => "Download kept in the browser",
        };
        Self {
            capture_state: Some(receipt.state),
            job_id: receipt.job_id,
            ..Self::success(message)
        }
    }
}
