// Fully native (no-webview) Windows player for MediaSage.
// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod audio;
mod config;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use eframe::egui;
use egui::{Color32, RichText};

use api::{Album, ApiClient, Artist, Decade, Device, Genre, HandoffJob, Lyrics, Notice, NowOnPlex, PlGroup, Player, QgEvent, Stats, Track};
use audio::Cmd;

// --- palette (matches the web "tulik player" CSS :root exactly) ---
// 2026-06-13 repaint: the old lavender/purple set read as "old MediaSage style";
// these are the web player's literal CSS vars (gold accent on neutral near-black).
const BG: Color32 = Color32::from_rgb(14, 14, 16); // web --bg #0e0e10
const CARD: Color32 = Color32::from_rgb(24, 24, 32); // web --tile #181820
const CARD2: Color32 = Color32::from_rgb(31, 31, 41); // web --tile2 #1f1f29
const ACCENT: Color32 = Color32::from_rgb(229, 160, 13); // web --accent #e5a00d (gold)
const GOLD: Color32 = Color32::from_rgb(240, 184, 51); // web --accent2 #f0b833 (bright gold)
const TEXT: Color32 = Color32::from_rgb(244, 244, 246); // web --text #f4f4f6
const MUTED: Color32 = Color32::from_rgb(154, 154, 166); // web --muted #9a9aa6
// dimmer than MUTED — section labels / hairlines (web --faint #5e5e6a)
const FAINT: Color32 = Color32::from_rgb(94, 94, 106);
// add-to-queue "armed" green (web --good #43c19a)
const GOOD: Color32 = Color32::from_rgb(67, 193, 154);

const BUILD: &str = match option_env!("TULIK_BUILD") {
    Some(b) => b,
    None => "dev",
};

