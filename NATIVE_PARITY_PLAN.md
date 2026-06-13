# Native player (TulikPlayerNative.exe) → web parity plan

> **STATUS 2026-06-13 (build 31, repo now PUBLIC): DONE through Round 5.** Rounds 1–3 (feedback
> tags/notices, presence/devices/hand-off, puppy + send-link) shipped earlier.
> **Round 4** repainted the whole app to the web's gold-on-near-black look + DM Sans
> (fixes "looks like old MediaSage") and added feedback **screenshots** (capture +
> clipboard paste / Ctrl+V — fixes "can't paste screenshots"). **Round 5** added the
> "Start something" idle panel (C6). Native is now at **full feature parity** except
> the items below explicitly deferred to their own Gil-runtime-tested branches:
> media keys/SMTC (E1), gapless splice (D3), stall buffer (D4), annotator/voice note
> (A4/A5), coach-mark ring (C7), WebSocket hand-off (B4). C3 (track-number hover-play)
> declined — native rows already play on click. See FEATURES.md for the live matrix.
> Repo went public 2026-06-13; per-user endpoints/creds now live only in the
> `BUILD_CONFIG`/`PW_*` secrets (nothing identifying in source or history).

_Authored 2026-06-13. Baseline: native **build 22** (2026-06-11). Source of truth:
`mediasage-gil/frontend/player.html` + `backend/player_ext.py`._

Native is the only player form that reimplements features (egui + rodio, no browser),
so it is the only one that drifts. The WebView wrappers (TulikPlayer.exe / Android)
inherit web changes for free. This plan closes the drift since build 22 **plus** the
standing backlog, with a deliberate port-vs-N/A call on every item.

Owner context (2026-06-13): the native player is now **in scope** for all future
bug/feature/improvement rollouts (watcher-pipeline-plan.md, changed in another chat).

Legend — Effort: S ≤~1h · M a few h · L a day+. Risk: Low / Med / High.
"Runtime-test" = I cannot verify it from the Pi CLI; needs Gil on real Windows.

---

## A. Headline drift — feedback + notices (the reason this task exists)

| # | Item | Decision | Effort | Risk | Notes |
|---|------|----------|--------|------|-------|
| A1 | Feedback form gains **🐞 Bug / 💡 Idea** tag, **"only for me"** checkbox, and `source`+`page` fields | **PORT** | S | Low | `send_feedback()` is currently text-only. Add `kind`, `personal`, `source=windows-app`, `page=/player` to the multipart POST. Server already accepts all of these. |
| A2 | **Notices system** — green "✓ Fixed — you reported …" box + **⬆ Update** + Dismiss | **PORT** | M | Low | Poll `GET /api/player/notices` on start + every 180s; render an egui banner; `POST /api/player/notices/dismiss`. The web's "Update" reloads the page — native can't self-update, so **⬆ Update opens the hub download URL in the browser** (or shows "re-download TulikPlayerNative.exe from your hub"). |
| A3 | Feedback **screenshot attach** (plain window capture) | **PORT (lite)** | S | Low | egui can hand us the framebuffer; attach it as a PNG file on the existing multipart. |
| A4 | Feedback **red-pen / text annotator** | **DEFER** | L | Med | The web draws on an html2canvas snapshot. A full egui draw-on-image editor is a day's work for marginal value on desktop. Plain screenshot (A3) covers most of the need. |
| A5 | Feedback **voice note** recording | **DEFER** | M | Med | Needs a cpal mic capture path; low demand on a desktop app. |

## B. Presence, devices & cross-device remote

These are coupled: the device list and web↔web hand-off all ride the **presence beat**.
Native today only follows/casts to **Plexamp**, and sends **no** presence — so it is
invisible to other devices and absent from the usage dashboard.

