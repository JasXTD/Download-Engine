//! Bounded, persisted retry policy. Waiting jobs release their worker/queue slot.
use crate::model::{Job, Status};
use anyhow::Result;
use reqwest::{Response, header};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, io::ErrorKind, time::UNIX_EPOCH};

pub const MAX_RETRIES: u8 = 3;
const BASE_DELAY_MS: i64 = 5_000;
const MAX_SERVER_WAIT_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Number of automatic retries scheduled in this user-initiated run.
    pub attempts: u8,
    /// UTC milliseconds. None while transferring or when no retry is pending.
    pub next_at: Option<i64>,
}
impl State {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn ready(&self, now: i64) -> bool {
        self.next_at.is_none_or(|at| at <= now)
    }
}

#[derive(Debug)]
struct Transient {
    message: String,
    not_before: Option<i64>,
}
impl fmt::Display for Transient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl Error for Transient {}

pub(crate) fn incomplete(message: &str) -> anyhow::Error {
    Transient {
        message: message.into(),
        not_before: None,
    }
    .into()
}

/// Only network errors reach this function: never classify filesystem/DB I/O as retryable.
pub(crate) fn network(error: reqwest::Error) -> anyhow::Error {
    let error = error.without_url();
    if error.is_builder() || error.is_redirect() {
        return error.into();
    }
    if error.is_timeout() {
        return incomplete("The server connection timed out.");
    }
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionRefused
                    | ErrorKind::NotConnected
                    | ErrorKind::TimedOut
                    | ErrorKind::Interrupted
                    | ErrorKind::UnexpectedEof
                    | ErrorKind::BrokenPipe
                    | ErrorKind::NetworkUnreachable
                    | ErrorKind::HostUnreachable
            )
        {
            return incomplete(
                "The server connection was interrupted or is temporarily unavailable.",
            );
        }
        if let Some(http) = cause.downcast_ref::<hyper::Error>()
            && (http.is_incomplete_message() || http.is_closed())
        {
            return incomplete("The server connection ended before the response was complete.");
        }
        source = cause.source();
    }
    // Certificate, proxy, malformed-response, and unknown failures need review.
    error.into()
}

pub(crate) fn response(response: &Response) -> Result<()> {
    let status = response.status().as_u16();
    if matches!(status, 408 | 429 | 500 | 502 | 503 | 504) {
        let now = chrono::Utc::now().timestamp_millis();
        let not_before = retry_after(response.headers(), now).map_err(|()| anyhow::anyhow!(
            "Server returned HTTP {status} and requested a retry delay longer than 24 hours. Resume later when ready."
        ))?;
        return Err(Transient {
            message: format!("Server returned HTTP {status}."),
            not_before,
        }
        .into());
    }
    Ok(())
}

fn retry_after(headers: &header::HeaderMap, now: i64) -> std::result::Result<Option<i64>, ()> {
    let mut latest = None;
    for value in headers.get_all(header::RETRY_AFTER) {
        let Ok(value) = value.to_str() else { continue };
        let value = value.trim();
        let at = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            // Overflow is a very long server delay, never permission to retry early.
            let seconds = value.parse::<i64>().map_err(|_| ())?;
            if seconds > MAX_SERVER_WAIT_MS / 1000 {
                return Err(());
            }
            now.checked_add(seconds * 1000).ok_or(())?
        } else if let Ok(date) = httpdate::parse_http_date(value) {
            let millis = date
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            i64::try_from(millis).map_err(|_| ())?
        } else {
            continue; // An invalid header does not replace the normal backoff.
        };
        if at.saturating_sub(now) > MAX_SERVER_WAIT_MS {
            return Err(());
        }
        latest = Some(latest.map_or(at, |previous: i64| previous.max(at)));
    }
    Ok(latest)
}

pub(crate) fn failed(job: &mut Job, error: &anyhow::Error, now: i64) {
    job.status = Status::Failed;
    job.retry.next_at = None;
    job.error = Some(format!("{error:#}"));
    let Some(transient) = error.downcast_ref::<Transient>() else {
        return;
    };
    if job.retry.attempts >= MAX_RETRIES {
        job.error = Some(format!(
            "Automatic retries exhausted ({MAX_RETRIES} retries). {error:#} Resume when ready."
        ));
        return;
    }
    let delay = BASE_DELAY_MS * (1_i64 << job.retry.attempts);
    job.retry.attempts += 1;
    job.retry.next_at = Some(
        now.saturating_add(delay)
            .max(transient.not_before.unwrap_or(now)),
    );
    job.status = Status::Queued;
}

