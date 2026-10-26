//! Twitch's public GraphQL endpoint, called with the client ID of Twitch's own
//! web player. Anonymous only: no account cookies or OAuth tokens are sent.
use crate::{
    engine,
    network_scope::{NetworkScope, network_scope},
    website_grabber::pinned_resolution,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, header};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const GQL: &str = "https://gql.twitch.tv/gql";
/// twitch.tv's own public web client ID (yt-dlp and streamlink use it too).
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const QUERY: &str = r#"query($id: ID!) { video(id: $id) { id title owner { displayName } } videoPlaybackAccessToken(id: $id, params: {platform: "web", playerBackend: "mediaplayer", playerType: "site"}) { value signature } }"#;
const MAX_BODY: usize = 1024 * 1024;
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

pub(super) async fn query(
    id: &str,
    cancel: &CancellationToken,
    route: &crate::proxy::Route,
) -> Result<Value> {
    let url = Url::parse(GQL)?;
    let resolved = tokio::select! { biased;
        _ = cancel.cancelled() => bail!("Twitch lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(10), pinned_resolution(&url)) => result.context("Twitch DNS lookup timed out")??,
    }
    .context("Twitch hostname did not resolve")?;
    ensure!(!resolved.1.is_empty(), "Twitch hostname did not resolve");
    for address in &resolved.1 {
        ensure!(
            network_scope(address.ip())? == NetworkScope::Public,
            "Twitch resolved to a non-public server"
        );
    }
    let client = route
        .configure(engine::client_builder())?
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&resolved.0, &resolved.1)
        .build()?;
    let body = serde_json::json!({ "query": QUERY, "variables": { "id": id } }).to_string();
    let read = async {
        let (client, request) = client
            .post(url)
            .header(header::USER_AGENT, WEB_UA)
            .header("Client-ID", CLIENT_ID)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT_ENCODING, "identity")
            .body(body)
            .build_split();
        let mut response = route.execute(&client, request?, None).await?;
        let status = response.status();
        ensure!(
            status == reqwest::StatusCode::OK,
            "Twitch returned HTTP {}",
            status.as_u16()
        );
        engine::identity(&response)?;
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_BODY as u64),
            "Twitch reply exceeds 1 MiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= MAX_BODY,
                "Twitch reply exceeds 1 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("Invalid Twitch reply")
    };
    tokio::select! { biased;
        _ = cancel.cancelled() => bail!("Twitch lookup cancelled"),
        result = tokio::time::timeout(Duration::from_secs(25), read) => result.context("Twitch request timed out")?,
    }
}