// 10-band EQ presets (dB per band: 31 62 125 250 500 1k 2k 4k 8k 16k)
const EQ_PRESETS: &[(&str, [f32; 10])] = &[
    ("Flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Bass", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Vocal", [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("Rock", [5.0, 3.0, 1.0, -1.0, -2.0, 0.0, 2.0, 3.0, 4.0, 4.0]),
    ("Electronic", [5.0, 4.0, 1.0, 0.0, -1.0, 1.0, 0.0, 2.0, 4.0, 5.0]),
    ("Jazz", [3.0, 2.0, 1.0, 2.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0]),
    ("Loud", [4.0, 3.0, 0.0, 0.0, -1.0, 0.0, 1.0, 3.0, 5.0, 4.0]),
];

/// Friends you can share a song to (label, base URL). Mirrors the web's
/// SHARE_USERS — a copied link opens `<base>/player#share=t.<rk>.<sender>` in
/// their player and shows a 💌 card. Injected privately at build time via
/// `TULIK_SHARE_HOSTS` ("Label=https://host:port,Label2=…"); NOTHING is in
/// source — a plain checkout has no friends list.
fn friends() -> Vec<(String, String)> {
    option_env!("TULIK_SHARE_HOSTS")
        .unwrap_or("")
        .split(',')
        .filter_map(|e| {
            let (label, base) = e.split_once('=')?;
            let (label, base) = (label.trim(), base.trim().trim_end_matches('/'));
            if label.is_empty() || base.is_empty() {
                None
            } else {
                Some((label.to_string(), base.to_string()))
            }
        })
        .collect()
}

fn main() -> eframe::Result<()> {
    let cfg = config::load();
    let title = format!("TulikPlayer — {} · build {}", cfg.label, BUILD);
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([860.0, 560.0])
        .with_title(&title);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(&title, options, Box::new(|cc| Ok(Box::new(App::new(cc, cfg)))))
}

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../../app-icon.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

/// The real dog brandmark (same image the web header uses) for the in-app
/// header — replaces the generic 🐶 emoji.
fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!("../../brandmark.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &img.into_raw());
    Some(ctx.load_texture("brandmark", color, egui::TextureOptions::LINEAR))
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Player,
    Library,
    History,
    Playlists,
}

#[derive(Clone)]
struct HistState {
    q: String,
    lyr: String,
    range: String, // today|week|month|year|all
    sort: String,  // recent|most
}

impl Default for HistState {
    fn default() -> Self {
        HistState { q: String::new(), lyr: String::new(), range: "all".into(), sort: "recent".into() }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum LibTarget {
    Artists,
    Albums,
    Songs,
}

enum LibResults {
    Artists(Vec<Artist>),
    Albums(Vec<Album>),
    Songs(Vec<Track>),
}

#[derive(Clone)]
struct LibState {
    target: LibTarget,
    q: String,
    lyr: String,
    genre: Option<String>,
    decade: Option<i64>,
    loved: bool,
    sort: String,
    add_mode: bool,
}

impl Default for LibState {
    fn default() -> Self {
        LibState {
            target: LibTarget::Albums,
            q: String::new(),
            lyr: String::new(),
            genre: None,
            decade: None,
            loved: false,
            sort: "year".into(),
            add_mode: false,
        }
    }
}

enum DataMsg {
    Lib(LibResults),
    Genres(Vec<Genre>),
    Decades(Vec<Decade>),
    History(Vec<Track>),
    Hist(Vec<Track>, Stats),
    AlbumTracks(String, String, Vec<Track>),
    ArtistAlbums(String, Vec<Album>),
    RadioReplace(Vec<Track>),
    RadioPlay(Vec<Track>),
    Meta(String, String, f32),
    LyricsMsg(String, Lyrics),
    Players(Vec<Player>),
    Pulled(Vec<Track>, f32, Option<String>), // queue, start-offset secs, plex player_id to stop
    QgProgress(String, f32),
    QgDone(String, Vec<Track>),
    QgError(String),
    PlGroups(Vec<PlGroup>),
    PlayList(String, Vec<Track>),
    FollowState(NowOnPlex),
    RadioExtend(Vec<Track>),
    PhoneCheck(NowOnPlex),
    /// tracks fetched for a context-menu action: 0 play · 1 play-next · 2 add-to-queue · 3 shuffle
    QueueTracks(u8, String, Vec<Track>),
    Notices(Vec<Notice>),
    Handoff(HandoffJob),
    RemoteCmd(String),
    WebDevices(Vec<Device>),
    Toast(String),
    Error(String),
    Noop,
}

struct App {
    api: ApiClient,
    audio: audio::Handle,
    art: ArtCache,
    ctx: egui::Context,
    logo: Option<egui::TextureHandle>,

    data_tx: Sender<DataMsg>,
    data_rx: Receiver<DataMsg>,

    tab: Tab,
    search_text: String,

    lib: LibState,
    lib_results: Option<LibResults>,
    lib_loading: bool,
    genres: Option<Vec<Genre>>,
    decades: Option<Vec<Decade>>,

    album_view: Option<(String, String, Vec<Track>)>,
    artist_view: Option<(String, Vec<Album>)>,
    history: Option<Vec<Track>>,
    history_loading: bool,

    hist: HistState,
    hist_results: Option<Vec<Track>>,
    hist_stats: Option<Stats>,
    hist_loading: bool,

    np_rk: String,
    np_badge: String,
    np_rating: f32,
    np_lyrics: Option<Lyrics>,
    np_lyric_idx: i64,

    save_open: bool,
    save_title: String,
    toast: String,
    show_help: bool,
    show_fb: bool,
    fb_text: String,
    fb_kind: String,    // "bug" | "idea" | ""  (web parity tag)
    fb_personal: bool,  // "only for me" flag
    fb_shot: Option<Vec<u8>>, // pending screenshot PNG attached to feedback
    fb_shot_dim: (u32, u32),  // attached image size, for the "✓ image NxN" label
    fb_want_shot: bool,       // set when "📸 Capture app" clicked; consumed on the next Screenshot event
    show_eq: bool,
    viz_bars: Vec<f32>,
    players: Vec<Player>,
    cast_confirm: Option<(Player, Vec<String>, usize, i64)>,
    show_qg: bool,
    qg_prompt: String,
    qg_busy: bool,
    qg_phase: String,
    qg_pct: f32,
    pl_groups: Option<Vec<PlGroup>>,
    pl_loading: bool,
    base_url: String,
    // follow / remote-control Plexamp
    follow_on: bool,
    follow_pid: Option<String>,
    follow_player: String,
    follow_track: Option<Track>,
    follow_offset_ms: f64,
    follow_dur_ms: f64,
    follow_playing: bool,
    follow_base: Option<std::time::Instant>,
    follow_last_poll: Option<std::time::Instant>,
    pre_mute_vol: Option<f32>,
    bal_presets: [f32; 3], // 50/50, A, B  (-1 left .. +1 right)
    bal_active: usize,
    seek_drag: Option<f32>,
    err: String,
    toast_at: Option<std::time::Instant>,
    last_toast: String,
    last_save: std::time::Instant,
    show_viz: bool,
    show_full_viz: bool,
    /// A–Z rail click → item index to scroll to on the next frame
    az_jump: Option<usize>,
    /// current library results came from a quick pick (not A–Z sorted)
    lib_is_quick: bool,
    /// sonic radio is active → auto-extend the queue near its end (web parity)
    radio_on: bool,
    radio_fetching: bool,
    /// the now-playing radio CTA is building
    radio_busy: bool,
    /// the ⊞ corner overlay with tulik pages/tools links
    show_links: bool,
    /// "your report is fixed" notices (polled best-effort)
    notices: Vec<Notice>,
    notice_last_check: Option<std::time::Instant>,
    /// stable per-install id for presence / hand-off; other live web/app devices
    device_id: String,
    device_name: String,
    web_devices: Vec<Device>,
    /// Gil's build only: the dancing puppy replaces the (removed) visualizer
    is_gil: bool,
    /// friendly user label, used as the "from" in share links
    user_label: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cfg: config::Config) -> Self {
        let ctx = cc.egui_ctx.clone();
        setup_theme(&ctx);
        let logo = load_logo(&ctx);
        let base_url = cfg.base_url.clone();
        let dev_id = device_id();
        let dev_name = format!("TulikPlayer (PC) — {}", cfg.label);
        let is_gil = cfg.user == "gil";
        let user_label = cfg.label.clone();
        let api = ApiClient::new(&cfg);
        let audio = audio::spawn(api.clone(), ctx.clone());
        // restore the previous session (queue + position), paused
        if let Some(sess) = load_session() {
            if !sess.tracks.is_empty() {
                let _ = audio.tx.send(Cmd::Restore {
                    tracks: sess.tracks,
                    index: sess.index,
                    pos: sess.position,
                });
            }
            if sess.volume > 0.0 {
                let _ = audio.tx.send(Cmd::SetVolume(sess.volume));
            }
        }
        let art = ArtCache::new(api.clone(), ctx.clone());
        let (data_tx, data_rx) = channel();
        // preload cast devices in the background
        {
            let api = api.clone();
            let tx = data_tx.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                if let Ok(p) = api.players() {
                    let _ = tx.send(DataMsg::Players(p));
                    ctx.request_repaint();
                }
            });
        }
        // one-shot phone check shortly after launch (web parity): if Plexamp is
        // already playing and nothing is queued here, follow it automatically
        {
            let api = api.clone();
            let tx = data_tx.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                thread::sleep(std::time::Duration::from_millis(1500));
                if let Ok(d) = api.now_on_plex() {
                    let _ = tx.send(DataMsg::PhoneCheck(d));
                    ctx.request_repaint();
                }
            });
        }
        // presence beat (web parity): every ~20s announce ourselves so we appear
        // in other devices' ⇄ popover + count in the usage dashboard, and pick up
        // any pending hand-off / remote-command / pull-request. Self-contained:
        // pull-requests are answered straight from the shared queue snapshot.
        {
            let api = api.clone();
            let tx = data_tx.clone();
            let ctx = ctx.clone();
            let shared = audio.shared.clone();
            let id = dev_id.clone();
            let name = dev_name.clone();
            thread::spawn(move || loop {
                let snap = shared.lock().map(|s| s.clone()).unwrap_or_default();
                let cur = snap.current.clone();
                let body = serde_json::json!({
                    "device_id": id,
                    "name": name,
                    "source": "windows-app",
                    "playing": snap.playing,
                    "title": cur.as_ref().map(|t| t.title.clone()).unwrap_or_default(),
                    "artist": cur.as_ref().map(|t| t.artist.clone()).unwrap_or_default(),
                    "album": cur.as_ref().map(|t| t.album.clone()).unwrap_or_default(),
                    "rating_key": cur.as_ref().map(|t| t.rating_key.clone()).unwrap_or_default(),
                    "thumb": cur.as_ref().and_then(|t| t.thumb.clone()).unwrap_or_default(),
                    "offset_ms": (snap.position * 1000.0) as i64,
                    "duration_ms": (snap.duration * 1000.0) as i64,
                    "state": if snap.playing { "playing" } else { "paused" },
                    "interval_s": 20,
                });
                if let Some(resp) = api.presence(&body) {
                    if let Some(h) = resp.handoff {
                        if !h.tracks.is_empty() || !h.rating_keys.is_empty() {
                            let _ = tx.send(DataMsg::Handoff(h));
                        }
                    }
                    for c in resp.cmds {
                        let _ = tx.send(DataMsg::RemoteCmd(c));
                    }
                    if let Some(pr) = resp.pullreq {
                        let rks: Vec<String> = snap.queue.iter().map(|t| t.rating_key.clone()).collect();
                        if !rks.is_empty() {
                            api.handoff(&pr.to_device, &rks, snap.index, (snap.position * 1000.0) as i64, &name, &snap.queue);
                        }
                    }
                    ctx.request_repaint();
                }
                // refresh the devices popover list off the same beat
                let _ = tx.send(DataMsg::WebDevices(api.devices()));
                ctx.request_repaint();
                thread::sleep(std::time::Duration::from_secs(20));
            });
        }
        App {
            api,
            audio,
            art,
            ctx,
            logo,
            data_tx,
            data_rx,
            tab: Tab::Player,
            search_text: String::new(),
            lib: LibState::default(),
            lib_results: None,
            lib_loading: false,
            genres: None,
            decades: None,
            album_view: None,
            artist_view: None,
            history: None,
            history_loading: false,
            hist: HistState::default(),
            hist_results: None,
            hist_stats: None,
            hist_loading: false,
            np_rk: String::new(),
            np_badge: String::new(),
            np_rating: 0.0,
            np_lyrics: None,
            np_lyric_idx: -1,
            save_open: false,
            save_title: String::new(),
            toast: String::new(),
            show_help: false,
            show_fb: false,
            fb_text: String::new(),
            fb_kind: String::new(),
            fb_personal: false,
            fb_shot: None,
            fb_shot_dim: (0, 0),
            fb_want_shot: false,
            show_eq: false,
            viz_bars: vec![0.0; 28],
            players: Vec::new(),
            cast_confirm: None,
            show_qg: false,
            qg_prompt: String::new(),
            qg_busy: false,
            qg_phase: String::new(),
            qg_pct: 0.0,
            pl_groups: None,
            pl_loading: false,
            base_url,
            follow_on: false,
            follow_pid: None,
            follow_player: String::new(),
            follow_track: None,
            follow_offset_ms: 0.0,
            follow_dur_ms: 0.0,
            follow_playing: false,
            follow_base: None,
            follow_last_poll: None,
            pre_mute_vol: None,
            bal_presets: [0.0, -0.6, 0.6],
            bal_active: 0,
            seek_drag: None,
            err: String::new(),
            toast_at: None,
            last_toast: String::new(),
            last_save: std::time::Instant::now(),
            show_viz: true,
            show_full_viz: false,
            az_jump: None,
            lib_is_quick: false,
            radio_on: false,
            radio_fetching: false,
            radio_busy: false,
            show_links: false,
            notices: Vec::new(),
            notice_last_check: None,
            device_id: dev_id,
            device_name: dev_name,
            web_devices: Vec::new(),
            is_gil,
            user_label,
        }
    }

    fn set_balance(&self, b: f32) {
        if let Ok(mut p) = self.audio.dsp.params.lock() {
            p.balance = b;
        }
    }

    fn enter_follow(&mut self) {
        self.follow_on = true;
        self.follow_last_poll = None; // poll immediately
        self.play(Cmd::Pause);
        self.toast = "📱 Following Plexamp".into();
    }

    fn exit_follow(&mut self) {
        self.follow_on = false;
        self.follow_track = None;
        self.follow_player.clear();
        self.toast = "Stopped following".into();
    }

    /// Remote transport while following the phone.
    fn follow_control(&mut self, action: &str) {
        let pid = match &self.follow_pid {
            Some(p) => p.clone(),
            None => {
                self.toast = "No phone session yet".into();
                return;
            }
        };
        if action == "play" {
            self.follow_playing = true;
            self.follow_base = Some(std::time::Instant::now());
        } else if action == "pause" {
            self.follow_offset_ms = self.follow_pos_ms();
            self.follow_playing = false;
        }
        let api = self.api.clone();
        let act = action.to_string();
        thread::spawn(move || {
            let _ = api.control(&pid, &act);
        });
        self.follow_last_poll = None; // resync soon
    }

    fn follow_pos_ms(&self) -> f64 {
        let mut pos = self.follow_offset_ms;
        if self.follow_playing {
            if let Some(b) = self.follow_base {
                pos += b.elapsed().as_secs_f64() * 1000.0;
            }
        }
        if self.follow_dur_ms > 0.0 {
            pos = pos.min(self.follow_dur_ms);
        }
        pos
    }

    fn start_qg(&mut self, prompt: String) {
        if self.qg_busy || prompt.trim().is_empty() {
            return;
        }
        self.qg_busy = true;
        self.qg_phase = "Starting…".into();
        self.qg_pct = 5.0;
        let api = self.api.clone();
        let tx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let mut tracks: Vec<Track> = Vec::new();
            let mut title = String::new();
            let mut narr = String::new();
            let res = api.generate_stream(&prompt, |ev| match ev {
                QgEvent::Progress(step, msg) => {
                    let _ = tx.send(DataMsg::QgProgress(msg, step_pct(&step)));
                    ctx.request_repaint();
                }
                QgEvent::Narrative(t, n) => {
                    if !t.is_empty() {
                        title = t;
                    }
                    narr = n;
                }
                QgEvent::Tracks(b) => tracks.extend(b),
                QgEvent::Error(e) => {
                    let _ = tx.send(DataMsg::QgError(e));
                }
            });
            match res {
                Ok(()) => {
                    let q: Vec<Track> = tracks.into_iter().filter(|t| !t.rating_key.is_empty()).collect();
                    if q.is_empty() {
                        let _ = tx.send(DataMsg::QgError("No matching tracks — try rephrasing".into()));
                    } else {
                        let name = if title.is_empty() { prompt.clone() } else { title.clone() };
                        let rks: Vec<String> = q.iter().map(|t| t.rating_key.clone()).collect();
                        let desc = if narr.is_empty() { format!("Quick generate: {prompt}") } else { narr.clone() };
                        let api2 = api.clone();
                        let nm = name.clone();
                        thread::spawn(move || api2.save_playlist(&nm, &rks, &desc));
                        let _ = tx.send(DataMsg::QgDone(name, q));
                    }
                }
                Err(e) => {
                    let _ = tx.send(DataMsg::QgError(e.to_string()));
                }
            }
            ctx.request_repaint();
        });
    }

    fn play(&mut self, cmd: Cmd) {
        // radio lifecycle (web parity): a manual queue replace/clear ends
        // radio mode; Cmd::RadioReplace (the now-playing CTA) starts it.
        match &cmd {
            Cmd::SetQueue { .. } | Cmd::Clear => self.radio_on = false,
            Cmd::RadioReplace(_) => self.radio_on = true,
            _ => {}
        }
        let _ = self.audio.tx.send(cmd);
    }

    fn fetch<F>(&self, f: F)
    where
        F: FnOnce(&ApiClient) -> DataMsg + Send + 'static,
    {
        let api = self.api.clone();
        let tx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let msg = f(&api);
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    /// (Re)load the library browse body from the current facets/target.
    fn reload_library(&mut self) {
        self.lib_loading = true;
        self.lib_is_quick = false;
        self.az_jump = None;
        let st = self.lib.clone();
        // lyric search overrides the normal browse
        if !st.lyr.trim().is_empty() {
            let q = st.lyr.clone();
            self.fetch(move |a| match a.lyric_search(&q) {
                Ok(v) => DataMsg::Lib(LibResults::Songs(v)),
                Err(e) => DataMsg::Error(e.to_string()),
            });
            return;
        }
        let genre = st.genre.clone().unwrap_or_default();
        match st.target {
            LibTarget::Artists => {
                let (q, g, sort) = (st.q.clone(), genre, st.sort.clone());
                self.fetch(move |a| match a.artists_f(&sort, &q, &g) {
                    Ok(v) => DataMsg::Lib(LibResults::Artists(v)),
                    Err(e) => DataMsg::Error(e.to_string()),
                });
            }
            LibTarget::Albums => {
                let (q, g, sort) = (st.q.clone(), genre, st.sort.clone());
                let (dec, loved) = (st.decade, st.loved);
                self.fetch(move |a| match a.albums_f(&sort, &q, &g, dec, loved, "") {
                    Ok(v) => DataMsg::Lib(LibResults::Albums(v)),
                    Err(e) => DataMsg::Error(e.to_string()),
                });
            }
            LibTarget::Songs => {
                let (q, g) = (st.q.clone(), genre);
                let (dec, loved) = (st.decade, st.loved);
                self.fetch(move |a| match a.tracks_f(&q, &g, dec, loved, false) {
                    Ok(v) => DataMsg::Lib(LibResults::Songs(v)),
                    Err(e) => DataMsg::Error(e.to_string()),
                });
            }
        }
    }

    fn reload_history(&mut self) {
        self.hist_loading = true;
        let h = self.hist.clone();
        self.fetch(move |a| match a.history_full(&h.q, &h.lyr, &h.range, &h.sort) {
            Ok((v, s)) => DataMsg::Hist(v, s),
            Err(e) => DataMsg::Error(e.to_string()),
        });
    }

    fn ensure_loaded(&mut self) {
        if self.history.is_none() && !self.history_loading {
            self.history_loading = true;
            self.fetch(|a| match a.history() {
                Ok(v) => DataMsg::History(v),
                Err(e) => DataMsg::Error(e.to_string()),
            });
        }
        if self.tab == Tab::History {
            if self.hist_results.is_none() && !self.hist_loading {
                self.reload_history();
            }
            return;
        }
        if self.tab == Tab::Playlists {
            if self.pl_groups.is_none() && !self.pl_loading {
                self.pl_loading = true;
                self.fetch(|a| match a.playlists() {
                    Ok(g) => DataMsg::PlGroups(g),
                    Err(e) => DataMsg::Error(e.to_string()),
                });
            }
            return;
        }
        if self.tab != Tab::Library {
            return;
        }
        if self.genres.is_none() {
            self.genres = Some(Vec::new()); // mark requested
            self.fetch(|a| match a.genres() {
                Ok(v) => DataMsg::Genres(v),
                Err(e) => DataMsg::Error(e.to_string()),
            });
        }
        if self.decades.is_none() {
            self.decades = Some(Vec::new());
            self.fetch(|a| match a.decades() {
                Ok(v) => DataMsg::Decades(v),
                Err(e) => DataMsg::Error(e.to_string()),
            });
        }
        if self.lib_results.is_none() && !self.lib_loading && self.album_view.is_none() && self.artist_view.is_none() {
            self.reload_library();
        }
    }

    fn refresh_now_playing(&mut self, current: &Option<Track>) {
        let rk = match current {
            Some(t) => t.rating_key.clone(),
            None => return,
        };
        if rk == self.np_rk {
            return;
        }
        self.np_rk = rk.clone();
        self.np_badge.clear();
        self.np_rating = 0.0;
        self.np_lyrics = None;
        self.np_lyric_idx = -1;

        let rk2 = rk.clone();
        self.fetch(move |a| match a.track_meta(&rk2) {
            Ok(m) => DataMsg::Meta(rk2.clone(), m.format_badge.unwrap_or_default(), m.rating.unwrap_or(0.0)),
            Err(_) => DataMsg::Noop, // best-effort; don't surface
        });
        self.fetch(move |a| match a.lyrics(&rk) {
            Ok(l) => DataMsg::LyricsMsg(rk.clone(), l),
            Err(_) => DataMsg::Noop, // lyrics are best-effort; never error the UI
        });
    }

    fn drain_data(&mut self) {
        while let Ok(msg) = self.data_rx.try_recv() {
            match msg {
                DataMsg::Lib(r) => {
                    self.lib_results = Some(r);
                    self.lib_loading = false;
                }
                DataMsg::Genres(v) => self.genres = Some(v),
                DataMsg::Decades(v) => self.decades = Some(v),
                DataMsg::History(v) => {
                    self.history = Some(v);
                    self.history_loading = false;
                }
                DataMsg::Hist(v, s) => {
                    self.hist_results = Some(v);
                    self.hist_stats = Some(s);
                    self.hist_loading = false;
                }
                DataMsg::AlbumTracks(album, artist, tracks) => self.album_view = Some((album, artist, tracks)),
                DataMsg::ArtistAlbums(name, albums) => self.artist_view = Some((name, albums)),
                DataMsg::RadioReplace(v) => {
                    self.radio_busy = false;
                    if v.is_empty() {
                        self.toast = "No sonic match for this track".into();
                    } else {
                        let n = v.len();
                        self.play(Cmd::RadioReplace(v));
                        self.toast = format!("☄ Radio: {n} similar tracks");
                    }
                }
                DataMsg::RadioPlay(q) => {
                    self.radio_busy = false;
                    if q.len() <= 1 {
                        self.toast = "No sonic match for this track".into();
                    } else {
                        let n = q.len() - 1;
                        self.play(Cmd::SetQueue { tracks: q, start: 0 });
                        self.radio_on = true; // seeded radio keeps extending too
                        self.tab = Tab::Player;
                        self.toast = format!("☄ Radio: {n} similar tracks");
                    }
                }
                DataMsg::Meta(rk, badge, rating) => {
                    if rk == self.np_rk {
                        self.np_badge = badge;
                        self.np_rating = rating;
                    }
                }
                DataMsg::LyricsMsg(rk, l) => {
                    if rk == self.np_rk {
                        self.np_lyrics = Some(l);
                    }
                }
                DataMsg::Players(v) => self.players = v,
                DataMsg::Pulled(q, off, pid) => {
                    if q.is_empty() {
                        self.toast = "Nothing playing on Plex".into();
                    } else {
                        self.play(Cmd::SetQueue { tracks: q, start: 0 });
                        if off > 0.5 {
                            self.play(Cmd::Seek(off));
                        }
                        if let Some(pid) = pid {
                            let api = self.api.clone();
                            thread::spawn(move || api.stop_plex(&pid));
                        }
                        self.tab = Tab::Player;
                        self.toast = "Pulled from Plex ✓".into();
                    }
                }
                DataMsg::QgProgress(msg, pct) => {
                    if !msg.is_empty() {
                        self.qg_phase = msg;
                    }
                    self.qg_pct = self.qg_pct.max(pct);
                }
                DataMsg::QgDone(title, q) => {
                    self.qg_busy = false;
                    self.qg_pct = 100.0;
                    let n = q.len();
                    self.play(Cmd::SetQueue { tracks: q, start: 0 });
                    self.tab = Tab::Player;
                    self.toast = format!("▶ {title} · {n} tracks");
                }
                DataMsg::QgError(e) => {
                    self.qg_busy = false;
                    self.err = format!("⚡ {e}");
                }
                DataMsg::PlGroups(g) => {
                    self.pl_groups = Some(g);
                    self.pl_loading = false;
                }
                DataMsg::PlayList(title, q) => {
                    if q.is_empty() {
                        self.toast = "No playable tracks".into();
                    } else {
                        let n = q.len();
                        self.play(Cmd::SetQueue { tracks: q, start: 0 });
                        self.tab = Tab::Player;
                        self.toast = format!("▶ {title} · {n} tracks");
                    }
                }
                DataMsg::FollowState(n) => {
                    if self.follow_on {
                        if n.playing {
                            self.follow_track = n.queue.first().cloned();
                            if n.player_id.is_some() {
                                self.follow_pid = n.player_id.clone();
                            }
                            self.follow_player = n.player.clone().unwrap_or_default();
                            self.follow_offset_ms = n.offset_ms as f64;
                            self.follow_dur_ms = n.duration_ms as f64;
                            self.follow_playing = n.state.as_deref() != Some("paused");
                            self.follow_base = Some(std::time::Instant::now());
                        } else {
                            self.follow_track = None;
                            self.follow_player = "idle".into();
                        }
                    }
                }
                DataMsg::RadioExtend(v) => {
                    self.radio_fetching = false;
                    if v.is_empty() {
                        // out of sonic matches — let the radio end naturally
                        self.radio_on = false;
                    } else if self.radio_on {
                        self.play(Cmd::EnqueueEnd(v));
                    }
                }
                DataMsg::PhoneCheck(d) => {
                    if d.playing && !self.follow_on {
                        let idle = self.audio.shared.lock().map(|s| s.queue.is_empty()).unwrap_or(false);
                        if idle {
                            // nothing queued here — follow the phone automatically (web parity)
                            self.enter_follow();
                        } else {
                            let who = d.player.clone().unwrap_or_else(|| "Plexamp".into());
                            self.toast = format!("📱 {who} is playing on your phone — 📡 → Follow phone to control it");
                        }
                    }
                }
                DataMsg::QueueTracks(act, title, tracks) => {
                    if tracks.is_empty() {
                        self.toast = "No tracks found".into();
                    } else {
                        let n = tracks.len();
                        match act {
                            0 => {
                                self.play(Cmd::SetQueue { tracks, start: 0 });
                                self.tab = Tab::Player;
                                self.toast = format!("▶ {title} ({n})");
                            }
                            3 => {
                                self.play(Cmd::SetQueue { tracks, start: 0 });
                                self.play(Cmd::Shuffle);
                                self.tab = Tab::Player;
                                self.toast = format!("🔀 {title} ({n})");
                            }
                            1 => {
                                for t in tracks.into_iter().rev() {
                                    self.play(Cmd::PlayNext(t));
                                }
                                self.toast = format!("⏭ Playing next: {title} ({n})");
                            }
                            _ => {
                                self.play(Cmd::EnqueueEnd(tracks));
                                self.toast = format!("＋ Queued {title} ({n})");
                            }
                        }
                    }
                }
                DataMsg::Notices(v) => self.notices = v,
                DataMsg::WebDevices(v) => self.web_devices = v,
                DataMsg::Handoff(h) => {
                    let mut tracks = h.tracks;
                    if tracks.is_empty() {
                        tracks = h.rating_keys.iter().map(|rk| Track { rating_key: rk.clone(), ..Default::default() }).collect();
                    }
                    if !tracks.is_empty() {
                        let start = h.index.min(tracks.len() - 1);
                        if self.follow_on {
                            self.exit_follow();
                        }
                        self.play(Cmd::SetQueue { tracks, start });
                        if h.offset_ms > 500 {
                            self.play(Cmd::Seek(h.offset_ms as f32 / 1000.0));
                        }
                        self.tab = Tab::Player;
                        let from = if h.from.is_empty() { "another device".to_string() } else { h.from.clone() };
                        self.toast = format!("⥁ Handed off from {from}");
                    }
                }
                DataMsg::RemoteCmd(a) => {
                    let playing = self.audio.shared.lock().map(|s| s.playing).unwrap_or(false);
                    match a.as_str() {
                        "play" => {
                            if !playing {
                                self.play(Cmd::Toggle);
                            }
                        }
                        "pause" => {
                            if playing {
                                self.play(Cmd::Pause);
                            }
                        }
                        "next" => self.play(Cmd::Next),
                        "previous" => self.play(Cmd::Prev),
                        _ => {}
                    }
                }
                DataMsg::Toast(t) => self.toast = t,
                DataMsg::Error(e) => {
                    self.radio_busy = false;
                    self.err = truncate(&e, 80);
                }
                DataMsg::Noop => {}
            }
        }
    }

    /// Send our queue + position to another of the user's web/app devices.
    fn do_handoff(&mut self, device: &Device, snap: &audio::Shared) {
        let rks: Vec<String> = snap.queue.iter().map(|t| t.rating_key.clone()).collect();
        if rks.is_empty() {
            self.toast = "Queue is empty".into();
            return;
        }
        let (api, id, from) = (self.api.clone(), device.id.clone(), self.device_name.clone());
        let tracks = snap.queue.clone();
        let (index, offset_ms) = (snap.index, (snap.position * 1000.0) as i64);
        thread::spawn(move || api.handoff(&id, &rks, index, offset_ms, &from, &tracks));
        self.toast = format!("▶ Sent to {}", device.name);
    }

    /// Ask another device to hand what it's playing over to us.
    fn do_pull_device(&mut self, device: &Device) {
        let (api, target, me, myname) = (self.api.clone(), device.id.clone(), self.device_id.clone(), self.device_name.clone());
        thread::spawn(move || api.pull_request(&target, &me, &myname));
        self.toast = format!("⥁ Pulling from {}…", device.name);
    }

    fn load_players(&self) {
        let api = self.api.clone();
        let tx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            if let Ok(p) = api.players() {
                let _ = tx.send(DataMsg::Players(p));
                ctx.request_repaint();
            }
        });
    }

    fn pull_from_plex(&self) {
        let api = self.api.clone();
        let tx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let msg = match api.now_on_plex() {
                Ok(n) if n.playing => DataMsg::Pulled(n.queue, n.offset_ms as f32 / 1000.0, n.player_id),
                Ok(_) => DataMsg::Pulled(vec![], 0.0, None),
                Err(e) => DataMsg::Error(e.to_string()),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn do_cast(&self, player_id: String, rks: Vec<String>, index: usize, offset_ms: i64, name: String) {
        let api = self.api.clone();
        let tx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let m = match api.cast(&rks, index, offset_ms, &player_id) {
                Ok(()) => DataMsg::Toast(format!("Casting to {name} ✓")),
                Err(e) => DataMsg::Error(e.to_string()),
            };
            let _ = tx.send(m);
            ctx.request_repaint();
        });
    }

    /// Poll the "your report is fixed" notices on startup, then every 3 min.
    fn poll_notices(&mut self) {
        let due = self.notice_last_check.map(|t| t.elapsed().as_secs() >= 180).unwrap_or(true);
        if !due {
            return;
        }
        self.notice_last_check = Some(std::time::Instant::now());
        let api = self.api.clone();
        let tx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let n = api.notices();
            let _ = tx.send(DataMsg::Notices(n));
            ctx.request_repaint();
        });
    }

    /// Green "✓ Fixed — you reported …" banner with Update + Dismiss (web parity).
    /// Native can't self-update, so Update opens the hub download page in a browser.
    fn notices_banner(&mut self, ctx: &egui::Context) {
        if self.notices.is_empty() {
            return;
        }
        let n = self.notices[0].clone();
        let more = self.notices.len().saturating_sub(1);
        let is_pc = n.fix_type.as_deref() == Some("pc");
        let mut dismiss = false;
        egui::TopBottomPanel::top("notices")
            .frame(egui::Frame::none().fill(Color32::from_rgb(18, 40, 28)).inner_margin(egui::Margin::symmetric(16.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let head = if more > 0 { format!("✓ Fixed  +{more} more") } else { "✓ Fixed".into() };
                        ui.label(RichText::new(head).strong().color(GOOD));
                        ui.label(RichText::new(format!("You reported: “{}”", truncate(&n.title, 70))).size(12.5).color(TEXT));
                        if let Some(w) = n.what_changed.as_ref().filter(|w| !w.is_empty()) {
                            ui.label(RichText::new(truncate(w, 90)).size(11.5).color(MUTED));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if hand(ui.button("Dismiss")).clicked() {
                            dismiss = true;
                        }
                        if is_pc {
                            // a fresh native build is out — point Gil at the hub download
                            ui.hyperlink_to(RichText::new("⬆ Update").strong().color(GOOD), format!("{}/hub/", self.base_url))
                                .on_hover_text("Opens your hub — re-download TulikPlayerNative.exe");
                        }
                    });
                });
            });
        if dismiss {
            let id = n.id.clone();
            let api = self.api.clone();
            thread::spawn(move || api.dismiss_notice(&id));
            self.notices.remove(0);
        }
    }

    fn open_album(&mut self, prk: String) {
        self.tab = Tab::Library;
        self.album_view = None;
        self.artist_view = None;
        self.fetch(move |a| match a.album_tracks(&prk) {
            Ok((al, ar, t)) => DataMsg::AlbumTracks(al, ar, t),
            Err(e) => DataMsg::Error(e.to_string()),
        });
    }

    /// Resolve a track's album by artist + name match (the web's gotoAlbum
    /// trick — track payloads carry no parent key) and open its detail view.
    fn goto_album(&mut self, t: &Track) {
        if t.album.is_empty() {
            // no album metadata — fall back to the artist page, like the web
            self.open_artist(t.artist.clone());
            return;
        }
        self.tab = Tab::Library;
        self.album_view = None;
        self.artist_view = None;
        let (artist, album) = (t.artist.clone(), t.album.clone());
        self.fetch(move |a| {
            let want = album.to_lowercase();
            let found = a.albums_by_artist(&artist).ok().and_then(|v| {
                v.iter()
                    .find(|x| x.album.to_lowercase() == want)
                    .or_else(|| v.iter().find(|x| x.album.to_lowercase().contains(&want)))
                    .map(|x| x.parent_rating_key.clone())
            });
            match found {
                Some(prk) => match a.album_tracks(&prk) {
                    Ok((al, ar, tr)) => DataMsg::AlbumTracks(al, ar, tr),
                    Err(e) => DataMsg::Error(e.to_string()),
                },
                None => DataMsg::Toast("Couldn't find that album in the library".into()),
            }
        });
    }

    fn open_artist(&mut self, name: String) {
        self.tab = Tab::Library;
        self.album_view = None;
        self.artist_view = None;
        let nm = name.clone();
        self.fetch(move |a| match a.albums_by_artist(&nm) {
            Ok(v) => DataMsg::ArtistAlbums(name, v),
            Err(e) => DataMsg::Error(e.to_string()),
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.art.poll(ctx);
        self.drain_data();

        // Feedback screenshot: a "📸 Capture app" click asked egui for the framebuffer
        // last frame; the rendered image arrives here as a Screenshot event. Encode it
        // to PNG and stage it as the feedback attachment.
        if self.fb_want_shot {
            let shot = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                self.fb_want_shot = false;
                if let Some(png) = colorimage_to_png(&img) {
                    self.fb_shot_dim = (img.size[0] as u32, img.size[1] as u32);
                    self.fb_shot = Some(png);
                    self.toast = "📸 Screenshot attached".into();
                }
            }
        }

        let snap = self.audio.shared.lock().unwrap().clone();
        self.refresh_now_playing(&snap.current);
        self.ensure_loaded();
        self.poll_notices();

        // radio auto-extend (web parity): keep ~3 tracks of runway by fetching
        // more sonically-similar tracks seeded from the end of the queue
        if self.radio_on && !self.radio_fetching && !snap.queue.is_empty() && snap.index + 3 >= snap.queue.len() {
            if let Some(last) = snap.queue.last() {
                self.radio_fetching = true;
                let seed = last.rating_key.clone();
                let have: std::collections::HashSet<String> = snap.queue.iter().map(|t| t.rating_key.clone()).collect();
                self.fetch(move |a| match a.similar(&seed) {
                    Ok(v) => DataMsg::RadioExtend(v.into_iter().filter(|t| !have.contains(&t.rating_key)).collect()),
                    Err(_) => DataMsg::RadioExtend(Vec::new()),
                });
            }
        }

        // keyboard shortcuts (mirrors the web player)
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if esc {
            self.show_help = false;
            self.show_fb = false;
            self.show_eq = false;
            self.show_qg = false;
            self.show_full_viz = false;
            self.show_links = false;
            self.cast_confirm = None;
        }
        if !ctx.wants_keyboard_input() {
            let (space, k_next, k_prev, k_fwd, k_back, k_mute, k_viz) = ctx.input(|i| {
                let shift = i.modifiers.shift;
                (
                    i.key_pressed(egui::Key::Space),
                    i.key_pressed(egui::Key::ArrowRight) && shift,
                    i.key_pressed(egui::Key::ArrowLeft) && shift,
                    i.key_pressed(egui::Key::ArrowRight) && !shift,
                    i.key_pressed(egui::Key::ArrowLeft) && !shift,
                    i.key_pressed(egui::Key::M),
                    i.key_pressed(egui::Key::V),
                )
            });
            if space {
                if self.follow_on {
                    self.follow_control(if self.follow_playing { "pause" } else { "play" });
                } else {
                    self.play(Cmd::Toggle);
                }
            }
            if k_next {
                if self.follow_on {
                    self.follow_control("next");
                } else {
                    self.play(Cmd::Next);
                }
            }
            if k_prev {
                if self.follow_on {
                    self.follow_control("previous");
                } else {
                    self.play(Cmd::Prev);
                }
            }
            if k_fwd && !self.follow_on {
                self.play(Cmd::Seek((snap.position + 5.0).min(snap.duration)));
            }
            if k_back && !self.follow_on {
                self.play(Cmd::Seek((snap.position - 5.0).max(0.0)));
            }
            if k_mute {
                if let Some(v) = self.pre_mute_vol.take() {
                    self.play(Cmd::SetVolume(v));
                } else {
                    self.pre_mute_vol = Some(snap.volume);
                    self.play(Cmd::SetVolume(0.0));
                }
            }
            if k_viz && self.is_gil {
                self.show_full_viz = !self.show_full_viz;
            }
        }

        // animate the visualizer smoothly while playing — and while the
        // fullscreen visualizer is open, so bars keep moving/decaying
        if snap.playing || self.show_full_viz {
            ctx.request_repaint();
        }

        // poll Plexamp while following
        if self.follow_on {
            let need = self.follow_last_poll.map(|t| t.elapsed().as_secs_f32() > 3.0).unwrap_or(true);
            if need {
                self.follow_last_poll = Some(std::time::Instant::now());
                let api = self.api.clone();
                let tx = self.data_tx.clone();
                let ctx2 = self.ctx.clone();
                thread::spawn(move || {
                    if let Ok(n) = api.now_on_plex() {
                        let _ = tx.send(DataMsg::FollowState(n));
                        ctx2.request_repaint();
                    }
                });
            }
            ctx.request_repaint();
        }

        self.top_bar(ctx, &snap);
        if self.follow_on {
            egui::TopBottomPanel::top("followbar")
                .exact_height(34.0)
                .frame(egui::Frame::none().fill(Color32::from_rgb(40, 32, 16)).inner_margin(egui::Margin::symmetric(16.0, 6.0)))
                .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        let who = if self.follow_player.is_empty() { "Plexamp".into() } else { self.follow_player.clone() };
                        ui.label(RichText::new(format!("📱 Following {who} — transport controls the phone")).color(GOLD));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if hand(ui.button("Stop following")).clicked() {
                                self.exit_follow();
                            }
                        });
                    });
                });
        }
        self.notices_banner(ctx);
        self.play_bar(ctx, &snap);
        if self.tab == Tab::Library {
            let panel_w = (ctx.screen_rect().width() * 0.26).clamp(228.0, 300.0);
            egui::SidePanel::right("libqueue")
                .resizable(false)
                .exact_width(panel_w)
                .frame(egui::Frame::none().fill(CARD).inner_margin(egui::Margin::same(12.0)))
                .show(ctx, |ui| {
                    self.queue_body(ui, &snap, false);
                    ui.add_space(6.0);
                    ui.label(RichText::new("Tip: turn on Add-to-queue mode, then click songs to stack them here.").size(11.0).color(MUTED));
                });
        }
        self.central(ctx, &snap);
        self.notifications(ctx);
        self.overlays(ctx, &snap);
        self.resize_grips(ctx);

        // auto-dismiss toasts after a few seconds
        if self.toast != self.last_toast {
            self.last_toast = self.toast.clone();
            self.toast_at = if self.toast.is_empty() { None } else { Some(std::time::Instant::now()) };
        }
        if let Some(t) = self.toast_at {
            if t.elapsed().as_secs_f32() > 4.0 {
                self.toast.clear();
                self.toast_at = None;
                self.last_toast.clear();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }

        // persist the session (queue + position) every few seconds
        if self.last_save.elapsed().as_secs() >= 5 && !snap.queue.is_empty() {
            self.last_save = std::time::Instant::now();
            let sess = Session {
                tracks: snap.queue.clone(),
                index: snap.index,
                position: snap.position,
                volume: snap.volume,
            };
            thread::spawn(move || save_session(&sess));
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Session {
    #[serde(default)]
    tracks: Vec<Track>,
    #[serde(default)]
    index: usize,
    #[serde(default)]
    position: f32,
    #[serde(default)]
    volume: f32,
}

fn session_path() -> Option<std::path::PathBuf> {
    let base = std::env::var("APPDATA").ok()?;
    let dir = std::path::Path::new(&base).join("tulik-player");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("session.json"))
}

/// Stable per-install device id (persisted next to the session file) for presence
/// + hand-off. Generated once from the clock; no extra crate needed.
fn device_id() -> String {
    let path = std::env::var("APPDATA")
        .ok()
        .map(|b| std::path::Path::new(&b).join("tulik-player").join("device_id"));
    if let Some(p) = &path {
        if let Ok(s) = std::fs::read_to_string(p) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("pc-{n:x}");
    if let Some(p) = &path {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, &id);
    }
    id
}

fn load_session() -> Option<Session> {
    let p = session_path()?;
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn save_session(sess: &Session) {
    if let Some(p) = session_path() {
        if let Ok(s) = serde_json::to_string(sess) {
            let _ = std::fs::write(p, s);
        }
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context, snap: &audio::Shared) {
        egui::TopBottomPanel::top("top")
            .exact_height(58.0)
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::symmetric(16.0, 8.0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // brand: "tulik" 🐶(real dog mark) "player" — matches the web header
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.label(RichText::new("tulik").size(22.0).strong().color(TEXT));
                        if let Some(logo) = &self.logo {
                            ui.add(
                                egui::Image::new(egui::load::SizedTexture::from_handle(logo))
                                    .max_height(40.0)
                                    .max_width(48.0),
                            );
                        } else {
                            ui.label(RichText::new("🐶").size(22.0));
                        }
                        ui.label(RichText::new("player").size(22.0).color(ACCENT).strong());
                    });
                    ui.add_space(18.0);
                    pill_tab(ui, &mut self.tab, Tab::Player, "Player");
                    pill_tab(ui, &mut self.tab, Tab::Library, "Library");
                    pill_tab(ui, &mut self.tab, Tab::Playlists, "Playlists");
                    pill_tab(ui, &mut self.tab, Tab::History, "History");
                    ui.add_space(14.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search_text)
                            .hint_text("Search your library…")
                            .desired_width(240.0),
                    );
                    if resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.lib.q = self.search_text.clone();
                        self.lib.lyr.clear();
                        self.tab = Tab::Library;
                        self.album_view = None;
                        self.artist_view = None;
                        self.reload_library();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // tool icons (rightmost first)
                        if hand(ui.button(RichText::new("❓").size(18.0))).on_hover_text("Help — what every button does").clicked() {
                            self.show_help = true;
                        }
                        if hand(ui.add(egui::Button::new(RichText::new("⚡").size(18.0).color(Color32::BLACK)).fill(GOLD)))
                            .on_hover_text("Quick generate — AI builds & plays a playlist from a line")
                            .clicked()
                        {
                            self.show_qg = true;
                        }
                        if hand(ui.button(RichText::new("💬").size(18.0))).on_hover_text("Send feedback / report a bug").clicked() {
                            self.show_fb = true;
                        }
                        if hand(ui.button(RichText::new("🎚").size(18.0))).on_hover_text("EQ & audio (equalizer + balance)").clicked() {
                            self.show_eq = true;
                        }
                        ui.menu_button(RichText::new("📡").size(18.0), |ui| {
                            if hand(ui.button("📥  Pull what's playing here")).clicked() {
                                self.pull_from_plex();
                                ui.close_menu();
                            }
                            if hand(ui.button("🔄  Refresh devices")).clicked() {
                                self.load_players();
                                ui.close_menu();
                            }
                            if self.follow_on {
                                if hand(ui.button("📱  Stop following phone")).clicked() {
                                    self.exit_follow();
                                    ui.close_menu();
                                }
                            } else if hand(ui.button("📱  Follow phone (remote)")).clicked() {
                                self.enter_follow();
                                ui.close_menu();
                            }
                            ui.separator();
                            ui.label(RichText::new("Hand off queue to:").color(MUTED));
                            let rks: Vec<String> = snap.queue.iter().map(|t| t.rating_key.clone()).collect();
                            let index = snap.index;
                            let offset_ms = (snap.position * 1000.0) as i64;
                            if self.players.is_empty() {
                                ui.label(RichText::new("(no devices found)").size(12.0).color(MUTED));
                            }
                            for p in self.players.clone() {
                                let label = if p.is_mine {
                                    p.name.clone()
                                } else {
                                    format!("{} — {}'s", p.name, p.owner.clone().unwrap_or_default())
                                };
                                if hand(ui.add_enabled(p.online, egui::Button::new(label))).clicked() {
                                    if rks.is_empty() {
                                        self.toast = "Queue is empty".into();
                                    } else if p.is_mine {
                                        self.do_cast(p.id.clone(), rks.clone(), index, offset_ms, p.name.clone());
                                    } else {
                                        self.cast_confirm = Some((p.clone(), rks.clone(), index, offset_ms));
                                    }
                                    ui.close_menu();
                                }
                            }
                            // ⇄ your other players (web tabs / PWA / phone app / other PCs)
                            let web: Vec<Device> = self.web_devices.iter().filter(|d| d.kind == "web" && d.id != self.device_id).cloned().collect();
                            if !web.is_empty() {
                                ui.separator();
                                ui.label(RichText::new("Your players:").color(MUTED));
                                for d in web {
                                    ui.horizontal(|ui| {
                                        let dot = if d.playing { "▶" } else if d.online { "●" } else { "○" };
                                        let sub = if d.playing && !d.title.is_empty() { format!("  · {}", truncate(&d.title, 22)) } else { String::new() };
                                        ui.label(RichText::new(format!("{dot} {}{sub}", truncate(&d.name, 22))).size(12.5).color(TEXT));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if d.playing && hand(ui.small_button("⥁ Pull")).on_hover_text("Bring what it's playing to here").clicked() {
                                                self.do_pull_device(&d);
                                                ui.close_menu();
                                            }
                                            if d.online && hand(ui.small_button("▶ Send")).on_hover_text("Send this queue to that device").clicked() {
                                                self.do_handoff(&d, snap);
                                                ui.close_menu();
                                            }
                                        });
                                    });
                                }
                            }
                        })
                        .response
                        .on_hover_text("Cast / hand off to a device · pull what's playing");
                        let sleep_lbl = match snap.sleep_left {
                            Some(s) if s > 0.0 => format!("🌙 {}m", (s / 60.0).ceil() as i64),
                            _ => "🌙".into(),
                        };
                        ui.menu_button(RichText::new(sleep_lbl).size(16.0), |ui| {
                            ui.label(RichText::new("Sleep timer").strong());
                            for (mins, lbl) in [(0i64, "Off"), (15, "15 min"), (30, "30 min"), (45, "45 min"), (60, "60 min")] {
                                if ui.button(lbl).clicked() {
                                    let cmd = if mins == 0 { Cmd::SetSleep(None) } else { Cmd::SetSleep(Some(mins as f32 * 60.0)) };
                                    self.play(cmd);
                                    ui.close_menu();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Sleep timer — stop playback after a while");

                        ui.menu_button(RichText::new("🔈").size(18.0), |ui| {
                            ui.label(RichText::new("Output device").strong());
                            if hand(ui.selectable_label(snap.device_auto, "Auto (follow system)")).clicked() {
                                self.play(Cmd::SetDevice(None));
                                ui.close_menu();
                            }
                            ui.separator();
                            if snap.devices.is_empty() {
                                ui.label(RichText::new("(scanning…)").size(12.0).color(MUTED));
                            }
                            for d in &snap.devices {
                                let sel = !snap.device_auto && &snap.device == d;
                                if hand(ui.selectable_label(sel, d)).clicked() {
                                    self.play(Cmd::SetDevice(Some(d.clone())));
                                    ui.close_menu();
                                }
                            }
                        })
                        .response
                        .on_hover_text(format!("Output: {}", if snap.device.is_empty() { "default" } else { snap.device.as_str() }));
                    });
                });
            });
    }

    /// Floating, non-overlapping notifications (errors / toasts) under the top bar.
    fn notifications(&mut self, ctx: &egui::Context) {
        if self.err.is_empty() && self.toast.is_empty() {
            return;
        }
        let mut clear_err = false;
        egui::Area::new(egui::Id::new("notify"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (fill, txt, col) = if !self.err.is_empty() {
                    (Color32::from_rgb(60, 24, 28), self.err.clone(), Color32::from_rgb(255, 150, 150))
                } else {
                    (CARD2, self.toast.clone(), ACCENT)
                };
                egui::Frame::none()
                    .fill(fill)
                    .rounding(egui::Rounding::same(12.0))
                    .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(truncate(&txt, 90)).color(col));
                            if !self.err.is_empty() && hand(ui.small_button("×")).clicked() {
                                clear_err = true;
                            }
                        });
                    });
            });
        if clear_err {
            self.err.clear();
        }
    }

    /// Modal-ish overlay windows (Help, Feedback) + the fullscreen visualizer.
    fn overlays(&mut self, ctx: &egui::Context, snap: &audio::Shared) {
        self.full_viz_overlay(ctx, snap);
        if self.show_links {
            let mut open = true;
            egui::Window::new("⊞ More from tulik")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(320.0);
                    ui.label(RichText::new("Links open in your browser — music keeps playing here.").size(12.0).color(MUTED));
                    ui.add_space(8.0);
                    for (icon, name, url) in [
                        ("🌐", "Web player", format!("{}/player", self.base_url)),
                        ("📋", "Playlists", format!("{}/playlists", self.base_url)),
                        ("🏠", "Hub — downloader & all tools", format!("{}/hub/", self.base_url)),
                        ("📖", "Guide", format!("{}/hub/guide.html", self.base_url)),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon).size(15.0));
                            ui.hyperlink_to(RichText::new(name).size(14.5), url);
                        });
                        ui.add_space(4.0);
                    }
                });
            self.show_links = open;
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Player — what everything does")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(660.0);
                    ui.label(RichText::new("A quick tour of the buttons. Press Esc to close.").color(MUTED));
                    egui::ScrollArea::vertical().id_salt("helpscroll").max_height(420.0).show(ui, |ui| {
                        let w = 300.0;
                        let grid = |ui: &mut egui::Ui, cards: &[(&str, &str, &str)]| {
                            for pair in cards.chunks(2) {
                                ui.horizontal_top(|ui| {
                                    for (icon, t, d) in pair {
                                        help_card(ui, w, icon, t, d);
                                    }
                                });
                                ui.add_space(8.0);
                            }
                        };
                        help_section(ui, "TRANSPORT (BOTTOM BAR)");
                        grid(ui, &[
                            ("▶", "Play / pause", "Start or stop the current track (Space)."),
                            ("⏭", "Previous / next", "Jump between tracks in the queue (Shift+←/→)."),
                            ("🔀", "Shuffle", "Play the rest of the queue in random order."),
                            ("🔁", "Repeat", "Loop the whole queue, or one track."),
                            ("♥", "Love & rate", "Like the song or give it stars — saved to Plex."),
                            ("▬", "Seek bar", "Click or drag to jump anywhere in the song (←/→ skips 5s)."),
                            ("🔊", "Volume / mute", "Set the level for this app (M mutes)."),
                            ("⚖", "Balance", "Cycle L/R balance — 50/50 → A → B presets."),
                            ("📊", "Visualizer", "Click the little wave for a fullscreen visual (V)."),
                        ]);
                        help_section(ui, "TOOLS (TOP-RIGHT ICONS)");
                        grid(ui, &[
                            ("📡", "Cast / hand-off", "Pick a device in the menu to move playback there — or pull what's playing into this app."),
                            ("📱", "Follow phone", "Mirror & remote-control Plexamp on your phone — the transport buttons drive the phone."),
                            ("🎚", "EQ & audio", "10-band equalizer, presets, L/R balance, output device."),
                            ("🌙", "Sleep timer", "Auto-stop the music after a set time."),
                            ("⚡", "Quick generate", "AI builds & plays a playlist from one line."),
                            ("💬", "Feedback", "Report a bug or idea — goes straight to Gil."),
                        ]);
                        help_section(ui, "PLAYER VIEW");
                        grid(ui, &[
                            ("☄", "Start radio", "Builds an endless mix of songs that sound like this one."),
                            ("≡", "Queue & History", "Save the queue as a playlist, clear it, or click HISTORY for the full list."),
                            ("🎵", "Lyrics", "Synced to the song as it plays."),
                            ("🕘", "Recently played", "Your latest plays — click to play again."),
                        ]);
                        help_section(ui, "LIBRARY VIEW");
                        grid(ui, &[
                            ("🔍", "Search", "Find anything in the library — or 🎤 search by lyrics."),
                            ("🎯", "Focus filters", "Narrow by Genre, Decade, ❤ Loved, then sort."),
                            ("＋", "Add-to-queue mode", "When on, clicking a song just adds it to the queue."),
                            ("🎲", "Quick picks", "Recently added/played, Most played, Surprise me."),
                        ]);
                        help_section(ui, "GOOD TO KNOW");
                        grid(ui, &[
                            ("🖱", "Right-click a song", "Play next · Add to queue · Start radio · Heart · Go to artist."),
                            ("↘", "Resize", "Grab the glowing corners (bottom-left / bottom-right)."),
                        ]);
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(RichText::new("Open in your browser:").strong().color(ACCENT));
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to("🌐 Web player", format!("{}/player", self.base_url));
                        ui.hyperlink_to("📋 Playlists page", format!("{}/playlists", self.base_url));
                        ui.hyperlink_to("📖 Guide", format!("{}/hub/guide.html", self.base_url));
                        ui.hyperlink_to("🏠 Hub", format!("{}/hub/", self.base_url));
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("TulikPlayer — native · build {BUILD}")).size(11.0).color(MUTED));
                });
            self.show_help = open;
        }
        if self.show_fb {
            let mut open = true;
            egui::Window::new("Send feedback")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(440.0);
                    ui.label(RichText::new("Found a bug or want a feature? Tell me:").color(MUTED));
                    ui.add(egui::TextEdit::multiline(&mut self.fb_text).desired_rows(5).desired_width(410.0).hint_text("Type your feedback…"));
                    ui.add_space(6.0);
                    // 🐞 Bug / 💡 Idea tag (web parity) — toggling the active one clears it
                    ui.horizontal(|ui| {
                        for (k, lbl) in [("bug", "🐞 Bug"), ("idea", "💡 Idea")] {
                            let on = self.fb_kind == k;
                            let chip = egui::Button::new(RichText::new(lbl).color(if on { Color32::BLACK } else { TEXT }))
                                .fill(if on { GOLD } else { CARD2 })
                                .rounding(9.0);
                            if hand(ui.add(chip)).clicked() {
                                self.fb_kind = if on { String::new() } else { k.to_string() };
                            }
                        }
                        ui.add_space(10.0);
                        if hand(ui.checkbox(&mut self.fb_personal, "Only for me")).on_hover_text("A personal tweak just for your build, not everyone's").changed() {}
                    });
                    ui.add_space(6.0);
                    // 📸 screenshot attach (web parity: "paste a screenshot"). Capture
                    // grabs this window's framebuffer; Paste / Ctrl+V pulls an image off
                    // the clipboard (e.g. a Win+Shift+S grab).
                    ui.horizontal(|ui| {
                        let cap = egui::Button::new(RichText::new("📸 Capture app").color(TEXT)).fill(CARD2).rounding(9.0);
                        if hand(ui.add(cap)).on_hover_text("Snapshot the app window and attach it").clicked() {
                            self.fb_want_shot = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                        }
                        let paste = egui::Button::new(RichText::new("📋 Paste image").color(TEXT)).fill(CARD2).rounding(9.0);
                        if hand(ui.add(paste)).on_hover_text("Paste a screenshot from the clipboard (or press Ctrl+V)").clicked() {
                            match clipboard_image_png() {
                                Some((png, w, h)) => { self.fb_shot = Some(png); self.fb_shot_dim = (w, h); self.toast = "📋 Image pasted from clipboard".into(); }
                                None => { self.toast = "No image on the clipboard".into(); }
                            }
                        }
                        if self.fb_shot.is_some() {
                            ui.label(RichText::new(format!("✓ image {}×{}", self.fb_shot_dim.0, self.fb_shot_dim.1)).color(GOOD).size(12.5));
                            if hand(ui.small_button("✕")).on_hover_text("Remove attachment").clicked() {
                                self.fb_shot = None;
                            }
                        }
                    });
                    // Ctrl+V anywhere in the form pastes a clipboard image (matches the web).
                    if self.fb_shot.is_none() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V)) {
                        if let Some((png, w, h)) = clipboard_image_png() {
                            self.fb_shot = Some(png);
                            self.fb_shot_dim = (w, h);
                            self.toast = "📋 Image pasted from clipboard".into();
                        }
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let send = egui::Button::new(RichText::new("Send").color(Color32::BLACK).strong()).fill(GOLD).rounding(9.0);
                        if hand(ui.add(send)).clicked() {
                            let msg = self.fb_text.trim().to_string();
                            if !msg.is_empty() {
                                let api = self.api.clone();
                                let tx = self.data_tx.clone();
                                let ctx2 = self.ctx.clone();
                                let (kind, personal) = (self.fb_kind.clone(), self.fb_personal);
                                let shot = self.fb_shot.clone();
                                thread::spawn(move || {
                                    let m = match api.send_feedback(&msg, &kind, personal, "/player", shot) {
                                        Ok(()) => DataMsg::Toast("Thanks — feedback sent ✓".into()),
                                        Err(e) => DataMsg::Error(format!("Couldn't send: {e}")),
                                    };
                                    let _ = tx.send(m);
                                    ctx2.request_repaint();
                                });
                                self.fb_text.clear();
                                self.fb_kind.clear();
                                self.fb_personal = false;
                                self.fb_shot = None;
                                self.show_fb = false;
                            }
                        }
                        if hand(ui.button("Cancel")).clicked() {
                            self.show_fb = false;
                        }
                    });
                });
            self.show_fb = open;
        }
        if self.show_eq {
            let mut open = true;
            egui::Window::new("EQ & audio")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    // read current params
                    let mut p = self.audio.dsp.params.lock().map(|g| g.clone()).unwrap_or_default();
                    let mut changed = false;
                    ui.set_max_width(560.0);
                    ui.horizontal(|ui| {
                        if hand(ui.selectable_label(p.eq_on, RichText::new(if p.eq_on { "● EQ ON" } else { "○ EQ OFF" }).color(if p.eq_on { ACCENT } else { MUTED })))
                            .clicked()
                        {
                            p.eq_on = !p.eq_on;
                            changed = true;
                        }
                        if hand(ui.button("Reset")).clicked() {
                            p.eq_db = [0.0; 10];
                            changed = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.is_gil && hand(ui.selectable_label(self.show_viz, "🐶 Dancing puppy")).on_hover_text("Show/hide the dancing puppy in the playbar").clicked() {
                                self.show_viz = !self.show_viz;
                            }
                        });
                    });
                    ui.add_space(6.0);
                    // presets
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Presets").size(12.0).color(MUTED));
                        for (name, vals) in EQ_PRESETS {
                            if hand(ui.button(*name)).clicked() {
                                p.eq_db = *vals;
                                p.eq_on = true;
                                changed = true;
                            }
                        }
                    });
                    ui.add_space(8.0);
                    // 10 vertical band sliders, each with a dB readout above
                    ui.horizontal(|ui| {
                        let labels = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
                        for i in 0..10 {
                            ui.vertical(|ui| {
                                let g = p.eq_db[i] as i32;
                                let gc = if p.eq_db[i].abs() > 0.5 { ACCENT } else { MUTED };
                                ui.label(RichText::new(format!("{g:+}")).size(10.0).color(gc));
                                ui.spacing_mut().slider_width = 120.0;
                                if ui.add(egui::Slider::new(&mut p.eq_db[i], -12.0..=12.0).vertical().show_value(false)).changed() {
                                    changed = true;
                                }
                                ui.label(RichText::new(labels[i]).size(10.0).color(MUTED));
                            });
                            ui.add_space(6.0);
                        }
                    });
                    if changed {
                        if let Ok(mut g) = self.audio.dsp.params.lock() {
                            *g = p;
                        }
                    }
                    ui.add_space(10.0);
                    ui.separator();
                    ui.label(RichText::new("L/R Balance (speakers)").strong());
                    ui.horizontal(|ui| {
                        for (i, nm) in [(0usize, "50/50"), (1, "A"), (2, "B")] {
                            if hand(ui.selectable_label(self.bal_active == i, nm)).clicked() {
                                self.bal_active = i;
                                let b = self.bal_presets[i];
                                self.set_balance(b);
                            }
                        }
                        ui.label(RichText::new(bal_readout(self.bal_presets[self.bal_active])).color(MUTED));
                    });
                    for (i, nm) in [(1usize, "Preset A"), (2, "Preset B")] {
                        ui.horizontal(|ui| {
                            ui.label(format!("{nm}:"));
                            let mut v = self.bal_presets[i];
                            let rd = bal_readout(v);
                            if ui.add(egui::Slider::new(&mut v, -1.0..=1.0).show_value(false).text(rd)).changed() {
                                self.bal_presets[i] = v;
                                if self.bal_active == i {
                                    self.set_balance(v);
                                }
                            }
                        });
                    }
                });
            self.show_eq = open;
        }
        if self.show_qg && !self.qg_busy {
            let mut open = true;
            let mut go = false;
            egui::Window::new("⚡ Quick generate")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(460.0);
                    ui.label(RichText::new("Ask for anything — a vibe, an activity, a mashup. AI builds & plays it (~1–2 min).").color(MUTED));
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut self.qg_prompt).hint_text("e.g. upbeat 90s road-trip songs").desired_width(440.0));
                    ui.add_space(6.0);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if hand(ui.button("⚡ Generate")).clicked() || enter {
                        go = true;
                    }
                });
            self.show_qg = open;
            if go {
                let p = self.qg_prompt.clone();
                self.start_qg(p);
                self.show_qg = false;
            }
        }
        if self.qg_busy {
            egui::Window::new("Building your playlist…")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(380.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("⚡").size(28.0));
                        ui.label(RichText::new(&self.qg_phase).color(MUTED));
                        ui.add_space(8.0);
                        ui.add(egui::ProgressBar::new(self.qg_pct / 100.0).desired_width(320.0).text(format!("{}%", self.qg_pct as i32)));
                        ui.add_space(4.0);
                        ui.label(RichText::new("AI powered — usually 1–2 minutes").size(11.0).color(MUTED));
                    });
                });
        }
        if let Some((p, rks, index, offset)) = self.cast_confirm.clone() {
            let mut decided: Option<bool> = None;
            egui::Window::new("Cast to someone else's device?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new(format!(
                        "This starts music on “{}” — that's {}'s device, not yours. They'll hear it wherever they are.",
                        p.name,
                        p.owner.clone().unwrap_or_else(|| "someone else".into())
                    )).color(TEXT));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if hand(ui.button("Cast it")).clicked() {
                            decided = Some(true);
                        }
                        if hand(ui.button("Cancel")).clicked() {
                            decided = Some(false);
                        }
                    });
                });
            match decided {
                Some(true) => {
                    self.do_cast(p.id.clone(), rks.clone(), index, offset, p.name.clone());
                    self.cast_confirm = None;
                }
                Some(false) => self.cast_confirm = None,
                None => {}
            }
        }
    }

    fn central(&mut self, ctx: &egui::Context, snap: &audio::Shared) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                match self.tab {
                    Tab::Player => self.player_view(ui, snap),
                    Tab::Library => self.library_view(ui),
                    Tab::History => self.history_view(ui),
                    Tab::Playlists => self.playlists_view(ui),
                }
            });
    }

    // -------------------- PLAYER TAB --------------------
    fn player_view(&mut self, ui: &mut egui::Ui, snap: &audio::Shared) {
        let cur = if self.follow_on { self.follow_track.clone() } else { snap.current.clone() };
        let mut nav_artist: Option<String> = None;
        let mut nav_album: Option<Track> = None;
        let mut cover_radio: Option<Track> = None;
        let mut cover_heart: Option<String> = None;
        card(CARD).show(ui, |ui| {
            ui.set_height(176.0);
            ui.horizontal(|ui| {
                if let Some(t) = &cur {
                    if let Some(tex) = self.art.get(&t.rating_key) {
                        // soft glow behind the cover (web's .coverglow): a few
                        // concentric translucent discs in the art's mean color
                        let (rect, cover_resp) = ui.allocate_exact_size(egui::vec2(152.0, 152.0), egui::Sense::click());
                        cover_resp.context_menu(|ui| {
                            if ui.button("📻  Start radio from this").clicked() { cover_radio = Some(t.clone()); ui.close_menu(); }
                            if ui.button("♥  Heart").clicked() { cover_heart = Some(t.rating_key.clone()); ui.close_menu(); }
                            ui.separator();
                            if ui.button("🎤  Go to artist").clicked() { nav_artist = Some(t.artist.clone()); ui.close_menu(); }
                            if !t.album.is_empty() && ui.button("💿  Go to album").clicked() { nav_album = Some(t.clone()); ui.close_menu(); }
                        });
                        if let Some(c) = self.art.avg(&t.rating_key) {
                            let p = ui.painter();
                            for (r, a) in [(102.0, 20u8), (88.0, 28), (76.0, 38)] {
                                p.circle_filled(rect.center(), r, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a));
                            }
                        }
                        img(&tex, 152.0).paint_at(ui, rect);
                    } else {
                        ui.add_space(156.0);
                    }
                    ui.add_space(18.0);
                    ui.vertical(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(&t.title).size(30.0).strong().color(TEXT));
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if hand(ui.add(egui::Label::new(RichText::new(&t.artist).size(16.0).color(MUTED)).sense(egui::Sense::click())))
                                .on_hover_text("Go to artist")
                                .clicked()
                            {
                                nav_artist = Some(t.artist.clone());
                            }
                            if !t.album.is_empty() {
                                ui.label(RichText::new("·").size(16.0).color(MUTED));
                                if hand(ui.add(egui::Label::new(RichText::new(&t.album).size(16.0).color(MUTED)).sense(egui::Sense::click())))
                                    .on_hover_text("Go to album")
                                    .clicked()
                                {
                                    nav_album = Some(t.clone());
                                }
                            }
                        });
                        ui.add_space(10.0);
                        if !self.np_badge.is_empty() {
                            badge_chip(ui, &self.np_badge);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        ui.add_space(4.0);
                        let lbl = if self.radio_busy { "☄ Building radio…" } else { "☄ Start radio from this track" };
                        let b = egui::Button::new(RichText::new(lbl).color(Color32::BLACK).strong())
                            .fill(GOLD)
                            .rounding(12.0)
                            .min_size(egui::vec2(0.0, 38.0));
                        if hand(ui.add_enabled(!self.radio_busy, b)).clicked() {
                            self.radio_busy = true;
                            spawn_radio(&self.api, &self.data_tx, &self.ctx, t.clone(), RadioKind::ReplaceRest);
                        }
                    });
                } else {
                    ui.add_space(156.0);
                    ui.vertical(|ui| {
                        ui.add_space(60.0);
                        ui.label(
                            RichText::new(if snap.status.is_empty() {
                                "Nothing playing — pick something from your Library"
                            } else {
                                snap.status.as_str()
                            })
                            .size(18.0)
                            .color(MUTED),
                        );
                    });
                }
            });
        });
        if let Some(a) = nav_artist {
            self.open_artist(a);
        }
        if let Some(t) = nav_album {
            self.goto_album(&t);
        }
        if let Some(t) = cover_radio {
            self.radio_busy = true;
            spawn_radio(&self.api, &self.data_tx, &self.ctx, t, RadioKind::ReplaceRest);
        }
        if let Some(rk) = cover_heart {
            self.api.rate(&rk, 10.0);
            self.np_rating = 10.0;
            self.toast = "♥ Hearted".into();
        }

        ui.add_space(12.0);
        let h = (ui.available_height() - 4.0).max(140.0);
        ui.columns(3, |cols| {
            self.col_queue(&mut cols[0], snap, h);
            self.col_lyrics(&mut cols[1], snap, h);
            self.col_recent(&mut cols[2], h);
        });
    }

    fn col_queue(&mut self, ui: &mut egui::Ui, snap: &audio::Shared, h: f32) {
        card(CARD).show(ui, |ui| {
            ui.set_height(h);
            self.queue_body(ui, snap, true);
        });
    }

    /// Queue header + rows, reused by the Player column and the Library side panel.
    fn queue_body(&mut self, ui: &mut egui::Ui, snap: &audio::Shared, show_hist: bool) {
        let hist_n = self.history.as_ref().map(|v| v.len()).unwrap_or(0);
        {
            ui.horizontal(|ui| {
                ui.label(RichText::new("QUEUE").size(12.0).strong().color(MUTED));
                ui.label(RichText::new(format!("· {}", snap.queue.len())).size(12.0).color(MUTED));
                if show_hist {
                    let h = hand(ui.add(
                        egui::Label::new(RichText::new(format!("  HISTORY · {hist_n}")).size(12.0).color(MUTED))
                            .sense(egui::Sense::click()),
                    ));
                    if h.on_hover_text("Open full history").clicked() {
                        self.tab = Tab::History;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // lowercase ghost actions, like the web queue header
                    let clear = egui::Button::new(RichText::new("clear").size(12.0).color(MUTED)).frame(false);
                    if hand(ui.add(clear)).on_hover_text("Empty the queue").clicked() {
                        self.play(Cmd::Clear);
                    }
                    let save = egui::Button::new(RichText::new("save").size(12.0).color(MUTED)).frame(false);
                    if hand(ui.add(save)).on_hover_text("Save the queue as a Plex playlist").clicked() {
                        self.save_open = !self.save_open;
                    }
                });
            });
            if self.save_open {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.save_title).hint_text("Playlist name…").desired_width(150.0));
                    if hand(ui.button("Save")).clicked() {
                        let title = self.save_title.trim().to_string();
                        let rks: Vec<String> = snap.queue.iter().map(|t| t.rating_key.clone()).collect();
                        if !title.is_empty() && !rks.is_empty() {
                            let api = self.api.clone();
                            let tx = self.data_tx.clone();
                            let ctx = self.ctx.clone();
                            thread::spawn(move || {
                                let msg = match api.save_queue(&title, &rks) {
                                    Ok(()) => DataMsg::Toast(format!("Saved playlist “{title}”")),
                                    Err(e) => DataMsg::Error(e.to_string()),
                                };
                                let _ = tx.send(msg);
                                ctx.request_repaint();
                            });
                            self.save_open = false;
                            self.save_title.clear();
                        }
                    }
                });
            }
            ui.add_space(6.0);
            if snap.queue.is_empty() {
                ui.add_space(36.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Queue is empty").color(MUTED));
                    ui.label(RichText::new("Play something, or use ＋ Queue mode in the Library.").size(11.5).color(FAINT));
                });
                return;
            }
            let mut jump = None;
            let mut remove = None;
            let mut move_op: Option<(usize, usize)> = None;
            // right-click menu actions (applied after the scroll area)
            let mut m_next: Option<Track> = None;
            let mut m_add: Option<Track> = None;
            let mut m_radio: Option<Track> = None;
            let mut m_heart: Option<String> = None;
            let mut m_artist: Option<String> = None;
            let mut m_album: Option<Track> = None;
            let qlen = snap.queue.len();
            egui::ScrollArea::vertical().id_salt("q").auto_shrink([false, false]).show_rows(ui, 54.0, qlen, |ui, range| {
                for i in range {
                    let t = &snap.queue[i];
                    let tex = self.art.get(&t.rating_key);
                    let here = i == snap.index;
                    row_frame(here).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(tex) = &tex {
                                ui.add(img(tex, 38.0));
                            }
                            let (a, b) = track_two_lines(t);
                            let row = hand(ui.add(two_line_btn(&a, &b, here)));
                            if row.clicked() {
                                jump = Some(i);
                            }
                            row.context_menu(|ui| {
                                if ui.button("⏭  Play next").clicked() { m_next = Some(t.clone()); ui.close_menu(); }
                                if ui.button("➕  Add to queue").clicked() { m_add = Some(t.clone()); ui.close_menu(); }
                                if ui.button("📻  Start radio").clicked() { m_radio = Some(t.clone()); ui.close_menu(); }
                                if ui.button("♥  Heart").clicked() { m_heart = Some(t.rating_key.clone()); ui.close_menu(); }
                                ui.separator();
                                if ui.button("🎤  Go to artist").clicked() { m_artist = Some(t.artist.clone()); ui.close_menu(); }
                                if !t.album.is_empty() && ui.button("💿  Go to album").clicked() { m_album = Some(t.clone()); ui.close_menu(); }
                                ui.separator();
                                if ui.button("✕  Remove from queue").clicked() { remove = Some(i); ui.close_menu(); }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if hand(ui.add(egui::Button::new(RichText::new("×").size(16.0)).frame(false)))
                                    .on_hover_text("Remove")
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                                if i + 1 < qlen && hand(ui.add(egui::Button::new(RichText::new("▾").size(13.0)).frame(false))).on_hover_text("Move down").clicked() {
                                    move_op = Some((i, i + 1));
                                }
                                if i > 0 && hand(ui.add(egui::Button::new(RichText::new("▴").size(13.0)).frame(false))).on_hover_text("Move up").clicked() {
                                    move_op = Some((i, i - 1));
                                }
                            });
                        });
                    });
                }
            });
            if let Some(i) = jump {
                self.play(Cmd::JumpTo(i));
            }
            if let Some(i) = remove {
                self.play(Cmd::RemoveAt(i));
            }
            if let Some((from, to)) = move_op {
                self.play(Cmd::Move(from, to));
            }
            if let Some(t) = m_next {
                self.play(Cmd::PlayNext(t));
            }
            if let Some(t) = m_add {
                self.play(Cmd::EnqueueEnd(vec![t]));
            }
            if let Some(t) = m_radio {
                spawn_radio(&self.api, &self.data_tx, &self.ctx, t, RadioKind::PlaySeed);
            }
            if let Some(rk) = m_heart {
                self.api.rate(&rk, 10.0);
                self.toast = "♥ Hearted".into();
            }
            if let Some(a) = m_artist {
                self.open_artist(a);
            }
            if let Some(t) = m_album {
                self.goto_album(&t);
            }
        }
    }

    fn col_lyrics(&mut self, ui: &mut egui::Ui, snap: &audio::Shared, h: f32) {
        let tx = self.audio.tx.clone();
        let can_seek = !self.follow_on; // line-click seeks local playback only
        let lyr = self.np_lyrics.clone();
        let synced = lyr.as_ref().map(|l| l.has_synced).unwrap_or(false);
        let active = if synced {
            let pos = snap.position;
            lyr.as_ref()
                .map(|l| l.synced.iter().rposition(|x| x.t <= pos).map(|i| i as i64).unwrap_or(-1))
                .unwrap_or(-1)
        } else {
            -1
        };
        let changed = active != self.np_lyric_idx;
        self.np_lyric_idx = active;

        // idle-panel intent is collected here and acted on AFTER the render closure
        // (the closure must not capture &mut self — file-wide pattern, cf. `quick`).
        let mut idle_act: Option<u8> = None;
        card(CARD).show(ui, |ui| {
            ui.set_height(h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("LYRICS").size(12.0).strong().color(MUTED));
                if synced {
                    ui.label(RichText::new("· SYNCED").size(12.0).color(ACCENT));
                }
            });
            ui.add_space(6.0);
            // idle = nothing playing at all (vs. a track playing whose lyrics are pending)
            let idle = snap.current.is_none() && lyr.is_none();
            let empty_msg = match &lyr {
                Some(l) if l.has_synced || l.plain.is_some() => None,
                Some(_) => Some("No lyrics found."),
                None if !idle => Some("Looking for lyrics…"),
                None => None,
            };
            if idle {
                // web parity (C6): "Start something" idle panel in the Lyrics tile —
                // jumping-off points when nothing is playing.
                ui.add_space((ui.available_height() * 0.18).max(0.0));
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Start something").size(18.0).strong().color(TEXT));
                    ui.add_space(2.0);
                    ui.label(RichText::new("Nothing's playing — pick a jumping-off point:").size(12.5).color(MUTED));
                    ui.add_space(12.0);
                });
                let mut act: Option<u8> = None;
                ui.vertical_centered(|ui| {
                    let w = 220.0;
                    for (id, lbl) in [(0u8, "🎲  Surprise me"), (1, "⭐  Top tracks"), (2, "🕘  Recently played"), (3, "📃  Playlists")] {
                        let b = egui::Button::new(RichText::new(lbl).color(TEXT)).fill(CARD2).rounding(9.0);
                        if hand(ui.add_sized([w, 34.0], b)).clicked() { act = Some(id); }
                        ui.add_space(6.0);
                    }
                });
                idle_act = act;
                return;
            }
            if let Some(msg) = empty_msg {
                // centered placeholder, like the web's lyrics tile
                ui.add_space((ui.available_height() * 0.42).max(0.0));
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(msg).size(15.0).color(MUTED));
                });
                return;
            }
            egui::ScrollArea::vertical().id_salt("ly").auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                match &lyr {
                    Some(l) if l.has_synced => {
                        for (i, line) in l.synced.iter().enumerate() {
                            let is_a = i as i64 == active;
                            let col = if is_a { TEXT } else { MUTED };
                            let mut rt = RichText::new(if line.line.is_empty() { "♪" } else { &line.line })
                                .size(if is_a { 18.0 } else { 15.0 })
                                .color(col);
                            if is_a {
                                rt = rt.strong();
                            }
                            let mut lab = egui::Label::new(rt).wrap_mode(egui::TextWrapMode::Wrap);
                            if can_seek {
                                lab = lab.sense(egui::Sense::click());
                            }
                            let r = ui.add(lab);
                            if can_seek && hand(r.clone()).clicked() {
                                let _ = tx.send(Cmd::Seek(line.t));
                            }
                            if is_a && changed {
                                r.scroll_to_me(Some(egui::Align::Center));
                            }
                            ui.add_space(6.0);
                        }
                    }
                    Some(l) => {
                        ui.label(RichText::new(l.plain.as_deref().unwrap_or("")).size(15.0).color(MUTED));
                    }
                    None => {}
                }
            });
        });
        // act on the idle-panel choice outside the render closure (no &mut self capture)
        match idle_act {
            Some(0) => { self.tab = Tab::Library; self.quick(|a| a.albums_f("name", "", "", None, false, "random").map(LibResults::Albums)); }
            Some(1) => { self.tab = Tab::Library; self.quick(|a| a.top().map(LibResults::Songs)); }
            Some(2) => { self.tab = Tab::Library; self.quick(|a| a.history().map(LibResults::Songs)); }
            Some(3) => { self.tab = Tab::Playlists; }
            _ => {}
        }
    }

    fn col_recent(&mut self, ui: &mut egui::Ui, h: f32) {
        let data = self.history.take();
        card(CARD).show(ui, |ui| {
            ui.set_height(h);
            ui.label(RichText::new("RECENTLY PLAYED").size(12.0).strong().color(MUTED));
            ui.add_space(6.0);
            match &data {
                Some(v) => self.render_track_rows(ui, v, false, None),
                None => {
                    ui.label(RichText::new("Loading…").color(MUTED));
                }
            }
        });
        self.history = data;
    }

    // -------------------- LIBRARY TAB --------------------
    fn library_view(&mut self, ui: &mut egui::Ui) {
        // album / artist detail panels take over when open
        if self.album_view.is_some() {
            let d = self.album_view.take();
            let mut back = false;
            if hand(ui.button("◀ Back")).clicked() {
                back = true;
            }
            if let Some((album, artist, tracks)) = &d {
                ui.heading(album);
                ui.label(RichText::new(artist.as_str()).color(MUTED));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if hand(ui.button("▶  Play all")).clicked() {
                        self.play(Cmd::SetQueue { tracks: tracks.clone(), start: 0 });
                        self.tab = Tab::Player;
                    }
                    if hand(ui.button("🔀  Shuffle")).clicked() {
                        self.play(Cmd::SetQueue { tracks: tracks.clone(), start: 0 });
                        self.play(Cmd::Shuffle);
                        self.tab = Tab::Player;
                    }
                    if hand(ui.button("➕  Queue all")).clicked() {
                        self.play(Cmd::EnqueueEnd(tracks.clone()));
                    }
                });
                ui.add_space(8.0);
                self.render_track_rows(ui, tracks, false, None);
            }
            if !back {
                self.album_view = d;
            }
            return;
        }
        if self.artist_view.is_some() {
            let d = self.artist_view.take();
            let mut back = false;
            if hand(ui.button("◀ Back")).clicked() {
                back = true;
            }
            if let Some((name, albums)) = &d {
                ui.heading(name.as_str());
                ui.add_space(8.0);
                self.render_albums(ui, albums, None);
            }
            if !back {
                self.artist_view = d;
            }
            return;
        }

        self.library_toolbar(ui);
        ui.add_space(12.0);

        let data = self.lib_results.take();
        let add_mode = self.lib.add_mode;
        // A–Z rail (web parity): only for the normal browse, sorted A–Z,
        // and only when at least 3 distinct letters are present.
        let letters: Vec<(char, usize)> = if !self.lib_is_quick && self.lib.sort == "name" {
            match &data {
                Some(LibResults::Artists(v)) => az_index(v.iter().map(|a| a.name.as_str())),
                Some(LibResults::Albums(v)) => az_index(v.iter().map(|a| a.album.as_str())),
                Some(LibResults::Songs(v)) => az_index(v.iter().map(|t| t.title.as_str())),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let show_rail = letters.len() >= 3;
        let mut chip_reload = false;
        match &data {
            Some(LibResults::Artists(v)) => {
                chip_reload |= self.lib_summary(ui, v.len(), "artists");
                ui.add_space(8.0);
                let jump = self.az_jump.take();
                self.with_az_rail(ui, show_rail, &letters, |s, ui| s.render_artists(ui, v, jump));
            }
            Some(LibResults::Albums(v)) => {
                chip_reload |= self.lib_summary(ui, v.len(), "albums");
                ui.add_space(8.0);
                let jump = self.az_jump.take();
                self.with_az_rail(ui, show_rail, &letters, |s, ui| s.render_albums(ui, v, jump));
            }
            Some(LibResults::Songs(v)) => {
                ui.horizontal(|ui| {
                    chip_reload |= self.lib_summary(ui, v.len(), "songs");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if hand(ui.button("🔀 Shuffle")).clicked() {
                            self.play(Cmd::SetQueue { tracks: v.clone(), start: 0 });
                            self.play(Cmd::Shuffle);
                            self.tab = Tab::Player;
                        }
                        if hand(ui.button("▶ Play all")).clicked() {
                            self.play(Cmd::SetQueue { tracks: v.clone(), start: 0 });
                            self.tab = Tab::Player;
                        }
                    });
                });
                ui.add_space(8.0);
                let jump = self.az_jump.take();
                self.with_az_rail(ui, show_rail, &letters, |s, ui| s.render_track_rows(ui, v, add_mode, jump));
            }
            None => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Loading your library…").color(MUTED));
                });
            }
        }
        self.lib_results = data;
        if chip_reload {
            self.reload_library();
        }
    }

    /// Lay out `body` next to the A–Z jump rail (when shown) and route rail
    /// clicks into `self.az_jump`, applied by the renderers on the next frame.
    fn with_az_rail(
        &mut self,
        ui: &mut egui::Ui,
        show: bool,
        letters: &[(char, usize)],
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        if !show {
            body(self, ui);
            return;
        }
        let avail = ui.available_size();
        let mut clicked = None;
        ui.horizontal(|ui| {
            ui.allocate_ui(egui::vec2(avail.x - 26.0, avail.y), |ui| body(self, ui));
            clicked = az_rail(ui, letters);
        });
        if clicked.is_some() {
            self.az_jump = clicked;
        }
    }

    fn library_toolbar(&mut self, ui: &mut egui::Ui) {
        let mut reload = false;
        let mut quick: Option<u8> = None; // deferred quick-pick (avoids self double-borrow)

        egui::Frame::none()
            .fill(CARD)
            .rounding(egui::Rounding::same(14.0))
            .inner_margin(egui::Margin::symmetric(14.0, 12.0))
            .show(ui, |ui| {
                // ---- Band 1: browse segmented control + search ----
                ui.horizontal(|ui| {
                    // segmented Artists | Albums | Songs (one connected capsule)
                    egui::Frame::none()
                        .fill(CARD2)
                        .rounding(egui::Rounding::same(999.0))
                        .inner_margin(egui::Margin::same(3.0))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for (t, label) in [(LibTarget::Artists, "Artists"), (LibTarget::Albums, "Albums"), (LibTarget::Songs, "Songs")] {
                                let sel = self.lib.target == t;
                                let txt = if sel {
                                    RichText::new(label).strong().color(Color32::BLACK)
                                } else {
                                    RichText::new(label).color(MUTED)
                                };
                                let b = egui::Button::new(txt)
                                    .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                                    .rounding(999.0)
                                    .min_size(egui::vec2(76.0, 28.0));
                                if hand(ui.add(b)).clicked() && !sel {
                                    self.lib.target = t;
                                    reload = true;
                                }
                            }
                        });

                    // search capsules, pushed to the right edge
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // lyric search (secondary) sits on the far right…
                        if search_pill(ui, "🎤", "Search lyrics…", &mut self.lib.lyr, 140.0) {
                            reload = true;
                        }
                        ui.add_space(8.0);
                        // …main library search to its left
                        if search_pill(ui, "🔍", "Search your library…", &mut self.lib.q, 200.0) {
                            self.lib.lyr.clear();
                            reload = true;
                        }
                    });
                });

                ui.add_space(11.0);

                // ---- Band 2: focus facets (left) + sort & add-mode (right) ----
                ui.horizontal(|ui| {
                    section_label(ui, "Focus");
                    ui.add_space(2.0);

                    // genre facet
                    let genres = self.genres.clone().unwrap_or_default();
                    let mut g = self.lib.genre.clone();
                    egui::ComboBox::from_id_salt("genre")
                        .selected_text(g.clone().unwrap_or_else(|| "Genre".into()))
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(g.is_none(), "All genres").clicked() {
                                g = None;
                            }
                            for it in &genres {
                                if ui.selectable_label(g.as_deref() == Some(it.name.as_str()), format!("{} ({})", it.name, it.count)).clicked() {
                                    g = Some(it.name.clone());
                                }
                            }
                        });
                    if g != self.lib.genre {
                        self.lib.genre = g;
                        reload = true;
                    }

                    // decade facet
                    let decades = self.decades.clone().unwrap_or_default();
                    let mut d = self.lib.decade;
                    egui::ComboBox::from_id_salt("decade")
                        .selected_text(d.map(|x| format!("{x}s")).unwrap_or_else(|| "Decade".into()))
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(d.is_none(), "All decades").clicked() {
                                d = None;
                            }
                            for it in &decades {
                                if ui.selectable_label(d == Some(it.decade), format!("{}s ({} albums)", it.decade, it.albums)).clicked() {
                                    d = Some(it.decade);
                                }
                            }
                        });
                    if d != self.lib.decade {
                        self.lib.decade = d;
                        reload = true;
                    }

                    // loved facet
                    if hand(ui.selectable_label(self.lib.loved, RichText::new("❤ Loved").color(if self.lib.loved { ACCENT } else { MUTED }))).clicked() {
                        self.lib.loved = !self.lib.loved;
                        reload = true;
                    }

                    // sort + add-mode, right-aligned
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // add-to-queue mode — a distinct, clearly-armed toggle
                        let on = self.lib.add_mode;
                        let b = egui::Button::new(RichText::new("＋ Queue mode").size(12.5).color(if on { Color32::BLACK } else { MUTED }))
                            .fill(if on { GOOD } else { Color32::TRANSPARENT })
                            .stroke(egui::Stroke::new(1.0, if on { GOOD } else { CARD2 }))
                            .rounding(999.0)
                            .min_size(egui::vec2(0.0, 28.0));
                        if hand(ui.add(b)).on_hover_text("When ON, clicking a song adds it to the queue instead of playing").clicked() {
                            self.lib.add_mode = !self.lib.add_mode;
                        }
                        ui.add_space(6.0);
                        // sort
                        let mut sort = self.lib.sort.clone();
                        egui::ComboBox::from_id_salt("sort")
                            .selected_text(sort_label(&sort))
                            .show_ui(ui, |ui| {
                                for (val, lbl) in [("name", "Sort: A–Z"), ("year", "Sort: Year"), ("added", "Sort: Recently added"), ("count", "Sort: Most tracks")] {
                                    ui.selectable_value(&mut sort, val.to_string(), lbl);
                                }
                            });
                        if sort != self.lib.sort {
                            self.lib.sort = sort;
                            reload = true;
                        }
                    });
                });

                ui.add_space(11.0);
                // hairline divider — quick picks are a separate "smart lists" layer
                let (_, line) = ui.allocate_space(egui::vec2(ui.available_width(), 1.0));
                ui.painter().hline(line.x_range(), line.center().y, egui::Stroke::new(1.0, CARD2));
                ui.add_space(11.0);

                // ---- Band 3: quick picks (ghost pills) ----
                ui.horizontal(|ui| {
                    section_label(ui, "Quick picks");
                    ui.add_space(2.0);
                    if ghost_pill(ui, "Recently added").clicked() { quick = Some(0); }
                    if ghost_pill(ui, "Recently played").clicked() { quick = Some(1); }
                    if ghost_pill(ui, "Most played").clicked() { quick = Some(2); }
                    if ghost_pill(ui, "🎲 Surprise").clicked() { quick = Some(3); }
                });
            });

        match quick {
            Some(0) => self.quick(|a| a.recent().map(LibResults::Songs)),
            Some(1) => self.quick(|a| a.history().map(LibResults::Songs)),
            Some(2) => self.quick(|a| a.top().map(LibResults::Songs)),
            Some(3) => self.quick(|a| a.albums_f("name", "", "", None, false, "random").map(LibResults::Albums)),
            _ => {}
        }
        if reload {
            self.reload_library();
        }
    }

    /// Calm summary line under the toolbar: bold count + noun, then removable
    /// chips for every active filter. Returns true if a chip was removed.
    fn lib_summary(&mut self, ui: &mut egui::Ui, count: usize, noun: &str) -> bool {
        let mut reload = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
            ui.label(RichText::new(count.to_string()).strong().color(TEXT));
            ui.label(RichText::new(noun).color(MUTED));

            if !self.lib.q.is_empty() {
                if filter_chip(ui, &format!("🔍 “{}”", truncate(&self.lib.q, 22))) {
                    self.lib.q.clear();
                    reload = true;
                }
            }
            if !self.lib.lyr.is_empty() {
                if filter_chip(ui, &format!("🎤 “{}”", truncate(&self.lib.lyr, 22))) {
                    self.lib.lyr.clear();
                    reload = true;
                }
            }
            if let Some(g) = self.lib.genre.clone() {
                if filter_chip(ui, &g) {
                    self.lib.genre = None;
                    reload = true;
                }
            }
            if let Some(d) = self.lib.decade {
                if filter_chip(ui, &format!("{d}s")) {
                    self.lib.decade = None;
                    reload = true;
                }
            }
            if self.lib.loved && filter_chip(ui, "❤ Loved") {
                self.lib.loved = false;
                reload = true;
            }
        });
        reload
    }

    /// Run a quick-pick fetch into the library body.
    fn quick<F>(&mut self, f: F)
    where
        F: FnOnce(&ApiClient) -> anyhow::Result<LibResults> + Send + 'static,
    {
        self.lib_loading = true;
        self.lib_is_quick = true;
        self.az_jump = None;
        self.lib_results = None;
        self.fetch(move |a| match f(a) {
            Ok(r) => DataMsg::Lib(r),
            Err(e) => DataMsg::Error(e.to_string()),
        });
    }

    /// Run the FFT over the DSP sample tap and smooth into `self.viz_bars`
    /// (attack fast, decay slow). `bars` lets the playbar strip (28) and the
    /// fullscreen visualizer (72) share one pipeline.
    fn update_viz_levels(&mut self, bars: usize) {
        if self.viz_bars.len() != bars {
            self.viz_bars = vec![0.0; bars];
        }
        let samples: Vec<f32> = match self.audio.dsp.viz.lock() {
            Ok(v) => {
                let n = v.len();
                let take = n.min(1024);
                v.iter().skip(n - take).copied().collect()
            }
            Err(_) => Vec::new(),
        };
        let mut target = vec![0.0f32; bars];
        if samples.len() >= 256 {
            let n = 1024;
            let mut re = vec![0.0f32; n];
            let mut im = vec![0.0f32; n];
            let m = samples.len();
            let win = m.min(n);
            for i in 0..n {
                let s = if i < win { samples[m - win + i] } else { 0.0 };
                let w = 0.5 - 0.5 * ((2.0 * std::f32::consts::PI * i as f32) / (n as f32 - 1.0)).cos();
                re[i] = s * w;
            }
            fft(&mut re, &mut im);
            let minb = 2.0f32;
            let maxb = 480.0f32;
            for bi in 0..bars {
                let f0 = minb * (maxb / minb).powf(bi as f32 / bars as f32);
                let f1 = minb * (maxb / minb).powf((bi + 1) as f32 / bars as f32);
                let lo = (f0 as usize).max(1);
                let hi = (f1 as usize).max(lo + 1).min(n / 2);
                let mut mag = 0.0f32;
                for k in lo..hi {
                    let m2 = re[k] * re[k] + im[k] * im[k];
                    if m2 > mag {
                        mag = m2;
                    }
                }
                let db = 20.0 * (mag.sqrt() + 1e-6).log10();
                target[bi] = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            }
        }
        // attack fast, decay slow
        for i in 0..bars {
            if target[i] > self.viz_bars[i] {
                self.viz_bars[i] = target[i];
            } else {
                self.viz_bars[i] *= 0.85;
            }
        }
    }

    /// The small spectrum strip in the playbar (click it — or press V — for
    /// Overall audio energy (0..1) from the FFT tap — drives the puppy's bounce.
    fn viz_energy(&self) -> f32 {
        if self.viz_bars.is_empty() {
            return 0.0;
        }
        let s: f32 = self.viz_bars.iter().sum();
        (s / self.viz_bars.len() as f32 * 1.4).clamp(0.0, 1.0)
    }

    /// The dancing puppy (Gil's build only — it replaced the removed visualizer):
    /// the real dog brandmark bobs + scales to the music. Mini version, drawn in
    /// the playbar strip where the spectrum used to be.
    fn draw_viz(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        self.update_viz_levels(28);
        let e = self.viz_energy();
        if let Some(logo) = &self.logo {
            let sz = rect.height() * (0.9 + 0.18 * e);
            let bob = e * rect.height() * 0.22;
            let center = egui::pos2(rect.center().x, rect.bottom() - sz * 0.5 - bob);
            let r = egui::Rect::from_center_size(center, egui::vec2(sz, sz));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(logo.id(), r, uv, Color32::WHITE);
        } else {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "🐶", egui::FontId::proportional(rect.height() * 0.8), TEXT);
        }
    }

    /// Fullscreen dancing-puppy overlay (Gil only — replaced the visualizer):
    /// big bobbing dog over black, now-playing text bottom-center, ✕/Esc/V close.
    fn full_viz_overlay(&mut self, ctx: &egui::Context, snap: &audio::Shared) {
        if !self.show_full_viz || !self.is_gil {
            return;
        }
        self.update_viz_levels(28);
        let e = self.viz_energy();
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("fullviz"))
            .fixed_pos(egui::Pos2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // eat clicks so the UI underneath can't be hit through the overlay
                let _bg = ui.interact(screen, egui::Id::new("fullviz-bg"), egui::Sense::click());
                ui.painter().rect_filled(screen, 0.0, Color32::BLACK);
                // the dancing puppy — big, bobbing + scaling to the beat
                let cx = screen.center().x;
                let cy = screen.center().y;
                let sz = screen.height() * (0.34 + 0.13 * e);
                let bob = e * screen.height() * 0.06;
                if let Some(logo) = &self.logo {
                    // soft glow disc behind the dog, pulsing with the music
                    ui.painter().circle_filled(egui::pos2(cx, cy - bob), sz * 0.62, ACCENT.gamma_multiply(0.10 + 0.18 * e));
                    let r = egui::Rect::from_center_size(egui::pos2(cx, cy - bob), egui::vec2(sz, sz));
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(logo.id(), r, uv, Color32::WHITE);
                } else {
                    ui.painter().text(egui::pos2(cx, cy - bob), egui::Align2::CENTER_CENTER, "🐶", egui::FontId::proportional(sz), Color32::WHITE);
                }
                // now-playing text, bottom-center (matches the web overlay)
                let cur = if self.follow_on { self.follow_track.clone() } else { snap.current.clone() };
                if let Some(t) = &cur {
                    ui.painter().text(
                        egui::pos2(screen.center().x, screen.bottom() - 66.0),
                        egui::Align2::CENTER_CENTER,
                        &t.title,
                        egui::FontId::proportional(26.0),
                        Color32::WHITE,
                    );
                    ui.painter().text(
                        egui::pos2(screen.center().x, screen.bottom() - 38.0),
                        egui::Align2::CENTER_CENTER,
                        &t.artist,
                        egui::FontId::proportional(16.0),
                        Color32::from_rgba_unmultiplied(255, 255, 255, 200),
                    );
                }
                // ✕ close, top-right (Esc and V work too)
                let close = egui::Rect::from_min_size(egui::pos2(screen.right() - 52.0, 16.0), egui::vec2(36.0, 36.0));
                let cresp = ui.interact(close, egui::Id::new("fullviz-x"), egui::Sense::click());
                ui.painter().text(
                    close.center(),
                    egui::Align2::CENTER_CENTER,
                    "✕",
                    egui::FontId::proportional(24.0),
                    if cresp.hovered() { Color32::WHITE } else { MUTED },
                );
                if hand(cresp).clicked() {
                    self.show_full_viz = false;
                }
            });
    }

    // -------------------- PLAYLISTS TAB --------------------
    fn playlists_view(&mut self, ui: &mut egui::Ui) {
        let data = self.pl_groups.take();
        let mut open_pl: Option<(String, String)> = None; // (rk, title)
        if let Some(groups) = &data {
            let total: usize = groups.iter().map(|g| g.playlists.len()).sum();
            ui.horizontal(|ui| {
                ui.label(RichText::new(total.to_string()).strong().color(TEXT));
                ui.label(RichText::new(format!("playlists · {} groups", groups.len())).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.hyperlink_to(RichText::new("Manage on the web ↗").size(12.0).color(MUTED), format!("{}/playlists", self.base_url));
                });
            });
            ui.add_space(8.0);
        }
        match &data {
            Some(groups) => {
                egui::ScrollArea::vertical().id_salt("plbody").auto_shrink([false, false]).show(ui, |ui| {
                    if groups.is_empty() {
                        ui.add_space(28.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("No playlists yet.").color(MUTED));
                        });
                    }
                    for g in groups {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{} · {}", g.category, g.playlists.len())).strong().color(ACCENT));
                        ui.add_space(4.0);
                        let card_w = 150.0;
                        let cols = (((ui.available_width() - 18.0) / card_w).floor() as usize).max(1);
                        for chunk in g.playlists.chunks(cols) {
                            ui.horizontal(|ui| {
                                for p in chunk {
                                    let tex = self.art.get(&p.rating_key);
                                    ui.allocate_ui(egui::vec2(card_w, 180.0), |ui| {
                                        ui.vertical(|ui| {
                                            let clicked = if let Some(tex) = &tex {
                                                hand(ui.add(img_clickable(tex, 128.0))).clicked()
                                            } else {
                                                hand(ui.add_sized([128.0, 128.0], egui::Button::new(if p.smart { "✨" } else { "🎵" }))).clicked()
                                            };
                                            let name = hand(ui.add(
                                                egui::Label::new(RichText::new(truncate(&p.title, 26)).color(TEXT)).truncate().sense(egui::Sense::click()),
                                            ))
                                            .clicked();
                                            ui.label(RichText::new(format!("{} tracks", p.track_count)).size(11.0).color(MUTED));
                                            if clicked || name {
                                                open_pl = Some((p.rating_key.clone(), p.title.clone()));
                                            }
                                        });
                                    });
                                }
                            });
                        }
                        ui.add_space(10.0);
                    }
                });
            }
            None => {
                ui.add_space(28.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Loading playlists…").color(MUTED));
                });
            }
        }
        self.pl_groups = data;
        if let Some((rk, title)) = open_pl {
            self.fetch(move |a| match a.playlist_tracks(&rk) {
                Ok(t) => DataMsg::PlayList(title.clone(), t),
                Err(e) => DataMsg::Error(e.to_string()),
            });
        }
    }

    // -------------------- HISTORY TAB --------------------
    fn history_view(&mut self, ui: &mut egui::Ui) {
        let mut reload = false;
        // grouped toolbar card — same calm grammar as the Library toolbar
        egui::Frame::none()
            .fill(CARD)
            .rounding(egui::Rounding::same(14.0))
            .inner_margin(egui::Margin::symmetric(14.0, 12.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    section_label(ui, "Range");
                    ui.add_space(2.0);
                    for (val, label) in [("today", "Today"), ("week", "Week"), ("month", "Month"), ("year", "Year"), ("all", "All")] {
                        let sel = self.hist.range == val;
                        let b = egui::Button::new(if sel { RichText::new(label).strong().color(Color32::BLACK) } else { RichText::new(label).color(MUTED) })
                            .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                            .stroke(egui::Stroke::new(1.0, if sel { ACCENT } else { CARD2 }))
                            .rounding(999.0)
                            .min_size(egui::vec2(0.0, 28.0));
                        if hand(ui.add(b)).clicked() && !sel {
                            self.hist.range = val.to_string();
                            reload = true;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if search_pill(ui, "🎤", "Search by lyrics…", &mut self.hist.lyr, 140.0) {
                            reload = true;
                        }
                        ui.add_space(8.0);
                        if search_pill(ui, "🔍", "Search title, artist, album…", &mut self.hist.q, 190.0) {
                            reload = true;
                        }
                        ui.add_space(8.0);
                        let mut sort = self.hist.sort.clone();
                        egui::ComboBox::from_id_salt("hsort")
                            .selected_text(if sort == "most" { "Most played" } else { "Recent" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut sort, "recent".to_string(), "Recent");
                                ui.selectable_value(&mut sort, "most".to_string(), "Most played");
                            });
                        if sort != self.hist.sort {
                            self.hist.sort = sort;
                            reload = true;
                        }
                    });
                });
            });
        ui.add_space(10.0);
        // count summary under the toolbar (same shape as the Library count line)
        if let Some(s) = &self.hist_stats {
            let since = s.since.map(fmt_date).unwrap_or_else(|| "—".into());
            ui.horizontal(|ui| {
                ui.label(RichText::new(s.total_plays.to_string()).strong().color(TEXT));
                ui.label(RichText::new(format!("plays · {} distinct tracks · since {}", s.distinct_tracks, since)).color(MUTED));
                if !self.hist.lyr.trim().is_empty() {
                    ui.label(RichText::new("· 🎤 lyric search only covers songs you've played").size(11.5).color(FAINT));
                }
            });
            ui.add_space(6.0);
        }
        let data = self.hist_results.take();
        match &data {
            Some(v) if v.is_empty() => {
                ui.add_space(28.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No plays match.").color(MUTED));
                });
            }
            Some(v) => {
                self.render_track_rows(ui, v, false, None);
            }
            None => {
                ui.add_space(28.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Loading history…").color(MUTED));
                });
            }
        }
        self.hist_results = data;
        if reload {
            self.reload_history();
        }
    }

    /// Virtualized track list (only visible rows are built + fetch art), owning
    /// its own ScrollArea so it stays light for thousand-row lists.
    fn render_track_rows(&mut self, ui: &mut egui::Ui, tracks: &[Track], add_mode: bool, jump_to: Option<usize>) {
        let tx = self.audio.tx.clone();
        let api = self.api.clone();
        let dtx = self.data_tx.clone();
        let ctx = self.ctx.clone();
        let sender = self.user_label.clone();
        let mut nav_artist: Option<String> = None;
        let mut nav_album: Option<Track> = None;
        let mut share_copied: Option<String> = None;
        let mut sa = egui::ScrollArea::vertical().id_salt("trackrows").auto_shrink([false, false]);
        if let Some(i) = jump_to {
            sa = sa.vertical_scroll_offset(i as f32 * (54.0 + ui.spacing().item_spacing.y));
        }
        sa.show_rows(ui, 54.0, tracks.len(), |ui, range| {
                for i in range {
                    let t = &tracks[i];
                    let tex = self.art.get(&t.rating_key);
                    row_frame(false).show(ui, |ui| {
                        // whole-row click target (so right-click works anywhere on the row)
                        let resp = ui
                            .horizontal(|ui| {
                                ui.set_min_width(ui.available_width());
                                ui.set_min_height(40.0);
                                if let Some(tex) = &tex {
                                    ui.add(img(tex, 38.0));
                                }
                                ui.add(egui::Label::new(two_line_job(&t.title, &t.artist, false)).selectable(false));
                                if t.ts.is_some() || t.plays.unwrap_or(0) > 1 {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if let Some(ts) = t.ts {
                                            ui.label(RichText::new(rel_time(ts)).size(11.0).color(MUTED));
                                        }
                                        if t.plays.unwrap_or(0) > 1 {
                                            ui.label(RichText::new(format!("{}×", t.plays.unwrap_or(0))).size(11.0).color(ACCENT));
                                        }
                                    });
                                }
                            })
                            .response
                            .interact(egui::Sense::click());
                        let resp = hand(resp).on_hover_text(if add_mode {
                            "Click to add to queue · right-click for more"
                        } else {
                            "Click to play · right-click for more"
                        });
                        if resp.clicked() {
                            if add_mode {
                                let _ = tx.send(Cmd::EnqueueEnd(vec![t.clone()]));
                            } else {
                                let _ = tx.send(Cmd::SetQueue { tracks: tracks.to_vec(), start: i });
                            }
                        }
                        resp.context_menu(|ui| {
                            if ui.button("▶  Play now").clicked() {
                                let _ = tx.send(Cmd::SetQueue { tracks: tracks.to_vec(), start: i });
                                ui.close_menu();
                            }
                            if ui.button("⏭  Play next").clicked() {
                                let _ = tx.send(Cmd::PlayNext(t.clone()));
                                ui.close_menu();
                            }
                            if ui.button("➕  Add to queue").clicked() {
                                let _ = tx.send(Cmd::EnqueueEnd(vec![t.clone()]));
                                ui.close_menu();
                            }
                            if ui.button("📻  Start radio").clicked() {
                                spawn_radio(&api, &dtx, &ctx, t.clone(), RadioKind::PlaySeed);
                                ui.close_menu();
                            }
                            if ui.button("♥  Heart").clicked() {
                                api.rate(&t.rating_key, 10.0);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("🎤  Go to artist").clicked() {
                                nav_artist = Some(t.artist.clone());
                                ui.close_menu();
                            }
                            if !t.album.is_empty() && ui.button("💿  Go to album").clicked() {
                                nav_album = Some(t.clone());
                                ui.close_menu();
                            }
                            ui.menu_button("🔗  Send to a friend", |ui| {
                                for (label, base) in friends() {
                                    if ui.button(&label).clicked() {
                                        let url = format!("{base}/player#share=t.{}.{}", t.rating_key, share_sender(&sender));
                                        ui.ctx().copy_text(url);
                                        share_copied = Some(label);
                                        ui.close_menu();
                                    }
                                }
                            });
                        });
                    });
                }
            });
        if let Some(a) = nav_artist {
            self.open_artist(a);
        }
        if let Some(t) = nav_album {
            self.goto_album(&t);
        }
        if let Some(name) = share_copied {
            self.toast = format!("🔗 Link for {name} copied — paste it to them");
        }
    }

    /// Virtualized album grid (only visible rows build + fetch art).
    fn render_albums(&mut self, ui: &mut egui::Ui, albums: &[Album], jump_to: Option<usize>) {
        // responsive grid: cards shrink a little as the window narrows, then
        // drop a column — instead of overflowing or wasting width
        let avail = (ui.available_width() - 18.0).max(130.0);
        let cols = ((avail / 150.0).floor() as usize).max(1);
        let card_w = (avail / cols as f32 - ui.spacing().item_spacing.x).clamp(124.0, 176.0);
        let img_s = card_w - 14.0;
        let card_h = img_s + 64.0;
        let nrows = (albums.len() + cols - 1) / cols;
        let mut open: Option<String> = None;
        let mut menu_act: Option<(u8, String, String)> = None; // (action, prk, title)
        let mut sa = egui::ScrollArea::vertical().id_salt("albgrid").auto_shrink([false, false]);
        if let Some(i) = jump_to {
            sa = sa.vertical_scroll_offset((i / cols) as f32 * (card_h + ui.spacing().item_spacing.y));
        }
        sa.show_rows(ui, card_h, nrows, |ui, range| {
                for r in range {
                    ui.horizontal(|ui| {
                        for c in 0..cols {
                            let idx = r * cols + c;
                            if idx >= albums.len() {
                                break;
                            }
                            let a = &albums[idx];
                            let tex = self.art.get(&a.parent_rating_key);
                            ui.allocate_ui(egui::vec2(card_w, card_h), |ui| {
                                ui.vertical(|ui| {
                                    let r = if let Some(tex) = &tex {
                                        hand(ui.add(img_clickable(tex, img_s)))
                                    } else {
                                        hand(ui.add_sized([img_s, img_s], egui::Button::new("♪")))
                                    };
                                    r.context_menu(|ui| {
                                        for (act, lbl) in [(0u8, "▶  Play album"), (3, "🔀  Shuffle album"), (1, "⏭  Play next"), (2, "＋  Add to queue")] {
                                            if ui.button(lbl).clicked() {
                                                menu_act = Some((act, a.parent_rating_key.clone(), a.album.clone()));
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                    let clicked = r.clicked();
                                    let name = hand(ui.add(
                                        egui::Label::new(RichText::new(truncate(&a.album, 28)).color(TEXT)).truncate().sense(egui::Sense::click()),
                                    ))
                                    .clicked();
                                    let yr = a.year.map(|y| y.to_string()).unwrap_or_default();
                                    ui.label(RichText::new(format!("{}  {}", truncate(&a.artist, 20), yr)).size(11.5).color(MUTED));
                                    if clicked || name {
                                        open = Some(a.parent_rating_key.clone());
                                    }
                                });
                            });
                        }
                    });
                }
            });
        if let Some(prk) = open {
            self.open_album(prk);
        }
        if let Some((act, prk, title)) = menu_act {
            self.toast = format!("Getting “{title}”…");
            self.fetch(move |a| match a.album_tracks(&prk) {
                Ok((al, _ar, tr)) => DataMsg::QueueTracks(act, al, tr),
                Err(e) => DataMsg::Error(e.to_string()),
            });
        }
    }

    fn render_artists(&mut self, ui: &mut egui::Ui, artists: &[Artist], jump_to: Option<usize>) {
        // album-style card grid (web feedback): square tiles + name + count.
        // The server now returns a cover `thumb` per artist, so tiles show a real
        // cover (web parity); a colored monogram is the fallback until art loads
        // or when an artist has no artwork.
        let avail = (ui.available_width() - 18.0).max(130.0);
        let cols = ((avail / 150.0).floor() as usize).max(1);
        let card_w = (avail / cols as f32 - ui.spacing().item_spacing.x).clamp(124.0, 176.0);
        let img_s = card_w - 14.0;
        let card_h = img_s + 50.0;
        let nrows = (artists.len() + cols - 1) / cols;
        let mut open: Option<String> = None;
        let mut menu_act: Option<(u8, String)> = None; // (action, artist)
        let mut sa = egui::ScrollArea::vertical().id_salt("artlist").auto_shrink([false, false]);
        if let Some(i) = jump_to {
            sa = sa.vertical_scroll_offset((i / cols) as f32 * (card_h + ui.spacing().item_spacing.y));
        }
        sa.show_rows(ui, card_h, nrows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    for c in 0..cols {
                        let idx = r * cols + c;
                        if idx >= artists.len() {
                            break;
                        }
                        let a = &artists[idx];
                        ui.allocate_ui(egui::vec2(card_w, card_h), |ui| {
                            ui.vertical(|ui| {
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(img_s, img_s), egui::Sense::click());
                                let tex = a
                                    .thumb
                                    .as_deref()
                                    .map(art_rk)
                                    .filter(|s| !s.is_empty())
                                    .and_then(|rk| self.art.get(rk));
                                if let Some(tex) = &tex {
                                    img(tex, img_s).paint_at(ui, rect);
                                } else {
                                    let p = ui.painter();
                                    p.rect_filled(rect, 10.0, monogram_color(&a.name));
                                    let initial = a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "♪".into());
                                    p.text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        initial,
                                        egui::FontId::proportional(img_s * 0.42),
                                        Color32::from_rgba_unmultiplied(255, 255, 255, 230),
                                    );
                                }
                                let resp = hand(resp);
                                resp.context_menu(|ui| {
                                    for (act, lbl) in [(0u8, "▶  Play everything"), (3, "🔀  Shuffle everything"), (2, "＋  Add all to queue")] {
                                        if ui.button(lbl).clicked() {
                                            menu_act = Some((act, a.name.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                });
                                let clicked = resp.clicked();
                                let name = hand(ui.add(
                                    egui::Label::new(RichText::new(truncate(&a.name, 24)).color(TEXT)).truncate().sense(egui::Sense::click()),
                                ))
                                .clicked();
                                ui.label(RichText::new(a.count.to_string()).size(11.5).color(MUTED));
                                if clicked || name {
                                    open = Some(a.name.clone());
                                }
                            });
                        });
                    }
                });
            }
        });
        if let Some(name) = open {
            self.open_artist(name);
        }
        if let Some((act, name)) = menu_act {
            self.toast = format!("Gathering everything by {name}…");
            self.fetch(move |a| {
                let albums = match a.albums_by_artist(&name) {
                    Ok(v) => v,
                    Err(e) => return DataMsg::Error(e.to_string()),
                };
                let mut tracks: Vec<Track> = Vec::new();
                for alb in &albums {
                    if let Ok((_, _, t)) = a.album_tracks(&alb.parent_rating_key) {
                        tracks.extend(t);
                    }
                }
                DataMsg::QueueTracks(act, format!("everything by {name}"), tracks)
            });
        }
    }

    // -------------------- BOTTOM PLAYBAR --------------------
    fn play_bar(&mut self, ctx: &egui::Context, snap: &audio::Shared) {
        let following = self.follow_on;
        let cur = if following { self.follow_track.clone() } else { snap.current.clone() };
        let position = if following { (self.follow_pos_ms() / 1000.0) as f32 } else { snap.position };
        let duration = if following { (self.follow_dur_ms / 1000.0) as f32 } else { snap.duration };
        let playing = if following { self.follow_playing } else { snap.playing };
        egui::TopBottomPanel::bottom("playbar")
            .exact_height(124.0)
            .frame(egui::Frame::none().fill(Color32::from_rgb(22, 22, 26)).inner_margin(egui::Margin::symmetric(18.0, 8.0)))
            .show(ctx, |ui| {
                ui.columns(3, |c| {
                    // LEFT — cover + meta + heart/stars
                    {
                        let ui = &mut c[0];
                        ui.horizontal(|ui| {
                            if let Some(t) = &cur {
                                if let Some(tex) = self.art.get(&t.rating_key) {
                                    ui.add(img(&tex, 54.0));
                                }
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(truncate(&t.title, 28)).strong().color(TEXT));
                                    ui.label(RichText::new(truncate(&t.artist, 28)).size(12.0).color(MUTED));
                                    ui.horizontal(|ui| {
                                        let hearted = self.np_rating >= 9.5;
                                        let hc = if hearted { ACCENT } else { MUTED };
                                        let hg = if hearted { "♥" } else { "♡" };
                                        if hand(ui.add(egui::Button::new(RichText::new(hg).color(hc)).frame(false))).clicked() {
                                            let nr = if hearted { 0.0 } else { 10.0 };
                                            self.api.rate(&t.rating_key, nr);
                                            self.np_rating = nr;
                                        }
                                        let filled = (self.np_rating / 2.0).round() as i32;
                                        for s in 1..=5 {
                                            let star = if s <= filled { "★" } else { "☆" };
                                            let col = if s <= filled { GOLD } else { MUTED };
                                            if hand(ui.add(egui::Button::new(RichText::new(star).color(col)).frame(false))).clicked() {
                                                let nr = (s as f32) * 2.0;
                                                self.api.rate(&t.rating_key, nr);
                                                self.np_rating = nr;
                                            }
                                        }
                                    });
                                });
                            } else {
                                ui.label(RichText::new(if following { "📱 Phone idle" } else { "Nothing playing" }).color(MUTED));
                            }
                        });
                    }
                    // CENTER — transport, emphasized + truly centered
                    {
                        let ui = &mut c[1];
                        ui.vertical_centered(|ui| {
                            egui::Frame::none()
                                .fill(Color32::from_rgb(31, 31, 41))
                                .rounding(egui::Rounding::same(34.0))
                                .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if hand(ui.add(round_btn("🔀", 34.0, CARD2))).on_hover_text("Shuffle the rest of the queue").clicked() && !following {
                                            self.play(Cmd::Shuffle);
                                        }
                                        ui.add_space(6.0);
                                        if hand(ui.add(round_btn("⏮", 44.0, CARD2))).clicked() {
                                            if following { self.follow_control("previous"); } else { self.play(Cmd::Prev); }
                                        }
                                        ui.add_space(8.0);
                                        let sym = if playing { "⏸" } else { "▶" };
                                        if hand(ui.add(round_btn(sym, 62.0, ACCENT))).clicked() {
                                            if following { self.follow_control(if playing { "pause" } else { "play" }); } else { self.play(Cmd::Toggle); }
                                        }
                                        ui.add_space(8.0);
                                        if hand(ui.add(round_btn("⏭", 44.0, CARD2))).clicked() {
                                            if following { self.follow_control("next"); } else { self.play(Cmd::Next); }
                                        }
                                        ui.add_space(6.0);
                                        let (rsym, rfill) = match snap.repeat {
                                            1 => ("🔁", ACCENT),
                                            2 => ("🔂", ACCENT),
                                            _ => ("🔁", CARD2),
                                        };
                                        let rtip = match snap.repeat {
                                            1 => "Repeat: all",
                                            2 => "Repeat: one",
                                            _ => "Repeat: off",
                                        };
                                        if hand(ui.add(round_btn(rsym, 34.0, rfill))).on_hover_text(rtip).clicked() && !following {
                                            self.play(Cmd::CycleRepeat);
                                        }
                                    });
                                });
                        });
                    }
                    // RIGHT — balance preset + volume
                    {
                        let ui = &mut c[2];
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // ⊞ Tools (web's fixed corner button) — opens an
                            // overlay; links launch the browser, music keeps playing
                            let tools = egui::Button::new(RichText::new("⊞").size(17.0).color(TEXT))
                                .fill(Color32::from_rgb(36, 36, 46))
                                .rounding(10.0)
                                .min_size(egui::vec2(34.0, 30.0));
                            if hand(ui.add(tools)).on_hover_text("More from tulik — pages & tools").clicked() {
                                self.show_links = !self.show_links;
                            }
                            ui.add_space(8.0);
                            let mut vol = snap.volume;
                            ui.style_mut().spacing.slider_width = 96.0;
                            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).changed() {
                                self.play(Cmd::SetVolume(vol));
                            }
                            let vico = if snap.volume <= 0.001 { "🔇" } else { "🔊" };
                            let vr = ui.add(egui::Button::new(RichText::new(vico).size(16.0)).frame(false));
                            if hand(vr).on_hover_text("Mute / unmute (M)").clicked() {
                                if let Some(v) = self.pre_mute_vol.take() {
                                    self.play(Cmd::SetVolume(v));
                                } else {
                                    self.pre_mute_vol = Some(snap.volume);
                                    self.play(Cmd::SetVolume(0.0));
                                }
                            }
                            ui.add_space(8.0);
                            let bn = bal_name(self.bal_active);
                            let bfill = if self.bal_active == 0 { CARD2 } else { ACCENT };
                            let bcol = if self.bal_active == 0 { TEXT } else { Color32::BLACK };
                            if hand(ui.add(egui::Button::new(RichText::new(format!("⚖ {bn}")).color(bcol)).fill(bfill).rounding(12.0)))
                                .on_hover_text("L/R balance — cycle 50/50 → A → B (set A/B in 🎚 EQ)")
                                .clicked()
                            {
                                self.bal_active = (self.bal_active + 1) % 3;
                                let b = self.bal_presets[self.bal_active];
                                self.set_balance(b);
                                self.toast = format!("Balance: {}", bal_name(self.bal_active));
                            }
                            if self.show_viz && self.is_gil {
                                ui.add_space(6.0);
                                let (vrect, vresp) = ui.allocate_exact_size(egui::vec2(60.0, 30.0), egui::Sense::click());
                                ui.painter().rect_filled(vrect, 6.0, Color32::from_rgb(12, 12, 14));
                                self.draw_viz(ui, vrect.shrink(3.0));
                                if hand(vresp).on_hover_text("Open the dancing puppy 🐶 (V)").clicked() {
                                    self.show_full_viz = true;
                                }
                            }
                        });
                    }
                });

                ui.horizontal(|ui| {
                    ui.label(RichText::new(fmt_time(position)).size(12.0).color(MUTED));
                    let dur = duration.max(0.1);
                    let mut pos = self.seek_drag.unwrap_or(position).min(dur);
                    let want = ui.available_width() - 56.0;
                    ui.style_mut().spacing.slider_width = want.max(80.0);
                    let resp = ui.add(egui::Slider::new(&mut pos, 0.0..=dur).show_value(false).trailing_fill(true));
                    if !following {
                        if resp.dragged() {
                            self.seek_drag = Some(pos);
                        }
                        if resp.drag_stopped() {
                            self.play(Cmd::Seek(pos));
                            self.seek_drag = None;
                        }
                    }
                    ui.label(RichText::new(fmt_time(duration)).size(12.0).color(MUTED));
                });
            });
    }

    /// Big, easy-to-grab resize handles in the bottom corners (the OS border is
    /// thin and hard to catch). Uses native BeginResize so it feels real.
    fn resize_grips(&self, ctx: &egui::Context) {
        for (id, dir, align, cursor) in [
            ("grip-se", egui::ResizeDirection::SouthEast, egui::Align2::RIGHT_BOTTOM, egui::CursorIcon::ResizeNwSe),
            ("grip-sw", egui::ResizeDirection::SouthWest, egui::Align2::LEFT_BOTTOM, egui::CursorIcon::ResizeNeSw),
        ] {
            egui::Area::new(egui::Id::new(id))
                .anchor(align, egui::vec2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(true)
                .show(ctx, |ui| {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::drag());
                    let resp = resp.on_hover_cursor(cursor);
                    let p = ui.painter();
                    let c = if resp.hovered() { ACCENT } else { MUTED };
                    if dir == egui::ResizeDirection::SouthEast {
                        for k in 1..=3 {
                            let o = k as f32 * 5.0;
                            p.line_segment(
                                [egui::pos2(rect.right() - o, rect.bottom() - 2.0), egui::pos2(rect.right() - 2.0, rect.bottom() - o)],
                                egui::Stroke::new(1.5, c),
                            );
                        }
                    } else {
                        for k in 1..=3 {
                            let o = k as f32 * 5.0;
                            p.line_segment(
                                [egui::pos2(rect.left() + 2.0, rect.bottom() - o), egui::pos2(rect.left() + o, rect.bottom() - 2.0)],
                                egui::Stroke::new(1.5, c),
                            );
                        }
                    }
                    if resp.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                    }
                });
        }
    }
}