| # | Item | Decision | Effort | Risk | Notes |
|---|------|----------|--------|------|-------|
| B1 | **Presence beat** — `POST /api/player/presence` every ~25s with now-playing + `interval_s` | **PORT** | M | Low-Med | New background thread. Also makes native count in the hub **usage dashboard** (currently it doesn't). |
| B2 | **⇄ Devices popover** — own devices, online/playing, **Send ▶** / **⥁ Pull** | **PORT** | M | Med | `GET /api/player/devices`; Send = `handoff`/`cast`, Pull = `pull-request`. Builds on B1. |
| B3 | **Web↔web hand-off + remote-cmd** — follow/control ANY TulikPlayer (not just Plexamp); **"⥁ Pull here"** banner; **auto-remote on open** | **PORT** | M-L | Med | Beat response carries `handoff`/`pullreq`/`cmds` — act on them. Extends the existing Plexamp-follow code. |
| B4 | **WebSocket** instant delivery (`/api/player/ws`) | **DEFER** | M | Med | Pure latency optimisation; the presence-beat inbox (B1) already delivers everything within ~25s. Add later if hand-off feels laggy. |

## C. Right-click everywhere + library polish

| # | Item | Decision | Effort | Risk | Notes |
|---|------|----------|--------|------|-------|
| C1 | Context menu on **history rows, playbar, big cover, queue rows** (native has it only on library song/album/artist) | **PORT** | M | Low | Reuse the existing `showTrackMenu` equivalent; wire the missing surfaces. |
| C2 | Track menu gains **💿 Go to album**, **☄ radio**, **🔗 Send to a friend** rows everywhere | **PORT** | S | Low | Most already exist in library context; unify. |
| C3 | Album view: **track number → ▶ on hover** (play just this song) | **PORT** | S | Low | |
| C4 | Artists view: **image grid** (cover thumb from `/library/artists` `thumb`) instead of monogram tiles | **PORT** | S-M | Low | Native already has an art cache; just feed it the artist thumb. |
| C5 | **Top-search carries into Library** on tab switch | **PORT/verify** | S | Low | Confirm native behaviour; wire if missing. |
| C6 | **"Start something" idle panel** in the Lyrics tile (surprise/shuffle/top/playlists) | **PORT** | S | Low | |
| C7 | Coach-mark ring/bubble help system (? per view) | **DEFER** | L | Low | Native already has a Help modal. The full ring overlay is heavy, web-shaped UX. |

## D. Now-playing / transport polish

| # | Item | Decision | Effort | Risk | Notes |
|---|------|----------|--------|------|-------|
| D1 | **Dancing puppy** replaces the spectrum visualizer (**Gil build only**; other 4 builds simply lose the visualizer per owner "remove viz everywhere") | **PORT** | M | Low-Med (runtime-test) | egui custom paint reacting to volume/tempo; gate on `cfg.user=="gil"`. Needs a visual check on Windows. |
| D2 | Filled **♥**, hollow **☆** until rated + hover color, **default volume 70%** honoured pre-play, **enqueue idx<0** guard (don't cut the playing track) | **VERIFY then fix** | S | Low | Audit says native largely has these; confirm each against the web and patch any gap. |
| D3 | **Gapless same-album splice** / crossfade | **DEFER** | L | High (runtime-test) | rodio has no real gapless; this is a meaningful audio-engine change and must be validated with real listening. Out of scope for this round. |
| D4 | Full-track **safety buffer + stall rescue** | **DEFER/verify** | M | Med (runtime-test) | rodio buffering behaviour; needs audio runtime testing. |

## E. System integration

| # | Item | Decision | Effort | Risk | Notes |
|---|------|----------|--------|------|-------|
| E1 | **System media keys / SMTC** (lock-screen, Bluetooth/car controls, play-pause-next from keyboard media keys) | **PORT** | M | Med (runtime-test) | Needs the `souvlaki` crate. The web gets this via mediaSession; it's the one true native-only win. Must be tested on real Windows. |
| E2 | **Send-to-a-friend** link → copy per-recipient URL to clipboard | **PORT** | S | Low | `arboard` crate; build the same `…/player#share=` link the web builds. |
| E3 | **Receive** `#share=` deep-link card | **N/A** | — | — | Native has no URL routing / no browser; nothing to receive a deep link into. The send half (E2) still ships. |

## F. Genuinely N/A for a native desktop app (deliberate, not skipped by omission)

- **Offline downloads** (IndexedDB, gated to `MediaSageAndroid` UA) — phone-only by design.
- **PWA install / "get the apps"** — native already links the apps from Help.
- **Mobile responsive layouts** (banner hide ≤700px, bottom-sheet popovers) — egui owns its own layout.
- **Music Files ⊞ overlay postMessage fix** — fixes a web-iframe bug that doesn't exist natively.
- **Android launcher icon** — Android-only.

---

## What I can verify from the Pi vs what needs Gil's Windows runtime

**I can verify here (CLI):** the code compiles (via CI), the right API calls/fields go
out, JSON shapes match `player_ext.py`, feedback/notice/presence requests are well-formed,
per-user gating (puppy = gil only), and FEATURES.md is updated.

**Only Gil can verify (real Windows runtime) — I'll list these explicitly on ship:**
- D1 puppy renders/animates correctly in the window.
- E1 media keys actually drive the app from the keyboard/lock-screen/Bluetooth.
- A2 the ⬆ Update button opens the right hub page.
- B2/B3 hand-off between the PC native app and the phone/another PC works end-to-end.
- A3 the screenshot capture grabs the window correctly.
- General: audio output, EQ, window chrome unaffected by the changes.

## Build / ship mechanics (unchanged)
CI-only: `gh workflow run build-native.yml` (matrix gil/ohad/rambo/noodles/canoli, bakes
`PW_*` secrets) → download the 5 exes → `sudo cp` each to `/var/www/<user>-hub/TulikPlayerNative.exe`
→ bump FEATURES.md + the build number in the title. Pi cannot compile Rust/Windows locally.

## Proposed sequencing (so each CI build is a coherent, testable drop)
1. **Round 1 (A1–A3, C1–C6, D2):** feedback tags + notices + right-click-everywhere +
   library polish + verify-fixes. All low-risk, no audio, no new crates. One CI build.
2. **Round 2 (B1–B3):** presence + devices + web hand-off. One CI build; Gil tests hand-off.
3. **Round 3 (D1, E1, E2):** puppy + media keys + share-link. New crates (`souvlaki`,
   `arboard`); Gil runtime-tests puppy + media keys.
Deferred (A4, A5, B4, C7, D3, D4): logged here, not this task.
