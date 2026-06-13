//! Thin typed client over the MediaSage player API (`/api/player/*` + `/api/art`).
//! Uses blocking reqwest with HTTP basic auth on every request. The client is
//! cheap to clone (reqwest::blocking::Client is internally ref-counted) so it is
//! handed to background worker threads freely.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

use crate::config::Config;

#[derive(Clone, Debug, Deserialize, serde::Serialize, Default)]
pub struct Track {
    pub rating_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub ts: Option<f64>,
    #[serde(default)]
    pub plays: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Stats {
    #[serde(default)]
    pub total_plays: i64,
    #[serde(default)]
    pub distinct_tracks: i64,
    #[serde(default)]
    pub since: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Album {
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub artist: String,
    pub parent_rating_key: String,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub thumb: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Artist {
    pub name: String,
    #[serde(default)]
    pub count: i64,
    /// Cover-art URL (`/api/art/{prk}`) — the server started returning this in
    /// the 2026-06 round so the grid can show a real tile instead of a monogram.
    #[serde(default)]
    pub thumb: Option<String>,
}

/// A "your report is fixed" notice from the auto-fix pipeline.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct Notice {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub what_changed: Option<String>,
    /// `"pc"` ⇒ a new native build is out (Update → open the hub download page);
    /// `"android"`/other ⇒ informational on desktop.
    #[serde(default)]
    pub fix_type: Option<String>,
}

/// One of the user's other players (web tab, PWA, phone app, or another native).
#[derive(Clone, Debug, Deserialize, Default)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String, // "web" | "plex"
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
}

/// A queue hand-off delivered to us (via the presence beat or WebSocket).
#[derive(Clone, Debug, Deserialize, Default)]
pub struct HandoffJob {
    #[serde(default)]
    pub rating_keys: Vec<String>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub offset_ms: i64,
    #[serde(default)]
    pub from: String,
}

/// Another device asking us to hand our music over to it.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct PullReq {
    #[serde(default)]
    pub to_device: String,
    #[serde(default)]
    pub to_name: String,
}