// ----------------------------- album art cache -----------------------------

enum ArtState {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

const ART_WORKERS: usize = 6;
const ART_CAP: usize = 320; // max textures kept in memory (FIFO eviction)

/// Album-art cache with a BOUNDED worker pool (so a 2000-album view can't spawn
/// thousands of threads/HTTP requests) and a FIFO texture cap (bounded memory).
/// Combined with virtualized lists (only visible items call `get`), this keeps
/// the native client light no matter how big the library is.
struct ArtCache {
    map: HashMap<String, ArtState>,
    order: std::collections::VecDeque<String>, // ready-texture insertion order
    avg: HashMap<String, Color32>,             // mean cover color (drives the glow)
    req_tx: Sender<String>,
    res_rx: Receiver<(String, Option<egui::ColorImage>)>,
}

impl ArtCache {
    fn new(api: ApiClient, ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<String>();
        let (res_tx, res_rx) = channel::<(String, Option<egui::ColorImage>)>();
        let req_rx = std::sync::Arc::new(std::sync::Mutex::new(req_rx));
        for _ in 0..ART_WORKERS {
            let req_rx = req_rx.clone();
            let res_tx = res_tx.clone();
            let api = api.clone();
            let ctx = ctx.clone();
            thread::spawn(move || loop {
                let rk = {
                    let guard = req_rx.lock().unwrap();
                    guard.recv()
                };
                match rk {
                    Ok(rk) => {
                        let img = api.art_bytes(&rk).ok().and_then(|b| decode_art(&b));
                        let _ = res_tx.send((rk, img));
                        ctx.request_repaint();
                    }
                    Err(_) => break,
                }
            });
        }
        ArtCache {
            map: HashMap::new(),
            order: std::collections::VecDeque::new(),
            avg: HashMap::new(),
            req_tx,
            res_rx,
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((rk, img)) = self.res_rx.try_recv() {
            let st = match img {
                Some(ci) => {
                    self.order.push_back(rk.clone());
                    self.avg.insert(rk.clone(), theme_tint(avg_color(&ci)));
                    ArtState::Ready(ctx.load_texture(format!("art-{rk}"), ci, egui::TextureOptions::LINEAR))
                }
                None => ArtState::Failed,
            };
            self.map.insert(rk, st);
        }
        // evict oldest ready textures beyond the cap
        while self.order.len() > ART_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
                self.avg.remove(&old);
            }
        }
    }

