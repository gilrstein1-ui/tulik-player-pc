# tulik-player-native — fully native (no-webview) Windows player

A second, **separate** PC player for MediaSage, built to compare head-to-head
against the Tauri app in this same repo. Where the Tauri app is a native *window*
wrapping the live `player.html` in **WebView2 (Chromium)**, this one has **no web
engine at all**:

- **UI:** [egui](https://github.com/emilk/egui) (immediate-mode, native)
- **Audio:** [rodio](https://github.com/RustAudio/rodio) → cpal → **WASAPI**
- Single self-contained `~10 MB .exe`, no installer, no WebView2 dependency.

The point of the comparison: native audio is never throttled when the window is
in the background, so this build should be glitch-free where a browser tab (or a
WebView) can stutter under system memory pressure.

## How it talks to the backend
It is just another API client of the MediaSage backend (`/api/player/*` +
`/api/art/*`). Each per-user build bakes **its owner's own** endpoint + basic-auth
credentials at compile time — **none of which live in this repo**. They are
injected by the CI matrix from repository **Secrets**:

- `BUILD_CONFIG` — one `variant|base_url|authuser|label` line per user (endpoint,
  auth username, friendly label, and the share-list of all users' endpoints).
- `PW_<VARIANT>` — that user's basic-auth password.

These reach the binary only via the `TULIK_BASE_URL` / `TULIK_AUTH_USER` /
`TULIK_AUTH_PW` / `TULIK_USER_LABEL` / `TULIK_SHARE_HOSTS` env vars at build time.
A plain checkout (no secrets) builds an app with an **empty endpoint and no
credentials** — it is not usable until pointed at a backend. No address, username,
or password is in source or history. See [`src/config.rs`](src/config.rs).

## Features — full parity with the web player
- **Player tab:** now-playing hero (cover, title, clickable artist→go-to-artist,
  format badge), **synced lyrics** (highlight + autoscroll), Queue (jump / remove /
  ▴▾ reorder / clear / save-as-playlist), Recently-played, and a **spectrum
  visualizer**.
- **Library tab:** Artists / Albums / Songs browse, library search + **lyric
  search**, **Focus facets** (Genre · Decade · ❤ Loved, combinable), **Sort**,
  **Add-to-queue mode**, **Quick picks** (recently added/played, most played,
  🎲 surprise), album/artist drill-in with Play-all / Shuffle / Queue-all.
- **Playlists tab:** grouped Plex playlists, click to play.
- **History tab:** text + lyric search, date-range (today/week/month/year/all),
  recent / most-played sort, stats line, per-row relative time + play counts.
- **Transport:** shuffle · prev · play/pause · next · repeat (off/all/one),
  full-width seek, volume, ♥ heart + ★ star rating.
- **🎚 EQ & audio:** native 10-band parametric EQ + L/R balance (RBJ biquads).
- **📻 Radio:** sonic-similar endless queue (bliss).
- **📡 Cast / hand-off** to any Plex device, **Pull** what's playing on Plexamp
  into the app, and **Follow** (remote-control Plexamp: mirror + transport).
- **⚡ Quick generate:** AI playlist from a line (SSE stream → play → save).
- **⏰ Sleep timer**, **💬 feedback**, **❓ help** (with browser links to the web
  player / playlists / guide / hub).
- **Keyboard:** Space play/pause · Shift+→/← next/prev · →/← seek ±5s · M mute ·
  Esc close. Pointing-hand cursor on everything; easy bottom-corner resize grips;
  dog window/taskbar/exe icon.

**Native-perf:** album art uses a bounded worker pool + FIFO texture cap, and the
album grid / track lists / queue are virtualized — a 2000-album library stays light.

**Not ported (cosmetic / N/A):** cover-glow effect, literal audio waveform on the
seek bar (a fill bar is used), A–Z scroll rail, drag-drop reorder (▴▾ buttons
instead), and the PWA "install" button (this *is* the native app).

## Layout
- `src/config.rs` — baked per-user endpoint + auth
- `src/api.rs` — typed blocking reqwest client over `/api/player/*`
- `src/audio.rs` — rodio engine on its own thread (queue, auto-advance, seek)
- `src/main.rs` — egui UI + async album-art texture cache

## Build
Cloud-built by [`.github/workflows/build-native.yml`](../.github/workflows/build-native.yml)
on `windows-latest` (the Pi has no Rust toolchain). One `.exe` per user uploaded
as an artifact, then hosted on each user's hub as `/hub/TulikPlayerNative.exe`.
To rebuild: edit under `native/` → push → `gh run watch <id>` → `gh run download`.
Per-user endpoint/auth comes from the `BUILD_CONFIG` / `PW_*` repo secrets (this
repo is public; nothing identifying is in source — see the repo README).