/// Drain every range worker. A disk/validation failure takes precedence over a
/// transient error; otherwise preserve the longest Retry-After across workers.
pub(crate) fn merge_error(current: &mut Option<anyhow::Error>, incoming: anyhow::Error) {
    let replace = match current.as_ref() {
        None => true,
        Some(previous) if crate::quota::reached(previous) => {
            !crate::quota::reached(&incoming) && incoming.downcast_ref::<Transient>().is_none()
        }
        Some(previous) => match (
            previous.downcast_ref::<Transient>(),
            incoming.downcast_ref::<Transient>(),
        ) {
            (Some(_), None) => true,
            (Some(a), Some(b)) => b.not_before > a.not_before,
            _ => false,
        },
    };
    if replace {
        *current = Some(incoming);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job() -> Job {
        Job::new(
            "https://example.com/file",
            None,
            std::env::temp_dir(),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap()
    }
    fn header(value: &str) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::RETRY_AFTER, value.parse().unwrap());
        headers
    }
    #[test]
    fn backoff_is_bounded_persisted_and_never_resets_on_progress() {
        let mut job = job();
        let error = incomplete("Temporary failure").context("Reading the file");
        for (attempt, delay) in [(1, 5000), (2, 10000), (3, 20000)] {
            job.downloaded += 123; // Progress must not grant another retry budget.
            failed(&mut job, &error, 1000);
            assert_eq!(job.status, Status::Queued);
            assert_eq!(job.retry.attempts, attempt);
            assert_eq!(job.retry.next_at, Some(1000 + delay));
            let json = serde_json::to_string(&job).unwrap();
            job = serde_json::from_str(&json).unwrap();
            assert!(!job.retry.ready(999 + delay));
            assert!(job.retry.ready(1000 + delay));
        }
        failed(&mut job, &error, 50000);
        assert_eq!(job.status, Status::Failed);
        assert_eq!(job.retry.attempts, MAX_RETRIES);
        assert!(job.retry.next_at.is_none());
        assert!(job.error.unwrap().contains("exhausted"));
    }
    #[test]
    fn server_delay_uses_longest_value_and_rejects_overflow_or_excessive_wait() {
        let now = 784111767000; // 1994-11-06 08:49:27 UTC
        for value in [
            "10",
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(
                retry_after(&header(value), now).unwrap(),
                Some(now + 10000),
                "{value}"
            );
        }
        let mut headers = header("10");
        headers.append(header::RETRY_AFTER, "20".parse().unwrap());
        assert_eq!(retry_after(&headers, now).unwrap(), Some(now + 20000));
        for value in ["invalid", "-10", "+10", "1.5"] {
            assert_eq!(retry_after(&header(value), now).unwrap(), None);
        }
        for value in [
            "86401",
            "999999999999999999999999999",
            "Sat, 06 Nov 2094 08:49:37 GMT",
        ] {
            assert!(retry_after(&header(value), now).is_err());
        }
        let mut job = job();
        let long = Transient {
            message: "Busy".into(),
            not_before: Some(now + 60000),
        }
        .into();
        failed(&mut job, &long, now);
        assert_eq!(job.retry.next_at, Some(now + 60000));
        let past = Transient {
            message: "Busy".into(),
            not_before: Some(now - 10000),
        }
        .into();
        failed(&mut job, &past, now);
        assert_eq!(job.retry.next_at, Some(now + 10000));
    }
    #[test]
    fn terminal_failures_override_transient_ranges_and_never_retry() {
        let mut error = Some(incomplete("Interrupted"));
        merge_error(
            &mut error,
            Transient {
                message: "Busy".into(),
                not_before: Some(90000),
            }
            .into(),
        );
        merge_error(&mut error, incomplete("Interrupted again"));
        let mut job = job();
        failed(&mut job, error.as_ref().unwrap(), 1000);
        assert_eq!(job.retry.next_at, Some(90000));
        merge_error(
            &mut error,
            std::io::Error::new(ErrorKind::PermissionDenied, "disk denied").into(),
        );
        merge_error(&mut error, incomplete("Interrupted"));
        failed(&mut job, error.as_ref().unwrap(), 1000);
        assert_eq!(job.status, Status::Failed);
        assert!(job.retry.next_at.is_none());
        assert!(job.error.unwrap().contains("disk denied"));
    }
    #[test]
    fn legacy_jobs_default_to_no_retry_metadata() {
        let job = job();
        let json = serde_json::to_string(&job).unwrap();
        assert!(!json.contains("retry"));
        let restored: Job = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.retry, State::default());
    }
    #[tokio::test]
    async fn timeouts_and_connections_closed_before_headers_are_transient() {
        for stall in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!(
                "http://{}/file?private-token",
                listener.local_addr().unwrap()
            );
            let server = tokio::spawn(async move {
                let (_stream, _) = listener.accept().await.unwrap();
                if stall {
                    std::future::pending::<()>().await;
                }
            });
            let client = crate::engine::client().unwrap();
            let error = client
                .get(url)
                .timeout(std::time::Duration::from_millis(150))
                .send()
                .await
                .unwrap_err();
            if stall {
                assert!(error.is_timeout());
            }
            let mut job = job();
            failed(&mut job, &network(error), 1000);
            assert_eq!(job.status, Status::Queued, "{:?}", job.error);
            assert!(!job.error.unwrap().contains("private-token"));
            server.abort();
        }
    }
}