    fn get(&mut self, rk: &str) -> Option<egui::TextureHandle> {
        if rk.is_empty() {
            return None;
        }
        match self.map.get(rk) {
            Some(ArtState::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                self.map.insert(rk.to_string(), ArtState::Loading);
                let _ = self.req_tx.send(rk.to_string());
                None
            }
        }
    }

    /// Average color of a cached cover (drives the now-playing glow).
    fn avg(&self, rk: &str) -> Option<Color32> {
        self.avg.get(rk).copied()
    }
}

/// Mean color of an image (subsampled) — the same trick the web player uses
/// to tint its cover glow.
fn avg_color(ci: &egui::ColorImage) -> Color32 {
    let px = &ci.pixels;
    if px.is_empty() {
        return ACCENT;
    }
    let step = (px.len() / 256).max(1);
    let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
    let mut i = 0;
    while i < px.len() {
        let c = px[i];
        r += c.r() as u32;
        g += c.g() as u32;
        b += c.b() as u32;
        n += 1;
        i += step;
    }
    Color32::from_rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

fn decode_art(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.thumbnail(280, 280);
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw()))
}

// ----------------------------- ui helpers -----------------------------

/// Encode an egui framebuffer screenshot to PNG bytes (feedback attach).
fn colorimage_to_png(img: &egui::ColorImage) -> Option<Vec<u8>> {
    let [w, h] = img.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in &img.pixels {
        rgba.extend_from_slice(&[p.r(), p.g(), p.b(), p.a()]);
    }
    let buf = image::RgbaImage::from_raw(w as u32, h as u32, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some(png.into_inner())
}

/// Read an image from the system clipboard as PNG bytes — the native answer to the
/// web form's "paste a screenshot (Ctrl+V)". Returns (png, width, height).
fn clipboard_image_png() -> Option<(Vec<u8>, u32, u32)> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;
    let (w, h) = (img.width as u32, img.height as u32);
    let buf = image::RgbaImage::from_raw(w, h, img.bytes.into_owned())?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some((png.into_inner(), w, h))
}

