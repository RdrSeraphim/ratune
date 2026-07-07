use discord_rich_presence::{
    activity::{self, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use reqwest::multipart;
use std::collections::HashMap;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone)]
pub enum DiscordNotify {
    Update {
        artist: String,
        song_name: String,
        album: String,
        start_time: i64,
        end_time: Option<i64>,
        paused: bool,
        cover_id: Option<String>,
        cover_bytes: Option<Vec<u8>>,
    },
    Clear,
    Shutdown,
}

pub struct DiscordLink {
    notify_tx: UnboundedSender<DiscordNotify>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl DiscordLink {
    pub fn notify_update(&self, update: DiscordNotify) {
        let _ = self.notify_tx.send(update);
    }

    pub fn shutdown(mut self) {
        let _ = self.notify_tx.send(DiscordNotify::Shutdown);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

pub fn setup(enabled: bool) -> Option<DiscordLink> {
    if !enabled {
        return None;
    }
    let (notify_tx, notify_rx) = unbounded_channel();
    let thread = spawn_discord_worker(notify_rx);
    Some(DiscordLink {
        notify_tx,
        thread: Some(thread),
    })
}

#[derive(serde::Deserialize)]
struct UguuFile {
    url: String,
}

#[derive(serde::Deserialize)]
struct UguuResponse {
    success: bool,
    files: Vec<UguuFile>,
}

fn spawn_discord_worker(mut rx: UnboundedReceiver<DiscordNotify>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warn: discord: could not create async runtime: {e}");
                return;
            }
        };

        rt.block_on(async move {
            let mut client = DiscordIpcClient::new("1523923868887289947");
            if let Err(e) = client.connect() {
                eprintln!("warn: discord: failed to connect to Discord IPC: {e}");
                return;
            }

            let http_client = reqwest::Client::builder()
                .user_agent(concat!("ratune/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let mut uguu_cache: HashMap<String, String> = HashMap::new();

            let mut latest_state: Option<DiscordNotify> = None;
            let mut force_update = false;

            loop {
                let timeout_dur = if force_update {
                    std::time::Duration::from_secs(5)
                } else {
                    std::time::Duration::from_secs(3600)
                };

                match tokio::time::timeout(timeout_dur, rx.recv()).await {
                    Ok(Some(msg)) => {
                        latest_state = Some(msg);
                        force_update = true;
                    }
                    Ok(None) => break,
                    Err(_) => {
                        // Timeout triggered, meaning we should retry the last state
                    }
                }

                if force_update {
                    if let Some(ref mut state) = latest_state {
                        match state {
                            DiscordNotify::Shutdown => {
                                let _ = client.close();
                                break;
                            }
                            DiscordNotify::Clear => {
                                if let Err(e) = client.clear_activity() {
                                    eprintln!("warn: discord: failed to clear activity: {e}");
                                } else {
                                    force_update = false;
                                }
                            }
                            DiscordNotify::Update {
                                artist,
                                song_name,
                                album,
                                start_time,
                                end_time,
                                paused,
                                cover_id,
                                cover_bytes,
                            } => {
                                if *paused {
                                    if let Err(e) = client.clear_activity() {
                                        eprintln!("warn: discord: failed to clear activity: {e}");
                                    } else {
                                        force_update = false;
                                    }
                                    continue;
                                }

                                let mut image_url = "ratune".to_string(); // Fallback asset key

                                if let Some(id) = cover_id {
                                    if let Some(cached_url) = uguu_cache.get(id) {
                                        image_url = cached_url.clone();
                                        *cover_bytes = None; // Avoid re-uploading
                                    } else if let Some(bytes) = cover_bytes.as_ref() {
                                        if !bytes.is_empty() {
                                            // Upload to uguu.se
                                            let part = multipart::Part::bytes(bytes.clone())
                                                .file_name("cover.jpg")
                                                .mime_str("image/jpeg")
                                                .unwrap_or_else(|_| multipart::Part::bytes(vec![]));
                                            let form = multipart::Form::new().part("files[]", part);

                                            match http_client
                                                .post("https://uguu.se/upload.php")
                                                .multipart(form)
                                                .send()
                                                .await
                                            {
                                                Ok(resp) => {
                                                    let status = resp.status();
                                                    match resp.text().await {
                                                        Ok(text) => match serde_json::from_str::<UguuResponse>(&text) {
                                                            Ok(parsed) if parsed.success && !parsed.files.is_empty() => {
                                                                image_url = parsed.files[0].url.clone();
                                                                uguu_cache.insert(id.clone(), image_url.clone());
                                                                *cover_bytes = None; // Successfully uploaded
                                                            }
                                                            Ok(_) => eprintln!("warn: discord: uguu.se returned success=false. Body: {text}"),
                                                            Err(e) => eprintln!("warn: discord: failed to parse uguu.se response (HTTP {status}): {e}\nBody: {text}"),
                                                        },
                                                        Err(e) => eprintln!("warn: discord: failed to read uguu.se response body: {e}"),
                                                    }
                                                }
                                                Err(e) => {
                                                    eprintln!("warn: discord: uguu.se upload failed: {e}")
                                                }
                                            }
                                        }
                                    }
                                }

                                let mut activity = activity::Activity::new()
                                    .activity_type(activity::ActivityType::Listening)
                                    .name(artist.as_str())
                                    .details(song_name.as_str())
                                    .state(artist.as_str())
                                    .assets(
                                        activity::Assets::new()
                                            .large_image(&image_url)
                                            .large_text(album.as_str()),
                                    );

                                let mut t = Timestamps::new().start(*start_time);
                                if let Some(end) = end_time {
                                    t = t.end(*end);
                                }
                                activity = activity.timestamps(t);

                                if let Err(e) = client.set_activity(activity) {
                                    eprintln!("warn: discord: failed to set activity, retrying in 5s: {}", e);
                                    // Retains force_update = true to try again on timeout
                                } else {
                                    force_update = false;
                                }
                            }
                        }
                    }
                }
            }
        });
    })
}
