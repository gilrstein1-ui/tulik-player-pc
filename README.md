# tulik player (desktop)

> **⚠️ Heads up — this builds a client for a *private, password-protected* music
> backend. It is not a standalone app and won't do anything useful on its own.**
> Each release is compiled for one specific person and points only at *their* own
> self-hosted server, which requires a login. With no server address and no
> credentials (none of which are in this repository), a build you make yourself
> just shows an empty "connecting…" window. Cloning this gives you the UI code —
> **not access to anyone's server, library, or data.**
>
> A **generic public version** that anyone can point at *their own* Plex /
> MediaSage setup is planned — it's not this build yet.

A tiny desktop wrapper for a self-hosted music player. It opens the player in its
own dedicated app window — not a browser tab — so the operating system never
throttles or suspends it the way it does background tabs. That removes the
playback stutter you get when running the player inside a busy browser.

This repo also contains a second, fully-native player under [`native/`](native/)
(egui + rodio, no web engine) — see its own README.

## How it works
- The app ships only a small "connecting…" shell (`src/index.html`) plus an icon.
- On launch, the native layer probes the configured server address(es) and, on
  the first that responds, navigates the window straight to the live player.
- From that point the window is the real player, served from the owner's server.

## What is NOT in this repository
This is the deliberate security boundary — nothing here grants access to a server:
- **No server addresses.** Endpoints are injected at build time from a private
  GitHub Actions secret (`BUILD_CONFIG`) and only ever end up inside the
  locally-downloaded installer of the person they were built for.
- **No usernames and no passwords.** Basic-auth credentials come from the private
  `BUILD_CONFIG` / `PW_*` secrets at build time; none are in the source or in the
  git history.
- **No library, listening, or personal data, and no `player.html`** — the player
  UI is loaded live from the owner's server at runtime.

## Build
Windows installers are built by GitHub Actions (`.github/workflows/build.yml`) and
can be run on demand from the Actions tab. The finished installer is uploaded as a
per-user artifact. A plain clone with no secrets configured builds an app with an
empty endpoint and no credentials.

Stack: [Tauri 2](https://tauri.app) (Rust shell + system WebView); native player is
[egui](https://github.com/emilk/egui) + [rodio](https://github.com/RustAudio/rodio).