fn setup_theme(ctx: &egui::Context) {
    // Typeface parity with the web player (it loads "DM Sans"). Prepend DM Sans to
    // the proportional family but KEEP egui's default fallbacks after it, so emoji
    // and symbol glyphs (🐞 ⚡ 🌙 ⊞ ♥ …) still resolve via the fallback chain.
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "dmsans".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/DMSans.ttf")),
    );
    if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        prop.insert(0, "dmsans".to_owned());
    }
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(10, 10, 12);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(74, 54, 16); // dark gold text-selection
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);

    let r = egui::Rounding::same(10.0);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = r;
    }
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.inactive.bg_fill = CARD2;
    v.widgets.inactive.weak_bg_fill = CARD2;
    v.widgets.hovered.bg_fill = Color32::from_rgb(42, 42, 52); // neutral hover (web --hover)
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(42, 42, 52);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = ACCENT;
    ctx.set_visuals(v);

    use egui::{FontFamily::Proportional, FontId, TextStyle};
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(24.0, Proportional)),
        (TextStyle::Body, FontId::new(15.0, Proportional)),
        (TextStyle::Button, FontId::new(15.0, Proportional)),
        (TextStyle::Small, FontId::new(12.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, egui::FontFamily::Monospace)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    ctx.set_style(style);
}

