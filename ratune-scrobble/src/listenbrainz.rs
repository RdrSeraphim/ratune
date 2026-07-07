//! ListenBrainz API client (JSON POST with bearer token).
//!
//! ## API
//! - Endpoint: `POST https://api.listenbrainz.org/1/submit-listens`
//! - Auth: `Authorization: Token <user_token>` header
//! - `listen_type`: `"playing_now"` (no timestamp) or `"single"` (with `listened_at`)
//!
//! Reference: <https://listenbrainz.readthedocs.io/en/latest/users/api/core.html>

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::json;

use crate::track::TrackInfo;

/// ListenBrainz API base URL.
const LISTENBRAINZ_API: &str = "https://api.listenbrainz.org/1/submit-listens";

/// Authenticated client for `POST /1/submit-listens`.
#[derive(Debug, Clone)]
pub struct ListenBrainzClient {
    http: Client,
    user_token: String,
}

impl ListenBrainzClient {
    pub fn new(user_token: String) -> Self {
        Self {
            http: Client::new(),
            user_token,
        }
    }

    /// Announce the currently playing track (`listen_type: "playing_now"`).
    ///
    /// `listened_at` is omitted — ListenBrainz stores playing-now notifications
    /// temporarily and replaces them on subsequent calls.
    pub async fn update_now_playing(&self, track: &TrackInfo) -> Result<()> {
        let body = json!({
            "listen_type": "playing_now",
            "payload": [track_payload(track, None)]
        });
        self.post(body).await
    }

    /// Submit a completed listen (`listen_type: "single"`).
    ///
    /// `timestamp` is Unix seconds when playback *started*.
    pub async fn scrobble(&self, track: &TrackInfo, timestamp: i64) -> Result<()> {
        let body = json!({
            "listen_type": "single",
            "payload": [track_payload(track, Some(timestamp))]
        });
        self.post(body).await
    }

    async fn post(&self, body: serde_json::Value) -> Result<()> {
        let resp = self
            .http
            .post(LISTENBRAINZ_API)
            .header("Authorization", format!("Token {}", self.user_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("POST to ListenBrainz")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let msg = if text.is_empty() {
                format!("HTTP {status}")
            } else {
                // Try to extract a concise error from the JSON response
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| format!("HTTP {status}: {s}"))
                        .unwrap_or_else(|| format!("HTTP {status}: {text}"))
                } else {
                    format!("HTTP {status}: {text}")
                }
            };
            bail!("ListenBrainz error: {msg}");
        }
        Ok(())
    }
}

/// Build a single listen payload object.
fn track_payload(track: &TrackInfo, timestamp: Option<i64>) -> serde_json::Value {
    let mut additional = json!({
        "media_player": "ratune",
        "submission_client": "ratune",
    });
    if let Some(d) = track.duration_secs {
        additional["duration_ms"] = json!(d as u64 * 1000);
    }

    let mut meta = json!({
        "artist_name": track.artist,
        "track_name": track.title,
        "additional_info": additional,
    });
    if let Some(ref album) = track.album {
        if !album.is_empty() {
            meta["release_name"] = json!(album);
        }
    }

    let mut payload = json!({
        "track_metadata": meta,
    });
    if let Some(ts) = timestamp {
        payload["listened_at"] = json!(ts);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackInfo;

    #[test]
    fn playing_now_payload_omits_timestamp() {
        let track = TrackInfo {
            song_id: String::new(),
            artist: "Artist".into(),
            title: "Title".into(),
            album: Some("Album".into()),
            track_number: Some(3),
            duration_secs: Some(240),
        };
        let payload = track_payload(&track, None);
        assert_eq!(payload["track_metadata"]["artist_name"], "Artist");
        assert_eq!(payload["track_metadata"]["track_name"], "Title");
        assert_eq!(payload["track_metadata"]["release_name"], "Album");
        assert_eq!(
            payload["track_metadata"]["additional_info"]["duration_ms"],
            240_000
        );
        assert!(payload.get("listened_at").is_none());
    }

    #[test]
    fn scrobble_payload_includes_timestamp() {
        let track = TrackInfo {
            song_id: String::new(),
            artist: "A".into(),
            title: "T".into(),
            album: None,
            track_number: None,
            duration_secs: None,
        };
        let payload = track_payload(&track, Some(1_234_567_890));
        assert_eq!(payload["listened_at"], 1_234_567_890);
        assert!(payload["track_metadata"]["additional_info"]
            .get("duration_ms")
            .is_none());
    }

    #[test]
    fn client_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ListenBrainzClient>();
    }
}