/// The presence-beat response: any pending hand-off / pull-request / remote cmds.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct PresenceResp {
    #[serde(default)]
    pub handoff: Option<HandoffJob>,
    #[serde(default)]
    pub pullreq: Option<PullReq>,
    #[serde(default)]
    pub cmds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct PlaylistItem {
    pub rating_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub track_count: i64,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub smart: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct PlGroup {
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub playlists: Vec<PlaylistItem>,
}

/// Streamed events from quick-generate.
pub enum QgEvent {
    Progress(String, String), // step, message
    Narrative(String, String), // title, narrative
    Tracks(Vec<Track>),
    Error(String),
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Player {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub is_mine: bool,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct NowOnPlex {
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub offset_ms: i64,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub player: Option<String>,
    #[serde(default)]
    pub player_id: Option<String>,
    #[serde(default)]
    pub queue: Vec<Track>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Genre {
    pub name: String,
    #[serde(default)]
    pub count: i64,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Decade {
    pub decade: i64,
    #[serde(default)]
    pub albums: i64,
    #[serde(default)]
    pub tracks: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct TrackMeta {
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub format_badge: Option<String>,
    #[serde(default)]
    pub rating: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct LyricLine {
    pub t: f32,
    #[serde(default)]
    pub line: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Lyrics {
    #[serde(default)]
    pub synced: Vec<LyricLine>,
    #[serde(default)]
    pub plain: Option<String>,
    #[serde(default)]
    pub has_synced: bool,
}

// --- response envelopes -----------------------------------------------------
#[derive(Deserialize)]
struct Tracks {
    #[serde(default)]
    tracks: Vec<Track>,
}
#[derive(Deserialize)]
struct Albums {
    #[serde(default)]
    albums: Vec<Album>,
}
#[derive(Deserialize)]
struct Artists {
    #[serde(default)]
    artists: Vec<Artist>,
}
#[derive(Deserialize)]
struct AlbumTracks {
    #[serde(default)]
    album: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    tracks: Vec<Track>,
}
#[derive(Deserialize)]
struct Similar {
    #[serde(default)]
    tracks: Vec<Track>,
}
#[derive(Deserialize)]
struct Genres {
    #[serde(default)]
    genres: Vec<Genre>,
}
#[derive(Deserialize)]
struct Decades {
    #[serde(default)]
    decades: Vec<Decade>,
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::blocking::Client,
    base: String,
    user: String,
    pw: String,
}

impl ApiClient {
    pub fn new(cfg: &Config) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("TulikPlayer/1.0")
            .build()
            .expect("build http client");
        ApiClient {
            http,
            base: cfg.base_url.clone(),
            user: cfg.user.clone(),
            pw: cfg.pw.clone(),
        }
    }

    fn get(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.http
            .get(format!("{}{}", self.base, path))
            .basic_auth(&self.user, Some(&self.pw))
    }

    /// Absolute URL for art / stream (thumbs come back as relative `/api/art/..`).
    pub fn abs(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.base, path)
        }
    }

    // --- library browse ---
    pub fn albums(&self, sort: &str) -> Result<Vec<Album>> {
        let r: Albums = self
            .get(&format!("/api/player/library/albums?sort={sort}&limit=2000"))
            .send()?
            .error_for_status()?
            .json()
            .context("albums json")?;
        Ok(r.albums)
    }

    pub fn artists(&self) -> Result<Vec<Artist>> {
        let r: Artists = self
            .get("/api/player/library/artists?sort=name")
            .send()?
            .error_for_status()?
            .json()
            .context("artists json")?;
        Ok(r.artists)
    }

    pub fn albums_by_artist(&self, artist: &str) -> Result<Vec<Album>> {
        let q = urlencode(artist);
        let r: Albums = self
            .get(&format!("/api/player/library/albums?artist={q}&sort=year"))
            .send()?
            .error_for_status()?
            .json()
            .context("artist albums json")?;
        Ok(r.albums)
    }

    pub fn album_tracks(&self, parent_rating_key: &str) -> Result<(String, String, Vec<Track>)> {
        let r: AlbumTracks = self
            .get(&format!("/api/player/library/album/{parent_rating_key}"))
            .send()?
            .error_for_status()?
            .json()
            .context("album tracks json")?;
        Ok((r.album, r.artist, r.tracks))
    }

    pub fn top(&self) -> Result<Vec<Track>> {
        let r: Tracks = self
            .get("/api/player/library/top?limit=100")
            .send()?
            .error_for_status()?
            .json()
            .context("top json")?;
        Ok(r.tracks)
    }

    pub fn recent(&self) -> Result<Vec<Track>> {
        let r: Tracks = self
            .get("/api/player/recent?limit=60")
            .send()?
            .error_for_status()?
            .json()
            .context("recent json")?;
        Ok(r.tracks)
    }

    pub fn history(&self) -> Result<Vec<Track>> {
        let r: Tracks = self
            .get("/api/player/history?range=all&sort=recent&limit=300")
            .send()?
            .error_for_status()?
            .json()
            .context("history json")?;
        Ok(r.tracks)
    }

    /// Full history browser: text + lyric search, range, sort, plus stats.
    pub fn history_full(&self, q: &str, lyrics: &str, range: &str, sort: &str) -> Result<(Vec<Track>, Stats)> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            tracks: Vec<Track>,
            #[serde(default)]
            stats: Stats,
        }
        let url = format!(
            "/api/player/history{}",
            qs(&[("q", q), ("lyrics", lyrics), ("range", range), ("sort", sort), ("limit", "400")])
        );
        let r: Resp = self.get(&url).send()?.error_for_status()?.json().context("history_full json")?;
        Ok((r.tracks, r.stats))
    }

    pub fn search(&self, q: &str) -> Result<Vec<Track>> {
        let r: Tracks = self
            .get(&format!("/api/player/search?q={}&limit=80", urlencode(q)))
            .send()?
            .error_for_status()?
            .json()
            .context("search json")?;
        Ok(r.tracks)
    }

    pub fn similar(&self, rating_key: &str) -> Result<Vec<Track>> {
        let r: Similar = self
            .get(&format!("/api/player/similar/{rating_key}?n=40"))
            .send()?
            .error_for_status()?
            .json()
            .context("similar json")?;
        Ok(r.tracks)
    }

    // --- filtered library browse (Focus facets + sort + lyric search) ---
    pub fn artists_f(&self, sort: &str, q: &str, genre: &str) -> Result<Vec<Artist>> {
        let url = format!(
            "/api/player/library/artists{}",
            qs(&[("sort", sort), ("q", q), ("genre", genre)])
        );
        let r: Artists = self.get(&url).send()?.error_for_status()?.json()?;
        Ok(r.artists)
    }

    pub fn albums_f(
        &self,
        sort: &str,
        q: &str,
        genre: &str,
        decade: Option<i64>,
        favorites: bool,
        view: &str,
    ) -> Result<Vec<Album>> {
        let dec = decade.map(|d| d.to_string()).unwrap_or_default();
        let url = format!(
            "/api/player/library/albums{}",
            qs(&[
                ("sort", sort),
                ("q", q),
                ("genre", genre),
                ("decade", &dec),
                ("favorites", if favorites { "true" } else { "" }),
                ("view", view),
                ("limit", "2000"),
            ])
        );
        let r: Albums = self.get(&url).send()?.error_for_status()?.json()?;
        Ok(r.albums)
    }

    pub fn tracks_f(
        &self,
        q: &str,
        genre: &str,
        decade: Option<i64>,
        favorites: bool,
        shuffle: bool,
    ) -> Result<Vec<Track>> {
        let dec = decade.map(|d| d.to_string()).unwrap_or_default();
        let url = format!(
            "/api/player/library/tracks{}",
            qs(&[
                ("q", q),
                ("genre", genre),
                ("decade", &dec),
                ("favorites", if favorites { "true" } else { "" }),
                ("shuffle", if shuffle { "true" } else { "" }),
                ("limit", "1200"),
            ])
        );
        let r: Tracks = self.get(&url).send()?.error_for_status()?.json()?;
        Ok(r.tracks)
    }

    pub fn genres(&self) -> Result<Vec<Genre>> {
        let r: Genres = self
            .get("/api/player/library/genres")
            .send()?
            .error_for_status()?
            .json()?;
        Ok(r.genres)
    }

    pub fn decades(&self) -> Result<Vec<Decade>> {
        let r: Decades = self
            .get("/api/player/library/decades")
            .send()?
            .error_for_status()?
            .json()?;
        Ok(r.decades)
    }

    pub fn lyric_search(&self, q: &str) -> Result<Vec<Track>> {
        let r: Tracks = self
            .get(&format!("/api/player/library/lyric-search?q={}&limit=400", urlencode(q)))
            .send()?
            .error_for_status()?
            .json()?;
        Ok(r.tracks)
    }

    pub fn save_queue(&self, title: &str, rks: &[String]) -> Result<()> {
        let body = serde_json::json!({ "title": title, "rating_keys": rks });
        self.http
            .post(format!("{}/api/player/save-queue", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send()?
            .error_for_status()?;
        Ok(())
    }

    // --- playback support ---
    pub fn track_meta(&self, rating_key: &str) -> Result<TrackMeta> {
        let m: TrackMeta = self
            .get(&format!("/api/player/track/{rating_key}"))
            .send()?
            .error_for_status()?
            .json()
            .context("track meta json")?;
        Ok(m)
    }

    pub fn lyrics(&self, rating_key: &str) -> Result<Lyrics> {
        let l: Lyrics = self
            .get(&format!("/api/player/lyrics/{rating_key}"))
            .send()?
            .error_for_status()?
            .json()
            .context("lyrics json")?;
        Ok(l)
    }

    /// Fetch the full encoded audio bytes for a track (mp3/flac/m4a/ogg/…).
    pub fn stream_bytes(&self, rating_key: &str) -> Result<Vec<u8>> {
        let bytes = self
            .get(&format!("/api/player/stream/{rating_key}"))
            .send()?
            .error_for_status()?
            .bytes()
            .context("stream bytes")?;
        Ok(bytes.to_vec())
    }

    /// Fetch raw cover-art bytes for a rating_key (`/api/art/{rk}`).
    pub fn art_bytes(&self, rating_key: &str) -> Result<Vec<u8>> {
        let bytes = self
            .get(&format!("/api/art/{rating_key}"))
            .send()?
            .error_for_status()?
            .bytes()
            .context("art bytes")?;
        Ok(bytes.to_vec())
    }

    /// Best-effort play-history log (mirrors the web player).
    pub fn log_play(&self, t: &Track) {
        let body = serde_json::json!({
            "rk": t.rating_key,
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
        });
        let _ = self
            .http
            .post(format!("{}/api/player/history/log", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    // --- playlists ---
    pub fn playlists(&self) -> Result<Vec<PlGroup>> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            groups: Vec<PlGroup>,
        }
        let r: Resp = self.get("/api/pl/list").send()?.error_for_status()?.json().context("pl list json")?;
        Ok(r.groups)
    }

    pub fn playlist_tracks(&self, rk: &str) -> Result<Vec<Track>> {
        let r: Tracks = self.get(&format!("/api/pl/{rk}/tracks")).send()?.error_for_status()?.json().context("pl tracks json")?;
        Ok(r.tracks)
    }

    // --- quick generate (AI playlist via SSE stream) ---
    pub fn generate_stream<F: FnMut(QgEvent)>(&self, prompt: &str, mut on: F) -> Result<()> {
        use std::io::BufRead;
        let body = serde_json::json!({
            "prompt": prompt, "genres": [], "decades": [], "track_count": 25,
            "exclude_live": true, "min_rating": 0, "max_tracks_to_ai": 500
        });
        let resp = self
            .http
            .post(format!("{}/api/generate/stream", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .timeout(Duration::from_secs(180))
            .json(&body)
            .send()?
            .error_for_status()?;
        let reader = std::io::BufReader::new(resp);
        let mut ev = String::new();
        let mut data = String::new();
        for line in reader.lines() {
            let line = line?;
            if let Some(e) = line.strip_prefix("event: ") {
                ev = e.to_string();
                data.clear();
            } else if let Some(d) = line.strip_prefix("data: ") {
                data.push_str(d);
            } else if line.is_empty() && !ev.is_empty() && !data.is_empty() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                    match ev.as_str() {
                        "progress" => on(QgEvent::Progress(
                            v["step"].as_str().unwrap_or("").to_string(),
                            v["message"].as_str().unwrap_or("").to_string(),
                        )),
                        "narrative" => on(QgEvent::Narrative(
                            v["playlist_title"].as_str().unwrap_or("").to_string(),
                            v["narrative"].as_str().unwrap_or("").to_string(),
                        )),
                        "tracks" => {
                            if let Some(arr) = v["batch"].as_array() {
                                let batch: Vec<Track> = arr
                                    .iter()
                                    .filter_map(|x| {
                                        let rk = match &x["rating_key"] {
                                            serde_json::Value::String(s) => s.clone(),
                                            serde_json::Value::Number(n) => n.to_string(),
                                            _ => return None,
                                        };
                                        Some(Track {
                                            rating_key: rk,
                                            title: x["title"].as_str().unwrap_or("").to_string(),
                                            artist: x["artist"].as_str().unwrap_or("").to_string(),
                                            album: x["album"].as_str().unwrap_or("").to_string(),
                                            thumb: None,
                                            index: None,
                                            ts: None,
                                            plays: None,
                                        })
                                    })
                                    .collect();
                                on(QgEvent::Tracks(batch));
                            }
                        }
                        "error" => on(QgEvent::Error(v["message"].as_str().unwrap_or("generation failed").to_string())),
                        _ => {}
                    }
                }
                ev.clear();
                data.clear();
            }
        }
        Ok(())
    }

    /// Save an AI-generated playlist (best-effort).
    pub fn save_playlist(&self, name: &str, rks: &[String], description: &str) {
        let body = serde_json::json!({ "name": name, "rating_keys": rks, "description": description });
        let _ = self
            .http
            .post(format!("{}/api/playlist", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    // --- cast / hand-off to Plex devices ---
    pub fn players(&self) -> Result<Vec<Player>> {
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            players: Vec<Player>,
        }
        let r: Resp = self.get("/api/pl/players").send()?.error_for_status()?.json().context("players json")?;
        Ok(r.players)
    }

    pub fn now_on_plex(&self) -> Result<NowOnPlex> {
        let r: NowOnPlex = self.get("/api/player/now-on-plex").send()?.error_for_status()?.json().context("now-on-plex json")?;
        Ok(r)
    }

    pub fn cast(&self, rating_keys: &[String], index: usize, offset_ms: i64, player_id: &str) -> Result<()> {
        let body = serde_json::json!({
            "rating_keys": rating_keys,
            "index": index,
            "offset_ms": offset_ms,
            "player_id": player_id,
        });
        self.http
            .post(format!("{}/api/player/cast", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send()?
            .error_for_status()?;
        Ok(())
    }

    /// Remote-control a Plex device: action = pause | play | stop | next | previous.
    pub fn control(&self, player_id: &str, action: &str) -> Result<()> {
        let body = serde_json::json!({ "player_id": player_id, "action": action });
        self.http
            .post(format!("{}/api/pl/control", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send()?
            .error_for_status()?;
        Ok(())
    }

    pub fn stop_plex(&self, player_id: &str) {
        let body = serde_json::json!({ "player_id": player_id });
        let _ = self
            .http
            .post(format!("{}/api/player/stop-plex", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    /// Send feedback (multipart, mirrors the web `/api/feedback`). `kind` is
    /// `"bug"`/`"idea"`/`""`; `personal` is the "only for me" flag. The native
    /// app tags itself `source=windows-app` so feedback triage can tell it apart.
    pub fn send_feedback(&self, text: &str, kind: &str, personal: bool, page: &str, shot: Option<Vec<u8>>) -> Result<()> {
        let mut form = reqwest::blocking::multipart::Form::new()
            .text("text", text.to_string())
            .text("source", "windows-app")
            .text("page", page.to_string())
            .text("kind", kind.to_string())
            .text("personal", if personal { "1" } else { "" }.to_string());
        // optional screenshot — same multipart field name the web uses ("files")
        if let Some(png) = shot {
            let part = reqwest::blocking::multipart::Part::bytes(png)
                .file_name("screenshot.png")
                .mime_str("image/png")?;
            form = form.part("files", part);
        }
        self.http
            .post(format!("{}/api/feedback", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .multipart(form)
            .send()?
            .error_for_status()?;
        Ok(())
    }

    /// Poll the "your report is fixed" notices (best-effort; never errors the UI).
    pub fn notices(&self) -> Vec<Notice> {
        #[derive(Deserialize, Default)]
        struct Resp {
            #[serde(default)]
            notices: Vec<Notice>,
        }
        self.get("/api/player/notices")
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<Resp>().ok())
            .map(|r| r.notices)
            .unwrap_or_default()
    }

    /// Dismiss a notice (best-effort).
    pub fn dismiss_notice(&self, id: &str) {
        let body = serde_json::json!({ "id": id });
        let _ = self
            .http
            .post(format!("{}/api/player/notices/dismiss", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    // --- presence / web-device hand-off (web parity) ------------------------
    /// Send a presence beat; returns any pending hand-off / pull-request / remote
    /// commands. Also registers this device for the ⇄ devices popover and counts
    /// toward the hub usage dashboard. Best-effort (never errors the UI).
    pub fn presence(&self, body: &serde_json::Value) -> Option<PresenceResp> {
        self.http
            .post(format!("{}/api/player/presence", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(body)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<PresenceResp>().ok())
    }

    /// List the user's other live players (web tabs / PWAs / phone / Plex).
    pub fn devices(&self) -> Vec<Device> {
        #[derive(Deserialize, Default)]
        struct Resp {
            #[serde(default)]
            devices: Vec<Device>,
        }
        self.get("/api/player/devices")
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<Resp>().ok())
            .map(|r| r.devices)
            .unwrap_or_default()
    }

    /// Hand our queue + position to another web/app device.
    pub fn handoff(&self, device_id: &str, rks: &[String], index: usize, offset_ms: i64, from_name: &str, tracks: &[Track]) {
        let body = serde_json::json!({
            "device_id": device_id,
            "rating_keys": rks,
            "index": index,
            "offset_ms": offset_ms,
            "from_name": from_name,
            "tracks": tracks,
        });
        let _ = self
            .http
            .post(format!("{}/api/player/handoff", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    /// Ask another device (`target`) to hand its music to us (`to_device`).
    pub fn pull_request(&self, target: &str, to_device: &str, to_name: &str) {
        let body = serde_json::json!({ "device_id": target, "to_device": to_device, "to_name": to_name });
        let _ = self
            .http
            .post(format!("{}/api/player/pull-request", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    /// Remote-control another of our web/app devices (play|pause|next|previous).
    pub fn remote_cmd(&self, device_id: &str, action: &str) {
        let body = serde_json::json!({ "device_id": device_id, "action": action });
        let _ = self
            .http
            .post(format!("{}/api/player/remote-cmd", self.base))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }

    /// Heart / clear a track (Plex 0..10 scale; heart = 10).
    pub fn rate(&self, rating_key: &str, rating: f32) {
        let body = serde_json::json!({ "rating": rating });
        let _ = self
            .http
            .post(format!("{}/api/player/rate/{}", self.base, rating_key))
            .basic_auth(&self.user, Some(&self.pw))
            .json(&body)
            .send();
    }
}

/// Build a `?a=b&c=d` query string, skipping empty values (which are encoded).
fn qs(parts: &[(&str, &str)]) -> String {
    let mut s = String::new();
    for (k, v) in parts {
        if v.is_empty() {
            continue;
        }
        s.push(if s.is_empty() { '?' } else { '&' });
        s.push_str(k);
        s.push('=');
        s.push_str(&urlencode(v));
    }
    s
}

/// Minimal percent-encoding for query values (space + a handful of reserved chars).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