fn card(fill: Color32) -> egui::Frame {
    egui::Frame::none().fill(fill).rounding(egui::Rounding::same(16.0)).inner_margin(egui::Margin::same(16.0))
}

fn row_frame(active: bool) -> egui::Frame {
    let fill = if active { CARD2 } else { Color32::TRANSPARENT };
    egui::Frame::none().fill(fill).rounding(egui::Rounding::same(10.0)).inner_margin(egui::Margin::symmetric(8.0, 5.0))
}

fn pill_tab(ui: &mut egui::Ui, tab: &mut Tab, this: Tab, label: &str) {
    let selected = *tab == this;
    let txt = if selected { RichText::new(label).strong().color(TEXT) } else { RichText::new(label).color(MUTED) };
    let fill = if selected { CARD2 } else { Color32::TRANSPARENT };
    let b = egui::Button::new(txt).fill(fill).rounding(14.0).min_size(egui::vec2(0.0, 30.0));
    if hand(ui.add(b)).clicked() {
        *tab = this;
    }
}

fn badge_chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::none()
        .fill(CARD2)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(MUTED));
        });
}

/// Small uppercase, faint section header (e.g. "FOCUS", "QUICK PICKS").
fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(10.5).color(FAINT).strong());
}

/// A rounded "pill" search field with a leading icon. Returns true when the
/// user submits (presses Enter). Draws its own background so it reads as one
/// calm capsule instead of a bare egui text box.
fn search_pill(ui: &mut egui::Ui, icon: &str, hint: &str, text: &mut String, width: f32) -> bool {
    let mut submitted = false;
    egui::Frame::none()
        .fill(BG)
        .rounding(egui::Rounding::same(999.0))
        .stroke(egui::Stroke::new(1.0, CARD2))
        .inner_margin(egui::Margin::symmetric(12.0, 6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(12.0).color(MUTED));
                let r = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .desired_width(width)
                        .frame(false),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submitted = true;
                }
            });
        });
    submitted
}

