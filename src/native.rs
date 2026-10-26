use crate::store::Store;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const MAX_FRAME: usize = 1024 * 1024;
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Ping,
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
            Self::BatchOptions => Action::BatchOptions,
            Self::RecoveryPending
            | Self::RecoveryStatus { .. }
            | Self::RecoveryOffer { .. }
            | Self::RecoveryCancel { .. } => Action::BrowserRecovery,
            Self::ReviewBrowserVideo { .. } => Action::ReviewVideo,
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
pub fn write_frame(writer: &mut impl Write, value: &Reply) -> Result<()> {
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
        Request::Ping => {
            let mut reply = Reply::success("Download Engine is connected");
            reply.capabilities.extend([
                "automatic_capture_v1".into(),
                "page_links_batch_v1".into(),
                "browser_login_v1".into(),
                "hls_preview_v1".into(),
                "video_preview_v1".into(),
                "browser_video_review_v1".into(),
                "browser_link_recovery_v1".into(),
            ]);
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