/// First-letter index for the A–Z rail: (letter, first item index) in the
/// web's order (A–Z then '#'), only for letters actually present.
fn az_index<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(char, usize)> {
    let mut first: HashMap<char, usize> = HashMap::new();
    for (i, name) in names.enumerate() {
        let l = name
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .filter(|c| c.is_ascii_alphabetic())
            .unwrap_or('#');
        first.entry(l).or_insert(i);
    }
    let mut out = Vec::new();
    for l in ('A'..='Z').chain(std::iter::once('#')) {
        if let Some(&i) = first.get(&l) {
            out.push((l, i));
        }
    }
    out
}

/// The vertical A–Z jump rail (web parity). Returns a clicked item index.
fn az_rail(ui: &mut egui::Ui, letters: &[(char, usize)]) -> Option<usize> {
    let mut jump = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let lh = 13.0;
        let pad = ((ui.available_height() - letters.len() as f32 * lh) * 0.5).max(0.0);
        ui.add_space(pad);
        for (l, idx) in letters {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, lh), egui::Sense::click());
            let resp = hand(resp);
            if resp.hovered() {
                ui.painter().rect_filled(rect, 3.0, CARD2);
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                l.to_string(),
                egui::FontId::proportional(9.5),
                if resp.hovered() { ACCENT } else { FAINT },
            );
            if resp.clicked() {
                jump = Some(*idx);
            }
        }
    });
    jump
}

/// One removable filter chip (accent capsule with an × ). Returns true if the
/// × was clicked (caller should drop that facet + reload).
fn filter_chip(ui: &mut egui::Ui, label: &str) -> bool {
    let mut removed = false;
    egui::Frame::none()
        .fill(CARD2)
        .rounding(egui::Rounding::same(999.0))
        .stroke(egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.5)))
        .inner_margin(egui::Margin::symmetric(10.0, 3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(label).size(11.5).color(ACCENT));
                if hand(ui.add(egui::Button::new(RichText::new("×").size(12.0).color(MUTED)).frame(false))).clicked() {
                    removed = true;
                }
            });
        });
    removed
}

/// Gil's rule: covers whose average color lands in the yellow/olive band
/// tint the UI muddy — remap those to a nice muted red. Every UI consumer of
/// a cover-derived color must pass through this (the glow, and any theming
/// that reads `ArtCache::avg`).
fn theme_tint(c: Color32) -> Color32 {
    const MUTED_RED: Color32 = Color32::from_rgb(172, 84, 82);
    let (r, g, b) = (c.r() as f32, c.g() as f32, c.b() as f32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max <= 0.0 {
        return c;
    }
    let sat = (max - min) / max;
    if sat < 0.12 {
        return c; // near-gray: hue is meaningless, leave it neutral
    }
    let d = max - min;
    let hue = if (max - r).abs() < f32::EPSILON {
        60.0 * (((g - b) / d) % 6.0)
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    if (42.0..100.0).contains(&hue) {
        MUTED_RED
    } else {
        c
    }
}

/// Deterministic, theme-friendly tile color for an artist monogram.
/// Pull the rating_key out of an `/api/art/{rk}` thumb URL (for the art cache).
fn art_rk(thumb: &str) -> &str {
    thumb.rsplit('/').next().unwrap_or("")
}

/// Sanitize the share "from" label for a `#share=` URL fragment.
fn share_sender(s: &str) -> String {
    let out: String = s.chars().filter(|c| c.is_alphanumeric()).collect();
    if out.is_empty() {
        "Friend".into()
    } else {
        out
    }
}

fn monogram_color(name: &str) -> Color32 {
    const PAL: [Color32; 8] = [
        Color32::from_rgb(92, 76, 170),
        Color32::from_rgb(60, 110, 150),
        Color32::from_rgb(150, 90, 60),
        Color32::from_rgb(70, 130, 100),
        Color32::from_rgb(150, 70, 110),
        Color32::from_rgb(120, 110, 60),
        Color32::from_rgb(80, 90, 140),
        Color32::from_rgb(130, 80, 80),
    ];
    let h: usize = name.bytes().map(|b| b as usize).sum();
    PAL[h % PAL.len()]
}

/// Gold uppercase section header inside the Help overlay (web parity).
fn help_section(ui: &mut egui::Ui, label: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(label).size(11.0).strong().color(GOLD));
    ui.add_space(6.0);
}

/// One Help card: icon chip + bold title + muted description — matches the
/// web Help overlay's card grid.
fn help_card(ui: &mut egui::Ui, w: f32, icon: &str, title: &str, desc: &str) {
    egui::Frame::none()
        .fill(CARD2.gamma_multiply(0.55))
        .rounding(egui::Rounding::same(12.0))
        .stroke(egui::Stroke::new(1.0, CARD2))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(w);
            ui.horizontal_top(|ui| {
                egui::Frame::none()
                    .fill(CARD)
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(6.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(icon).size(14.0));
                    });
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong().color(TEXT));
                    ui.label(RichText::new(desc).size(12.0).color(MUTED));
                });
            });
        });
}

/// Outlined (ghost) pill button — calm, low-emphasis. For the quick-pick row.
fn ghost_pill(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let b = egui::Button::new(RichText::new(label).size(12.5).color(MUTED))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, CARD2))
        .rounding(999.0)
        .min_size(egui::vec2(0.0, 28.0));
    hand(ui.add(b))
}

fn round_btn(sym: &str, size: f32, fill: Color32) -> egui::Button<'static> {
    let col = if fill == ACCENT { Color32::BLACK } else { TEXT };
    egui::Button::new(RichText::new(sym).size(size * 0.42).color(col))
        .fill(fill)
        .rounding(size / 2.0)
        .min_size(egui::vec2(size, size))
}

fn two_line_btn(line1: &str, line2: &str, active: bool) -> egui::Button<'static> {
    let mut job = egui::text::LayoutJob::default();
    let c1 = if active { ACCENT } else { TEXT };
    job.append(line1, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(14.5), color: c1, ..Default::default() });
    job.append(&format!("\n{line2}"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: MUTED, ..Default::default() });
    egui::Button::new(job).frame(false)
}

fn track_two_lines(t: &Track) -> (String, String) {
    (truncate(&t.title, 36), truncate(&t.artist, 36))
}

/// Two-line (title + artist) text block for a non-button row label.
fn two_line_job(title: &str, artist: &str, active: bool) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let c1 = if active { ACCENT } else { TEXT };
    job.append(&truncate(title, 40), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(14.5), color: c1, ..Default::default() });
    job.append(&format!("\n{}", truncate(artist, 40)), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: MUTED, ..Default::default() });
    job
}

fn img(tex: &egui::TextureHandle, size: f32) -> egui::Image<'static> {
    egui::Image::new(egui::load::SizedTexture::from_handle(tex)).fit_to_exact_size(egui::vec2(size, size)).rounding(egui::Rounding::same(8.0))
}

fn img_clickable(tex: &egui::TextureHandle, size: f32) -> egui::ImageButton<'static> {
    let st = egui::load::SizedTexture::new(tex.id(), egui::vec2(size, size));
    egui::ImageButton::new(st).frame(false)
}

#[derive(Clone, Copy)]
enum RadioKind {
    /// Keep the currently-playing track, replace everything below it. (now-playing CTA)
    ReplaceRest,
    /// Play the seed track, then the similar tracks. (right-click a song)
    PlaySeed,
}

fn spawn_radio(api: &ApiClient, dtx: &Sender<DataMsg>, ctx: &egui::Context, seed: Track, kind: RadioKind) {
    let api = api.clone();
    let dtx = dtx.clone();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let msg = match api.similar(&seed.rating_key) {
            Ok(v) => match kind {
                RadioKind::ReplaceRest => DataMsg::RadioReplace(v),
                RadioKind::PlaySeed => {
                    let mut q = Vec::with_capacity(v.len() + 1);
                    q.push(seed);
                    q.extend(v);
                    DataMsg::RadioPlay(q)
                }
            },
            Err(e) => DataMsg::Error(e.to_string()),
        };
        let _ = dtx.send(msg);
        ctx.request_repaint();
    });
}

fn hand(r: egui::Response) -> egui::Response {
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn bal_name(i: usize) -> &'static str {
    match i {
        1 => "A",
        2 => "B",
        _ => "50/50",
    }
}

/// Human L/R readout for a balance value (-1 left .. +1 right).
fn bal_readout(v: f32) -> String {
    let r = (((v + 1.0) / 2.0) * 100.0).round() as i32;
    let l = 100 - r;
    if l == r {
        "50/50".into()
    } else {
        format!("L{l}/R{r}")
    }
}

fn step_pct(step: &str) -> f32 {
    match step {
        "fetching" => 12.0,
        "filtering" => 20.0,
        "preparing" => 28.0,
        "ai_working" => 50.0,
        "parsing" => 72.0,
        "matching" => 82.0,
        "narrative" => 92.0,
        "complete" => 100.0,
        _ => 8.0,
    }
}

fn sort_label(s: &str) -> &'static str {
    match s {
        "name" => "Sort: A–Z",
        "added" => "Sort: Recently added",
        "count" => "Sort: Most tracks",
        _ => "Sort: Year",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Relative time like "5m ago" / "3h ago" / a date, from a unix epoch (seconds).
fn rel_time(ts: f64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(ts);
    let s = (now - ts).max(0.0);
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        format!("{}m ago", (s / 60.0) as i64)
    } else if s < 86400.0 {
        format!("{}h ago", (s / 3600.0) as i64)
    } else if s < 7.0 * 86400.0 {
        format!("{}d ago", (s / 86400.0) as i64)
    } else {
        fmt_date(ts)
    }
}

/// Format a unix epoch (seconds) as YYYY-MM-DD (no chrono dependency).
fn fmt_date(epoch_secs: f64) -> String {
    let days = (epoch_secs / 86400.0).floor() as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// In-place iterative radix-2 FFT (n must be a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = i + k;
                let b = i + k + len / 2;
                let tr = cr * re[b] - ci * im[b];
                let ti = cr * im[b] + ci * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}

fn fmt_time(s: f32) -> String {
    if !s.is_finite() || s < 0.0 {
        return "0:00".into();
    }
    let s = s as i64;
    format!("{}:{:02}", s / 60, s % 60)
}
