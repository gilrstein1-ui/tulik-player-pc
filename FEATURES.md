# tulik player — cross-version feature matrix

Single source of truth for **what exists where**, so a change in one version can be
mirrored to the others. The **web player** (`mediasage*/frontend/player.html`) is the
canonical feature set; everything else is measured against it.

**Versions**
- **Web** — `player.html`, served at `/player`. The reference implementation.
- **Native (PC)** — `tulik-player-pc/native/` (egui + rodio, *reimplements* each
  feature natively; the only version that can drift, so it's the one to track).
- **Tauri (PC)** — `tulik-player-pc/` (WebView2 wrapper of the live `/player`).
  Inherits every web feature automatically (🌐).
- **Android** — `player-android/` (WebView wrapper of the live `/player`).
  Inherits every web feature automatically (🌐).

**Legend:** ✅ done · ⚠️ partial · ❌ missing · 🌐 inherited via webview · — N/A
· ❓ verify in web

_Last updated: 2026-06-13 — native **build 31** (Rounds 1–5 done: theme repaint + DM Sans, feedback screenshots, idle panel); **repo is now PUBLIC**, endpoints/creds only in `BUILD_CONFIG`/`PW_*` secrets. **Visualizer REMOVED everywhere** by owner request, replaced on Gil's PC app only by the dancing puppy._

## Native parity Round 4 — 2026-06-13 (build 27) — "stop looking like old MediaSage" + screenshots
The two PC-app feedback items Gil flagged as still "planned":
| Item | Native before | Native now (build 27) |
|---|---|---|
| **"PC app looks like the old MediaSage style"** | lavender/purple egui palette | ✅ **Repainted to the web player's literal CSS `:root`** — `--bg #0e0e10`, gold `--accent #e5a00d` + bright `--accent2 #f0b833`, `--tile/--tile2`, `--text/--muted/--faint`, `--good`. Every inline purple fill neutralised (top bar, transport pill, search box, ⊞ tools button, hover/selection, visualizer). **DM Sans embedded** (`native/assets/DMSans.ttf`, the web font) as the proportional typeface; egui default fonts stay as the emoji/symbol fallback chain. |
| **"Can't paste screenshots in the PC app's feedback form"** | text-only | ✅ **📸 Capture app** (egui `ViewportCommand::Screenshot` → framebuffer → PNG), **📋 Paste image** + **Ctrl+V** (arboard clipboard → PNG), `✓ image W×H / ✕` attachment row; PNG rides the existing multipart POST under the same **`files`** field the web uses. |

Runtime-test on Windows (can't verify from the Pi): the new look reads right, 📸 grabs the window, Ctrl+V pastes a Win+Shift+S grab. Deployed to all 5 hubs.
**Already at parity (verified, not changed):** loved heart hollow ♡→filled ♥, ★ stars hollow ☆→gold ★ until rated, default volume 70%, enqueue idx guard, global search routes into Library.
**Round 5 (C6) — ✅ SHIPPED (build 31, live all 5):** "Start something" idle panel in the Lyrics tile — when nothing is playing the tile offers 🎲 Surprise / ⭐ Top tracks / 🕘 Recently played / 📃 Playlists. (Was briefly blocked when GitHub Actions hit the free-minute limit; resolved by making this repo **public** — public repos get unlimited free Actions.)

**Repo went PUBLIC 2026-06-13.** All personal endpoints/usernames/ports were removed from the source **and the entire git history** (squashed to one clean commit; old tags + releases deleted) and now live only in the private `BUILD_CONFIG` / `PW_*` Actions secrets, injected + log-masked at build time. A plain checkout builds an empty-endpoint app. See README warning. No password/token was ever in history.
**Declined (native uses an equivalent interaction, not a gap):** album view track-number → ▶ on hover (C3) — native track rows already play on click + full right-click menu; the number→▶ is web decoration over the same action.
**Owner-deferred (own runtime-tested branches, need Gil on Windows — a wrong guess breaks the build/audio):** system media keys/SMTC (E1), gapless same-album splice (D3), full-track stall buffer (D4), feedback red-pen annotator + voice note (A4/A5), coach-mark ring overlay (C7), WebSocket instant hand-off (B4). N/A for native: offline downloads, PWA install, receive `#share=` deep link, mobile responsive layouts.

**Parity status after build 27: at full feature parity with the web player except the owner-deferred audio-engine / system-integration items above.**

## Native parity catch-up — 2026-06-13 (owner made native a first-class rollout target)
Shipping in three CI builds. The build number is the CI run number (was 22 at the
last rename). **Round 1 (shipped this build):**
| Item | Native before | Native now |
|---|---|---|
| Feedback 🐞 Bug / 💡 Idea tag + "Only for me" + `source=windows-app`/`page` tags | ❌ text-only | ✅ |
| **Notices** green "✓ Fixed — you reported …" banner (poll `/notices` every 3 min, Dismiss → `/notices/dismiss`; `fix_type=pc` shows **⬆ Update → opens the hub** since native can't self-update) | ❌ | ✅ |
| Right-click track menu on the **queue side-panel rows** + the **big now-playing cover** (history/recent/browser/album/playlist rows already had it) | ⚠ library only | ✅ (playbar mini-thumb still pending) |
| Track menu enriched: **▶ Play now** + **💿 Go to album** added everywhere | ⚠ | ✅ |
| Artists view = **real cover-art grid** (server `thumb`), monogram only as fallback | ❌ monograms | ✅ |
| Loved heart renders **hollow ♡ → filled ♥** | ⚠ always ♥ | ✅ |
| Default volume **70%** on fresh start (web parity) | ❌ 100% | ✅ |
| Enqueue never cuts off the playing track (idx guard) | ✅ (verified — already correct) | ✅ |

**Round 2 (shipped):** presence beat every 20s → native now appears in other devices' ⇄ popover **and counts in the hub usage dashboard**; receives hand-off + remote play/pause/next/prev; answers pull-requests from its own queue; the 📡 menu gained a **"Your players"** section (online/playing status + **▶ Send** / **⥁ Pull**) for web/PWA/phone/other-PC devices. (Deferred minor: a dedicated "⥁ Pull here" banner + auto-web-remote-on-open — the popover Pull + existing Plexamp auto-follow cover the core.)
**Round 3 (shipped):** **dancing puppy** (Gil build only) replaced the spectrum visualizer — the real dog brandmark bobs + scales to the FFT energy, mini in the playbar + fullscreen on `V`, with a pulsing glow; the other 4 builds have **no visualizer** (owner "removed everywhere"). **Send-to-a-friend** copy-link added to the track menu (🔗 → friend → copies `…/player#share=t.<rk>.<sender>` to the clipboard via egui's built-in clipboard — no extra crate). **Deferred: system media keys / SMTC** — needs a window-handle (HWND) integration with eframe that can't be compile-verified from the Pi, and a wrong guess breaks the whole build; it gets its own branch + Gil runtime test (receive-side `#share=` stays N/A: native has no URL routing).
**Deferred / N/A (see `NATIVE_PARITY_PLAN.md`):** feedback screenshot annotator + voice note, gapless same-album splice, coach-mark ring overlay, WebSocket instant hand-off, offline downloads, PWA install, receive `#share=` deep link.

## Offline downloads — 2026-06-13 (PHONE ONLY · gil + rambo TEST PHASE, not GA)
"Make available offline": save songs/albums/playlists to the phone and play with no
signal. Gated to the Android app UA (`MediaSageAndroid`) AND deployed only to the
gil + rambo instances — the other 3 instances and all PC/desktop contexts are
byte-identical to before (offline detection early-returns when not the phone app).
| Piece | Where | Notes |
|---|---|---|
| `GET /api/player/offline/{rk}` | backend player_ext.py (gil+rambo) | lossless/>320k → ffmpeg 256k MP3 (~10× smaller, token piped via stdin, ionice/nice); already-compressed → passthrough original. Online playback still uses `/stream/{rk}` at full quality. |
| IndexedDB `tlk-offline` | player.html module | 2 stores: `meta` (small, +tiny art, for browsing) / `audio` (big blob, read only on play). |
| "⬇ Make available offline" | track/album/playlist right-click | sequential download (one at a time), progress toast, `✓ Saved — remove` toggle. |
| ⬇ Offline manager | toolbelt button | grouped by album, total size + device-free estimate, per-item / per-album / remove-all. |
| Offline mode | auto on unreachable server | library/search/recent/album/track/lyrics served from IDB only (so "Albums" shows only downloads); ✈ banner; radio/⚡/📱/devices/playlists hidden; **online always hi-q, downloads used only offline**. |
| SW v2 (`player-sw.js`) | gil+rambo | network-first shell cache for cold offline boot + `tlk-sw-reset` kill-switch. |

**On-phone test gate (the real unknown):** cold offline BOOT (page loading with zero
signal) depends on the Android System WebView running the service worker. Download +
storage + offline playback + offline browsing all work regardless; only first-load-
offline rides the SW. Confirm on Gil's actual phone (airplane mode) before Rambo.
Fallback if WebView SW is unreliable: bundle the shell in the APK.
_Polish deferred:_ per-row "saved" badges in the online library (today: shown in the
menu per-item + the manager + offline-mode filtering).

## Web feedback round 3 — 2026-06-13 morning (GA all 5)
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| ★ stars hollow (☆) until rated + hover color (picker itself shipped round 2) | ✅ | ❌ | 🌐 | 🌐 |
| 🔗 Send to a friend: rclick song/album/playlist → copy per-recipient link; `#share=` deep link shows a 💌 card (Play/Queue) on arrival | ✅ | ❌ | 🌐 | 🌐 |
| Coach-mark content rewrite: "How it flows" intro bubble + only non-obvious marks | ✅ | ❌ | 🌐 | 🌐 |
| Music Files "Back to the player" closes the ⊞ overlay (postMessage) — fixes the second-frozen-player / "UI stuck" bug; toolmenu.js bumped all 8 copies | ✅ | — | 🌐 | 🌐 |

## Tap-along beat game — 2026-06-14 (Gil ONLY)
First **native** parity item ported back from web. The webview forms (Tauri/Android) inherit the
live page; native re-implements it in egui. Beat data via `/api/player/beatmap/{rk}` (librosa beat-map;
endpoint exists on Gil's instance only, so it's naturally gil-gated). Folded into the puppy `V` overlay.
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| 🥁 Tap-along beat game: scores taps vs the real beat-map (±window, double-time midpoint accepted), seconds-on-beat streak, miss → red flash + reset, session best, beat-pulse ring; pad-click or **Space**, `←`/Esc back to the puppy | ✅ | ✅ | 🌐 | 🌐 |

## Web feedback round 2 — 2026-06-13 ~04:00 (GA all 5)
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| 🐕 puppy GA EVERYONE + steps to the track's bliss tempo score (`v` key back) | ✅ | ❌ | 🌐 | 🌐 |
| Gapless same-album splice (no fade, ~1-frame start; crossfade still for mixes) | ✅ | ❌ | 🌐 | 🌐 |
| Annotator v2: html2canvas-**pro** (modern-CSS colors; 1.4.1 silently failed) + 📸 button in ✉ form + hover hint | ✅ | ❌ | 🌐 | 🌐 |
| Artist grid name fix (`.arow` CSS clash crushed names) | ✅ | — | 🌐 | 🌐 |
| Now-playing banner slimmer; hidden ≤700px/560px (queue-only mini layout) | ✅ | ❌ | 🌐 | 🌐 |
| "Start something" idle panel in the Lyrics tile (surprise/shuffle/top/playlists) | ✅ | ❌ | 🌐 | 🌐 |
| Coach-mark help: ? labels the live screen per view; 📖 full guide inside | ✅ | ❌ | 🌐 | 🌐 |
| Playlists tab = embedded /playlists cockpit (old card grid retired) | ✅ | ❌ | 🌐 | 🌐 |
| ⊞ Tools menu: Plex+Playlists rows removed; Get-Android/Get-PC blob-downloads; Music Files (gil+ohad) | ✅ | — | 🌐 | 🌐 |
| Android launcher icon = dog mark (v2.5 build 7) | — | — | — | ✅ |

## Web feedback round 2026-06-13 (GA all 5 unless noted)
All Web ✅ · Tauri/Android 🌐 inherited · Native ❌ (adds to the native-parity TODO)
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Right-click song menu EVERYWHERE (history tile+browser, playbar, big title/cover) | ✅ | ❌ | 🌐 | 🌐 |
| Context menus for artists (play/shuffle/radio/albums) + playlist cards (play/shuffle/next/queue) | ✅ | ❌ | 🌐 | 🌐 |
| **Visualizer REMOVED** (mini + fullscreen, `v` key) — owner request 2026-06-13 | ✅ | — | 🌐 | 🌐 |
| 🐕 Dancing puppy (mini canvas by volume + fullscreen, beat-reactive) — **Gil + PC app ONLY** | ✅ | ❌ | 🌐 | — |
| ⋮ three-dots: bigger hit area, accent highlight on hover | ✅ | ❌ | 🌐 | 🌐 |
| Top-right buttons restyled (gradient, lift+glow on hover) | ✅ | ❌ | 🌐 | 🌐 |
| Right-click ✉ → screenshot + red-pen/text annotator → send as feedback (html2canvas vendored at `/static/html2canvas.min.js`) | ✅ | ❌ | 🌐 | 🌐 |
| Album view: track number → ▶ play-this-song on hover | ✅ | ❌ | 🌐 | 🌐 |
| Artists view: image-square grid (tile art = one of the artist's album covers; `/library/artists` now returns `thumb`) | ✅ | ❌ | 🌐 | 🌐 |
| 📱 Follow/remote works on ANY playing TulikPlayer, not just Plexamp (presence carries rk/offset/duration/state; new `POST /api/player/remote-cmd`; auto-enters web-follow on load) | ✅ | ❌ | 🌐 | 🌐 |

## Web feedback batch 2026-06-12 (GA, all 5 users)
All Web ✅ · Tauri/Android 🌐 inherited · **Native ❌ — every row below is native-parity TODO**
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| ⇄ Devices popover (own devices only, playing-status, Send / Pull) | ✅ | ❌ | 🌐 | 🌐 |
| Presence beats + direct web↔web hand-off (PC→phone send) | ✅ | ❌ | 🌐 | 🌐 |
| Auto-remote on open + "⥁ Pull here" banner | ✅ | ❌ | 🌐 | 🌐 |
| One History tile w/ per-device filter chips (Recently-played tile retired) | ✅ | ❌ | 🌐 | 🌐 |
| Queue auto-consumes finished songs (stay in History) | ✅ | ❌ | 🌐 | 🌐 |
| Queue art → ▶ on hover; ⋮ menu on every row (touch-visible) | ✅ | ❌ | 🌐 | 🌐 |
| Seek bar press-and-drag (mouse + touch) | ✅ | ✅? (verify) | 🌐 | 🌐 |
| Playbar title/artist → album/artist jump | ✅ | ❌ | 🌐 | 🌐 |
| Default volume 70% + slider honoured pre-play | ✅ | ❓ | 🌐 | 🌐 |
| Unexpected-resume guard (paused stays paused after a call) | ✅ | — | 🌐 | 🌐 |
| Top-search carries into Library on tab switch | ✅ | ❌ | 🌐 | 🌐 |
| Feedback "only for me" tickbox (`personal` flag) | ✅ | ❌ | 🌐 | 🌐 |
| `window.__np`/`__npAction` bridge hooks (car/Bluetooth metadata + controls) | ✅ | — | — | ✅ (v2.1) |
| Usage accounting (presence → /api/player/usage) + hub versions dashboard | ✅ | ❌ | 🌐 | 🌐 |
| New bandana-dog brandmark (NOT on Rambo's player) | ✅ | ❌ | 🌐 | 🌐 |

## Shell / navigation
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Player / Library / Playlists / History tabs | ✅ | ✅ | 🌐 | 🌐 |
| Global library search | ✅ | ✅ (routes to Library) | 🌐 | 🌐 |
| Window min / max / close | ✅ (custom) | ✅ (OS frame) | ✅ | — |
| Easy resize (grips) | ✅ | ✅ | ⚠️ | — |
| App icon (dog) — window/taskbar | ✅ | ✅ | ✅ | ✅ |
| In-app header brandmark (real dog photo, not 🐶 emoji) | ✅ | ✅ | 🌐 | 🌐 |
| App icon — .exe / launcher | — | ✅ | ✅ | ✅ |
| Help (what each button does) | ✅ | ✅ | 🌐 | 🌐 |
| Feedback / report a bug | ✅ | ✅ | 🌐 | 🌐 |
| "Install app" / get-the-apps links | ✅ | ✅ (Help → browser links) | 🌐 | 🌐 |
| ⊞ Tools/links menu (open without stopping music) | ✅ (overlay) | ✅ (⊞ → browser) | 🌐 | 🌐 |
| Sleep timer 🌙 (web's "moon" icon — NOT a theme toggle) | ✅ | ✅ | 🌐 | 🌐 |

## Now playing (Player tab)
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Cover art | ✅ | ✅ | 🌐 | 🌐 |
| Cover glow effect | ✅ | ✅ | 🌐 | 🌐 |
| Title / artist / album | ✅ | ✅ | 🌐 | 🌐 |
| Clickable artist / album → go to artist/album | ✅ | ✅ | 🌐 | 🌐 |
| Format badge (codec · kbps) | ✅ | ✅ | 🌐 | 🌐 |
| Synced lyrics + autoscroll + click-to-seek | ✅ | ✅ | 🌐 | 🌐 |
| Plain-lyrics fallback | ✅ | ✅ | 🌐 | 🌐 |
| Queue: jump / remove / clear | ✅ | ✅ | 🌐 | 🌐 |
| Queue: reorder | ✅ (drag) | ✅ (▴▾ buttons) | 🌐 | 🌐 |
| Save queue as Plex playlist | ✅ | ✅ | 🌐 | 🌐 |
| Recently-played panel | ✅ | ✅ | 🌐 | 🌐 |
| Persistent Queue side-panel (in Library) | ✅ | ✅ | 🌐 | 🌐 |
| Start radio (sonic similar) — keeps current track, replaces queue below | ✅ | ✅ | 🌐 | 🌐 |
| Visualizer | ✅ (mini + full) | ✅ (playbar strip + fullscreen, V key) | 🌐 | 🌐 |

## Transport / audio
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Shuffle | ✅ | ✅ | 🌐 | 🌐 |
| Prev / Play-Pause / Next | ✅ | ✅ | 🌐 | 🌐 |
| Repeat off/all/one | ✅ | ✅ | 🌐 | 🌐 |
| Seek bar | ✅ (waveform) | ✅ (fill bar) | 🌐 | 🌐 |
| Volume | ✅ | ✅ | 🌐 | 🌐 |
| Mute | ✅ | ✅ (M key) | 🌐 | 🌐 |
| Heart + star rating | ✅ | ✅ | 🌐 | 🌐 |
| L/R balance presets (50/50 · A · B) | ✅ | ✅ | 🌐 | 🌐 |
| 10-band EQ | ✅ | ✅ | 🌐 | 🌐 |
| System media keys / lock-screen | 🌐(mediaSession) | ❌ | ✅ | ✅ (foreground svc) |
| Background audio not throttled | ❌ (tab) | ✅ (native) | ✅ | ✅ |

## Library
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Artists / Albums / Songs browse | ✅ | ✅ (artist card grid w/ monograms) | 🌐 | 🌐 |
| Library search | ✅ | ✅ | 🌐 | 🌐 |
| Lyric search | ✅ | ✅ | 🌐 | 🌐 |
| Focus: Genre / Decade / ❤ Loved (combine) | ✅ | ✅ | 🌐 | 🌐 |
| Sort (A–Z / Year / Added / Count) | ✅ | ✅ | 🌐 | 🌐 |
| Add-to-queue mode | ✅ | ✅ | 🌐 | 🌐 |
| Quick picks (recent/played/top/surprise) | ✅ | ✅ | 🌐 | 🌐 |
| Album/artist right-click (play/shuffle/next/add) | ✅ | ✅ | 🌐 | 🌐 |
| Album drill-in (play all/shuffle/queue) | ✅ | ✅ | 🌐 | 🌐 |
| Artist drill-in | ✅ | ✅ | 🌐 | 🌐 |
| Calm grouped toolbar (segmented browse · banded Focus/Quick) | ✅ | ✅ | 🌐 | 🌐 |
| Active-filter chips (removable) under count | ✅ | ✅ | 🌐 | 🌐 |
| A–Z scroll rail | ✅ | ✅ (Library, A–Z sort only) | 🌐 | 🌐 |

## Playlists
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Grouped playlists browse | ✅ | ✅ | 🌐 | 🌐 |
| Play a playlist | ✅ | ✅ | 🌐 | 🌐 |
| Mood categories / reorder / device play (full /playlists page) | ✅ | ❌ (use web page) | 🌐 | 🌐 |

## History
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Text + lyric search | ✅ | ✅ | 🌐 | 🌐 |
| Date range (today/week/month/year/all) | ✅ | ✅ | 🌐 | 🌐 |
| Sort recent / most-played | ✅ | ✅ | 🌐 | 🌐 |
| Stats line (plays / distinct / since) | ✅ | ✅ | 🌐 | 🌐 |
| Per-row relative time + play count | ✅ | ✅ | 🌐 | 🌐 |

## Devices / remote
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Cast / hand-off to a Plex device | ✅ | ✅ | 🌐 | 🌐 |
| Foreign-device confirm guard | ✅ | ✅ | 🌐 | 🌐 |
| Pull what's playing on Plexamp → here | ✅ | ✅ | 🌐 | 🌐 |
| Follow / remote-control Plexamp | ✅ | ✅ | 🌐 | 🌐 |

## AI / extras
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| Quick generate (AI playlist, SSE) | ✅ | ✅ | 🌐 | 🌐 |
| Sleep timer | ✅ | ✅ | 🌐 | 🌐 |
| Keyboard shortcuts (space/arrows/m/esc/V) | ✅ | ✅ | 🌐 | 🌐 |
| PWA / offline shell | ✅ | — | — | — |

## Native-only / cross-cutting notes
- **Native (PC)** is the only version that reimplements features, so it's the one
  that can fall behind — **when a web feature changes, update `native/` here.**
- **Tauri & Android** render the live `/player`, so a web change ships to them with
  no code change (just confirm the wrapper still loads).
- **No theme toggle exists** — the web's 🌙 "moon" icon is the **sleep timer** (native
  now uses 🌙 too). There is no light/dark switch to port.
- **Native deliberately skipped** (cosmetic / N/A): literal waveform-shaped seek bar,
  drag-drop queue reorder (uses ▴▾), full /playlists management page, PWA install,
  Badges/achievements (web-only). **Candidate to add:** system media keys / SMTC
  (needs a new crate — `souvlaki`/`windows` — so it's an opt-in build, not in build 19).
- **Build 19 (Claude Fable web-parity round)** closed: A–Z rail, fullscreen visualizer,
  cover glow, radio auto-extend, startup phone-follow, lyrics click-to-seek, clickable
  album, mute-on-icon, and History/Help/Playlists polish. `play()` is now `&mut self`
  (tracks the radio lifecycle at the command funnel).
- **Build 22 (2026-06-11)** — rename only: binary `MediaSagePlayerNative.exe` → **`TulikPlayer.exe`**
  (taskbar + Windows volume-mixer name), window title → "TulikPlayer — <user> · build N",
  API user-agent → `TulikPlayer/1.0` (server feedback-tagging recognises old + new).

## Web feedback round 2026-06-11 — native parity TODO
Shipped to the web player (all 5 instances); wrappers (Tauri/Android) inherit 🌐.
**Native needs its own implementation** (next Fable round candidates):
| Feature | Web | Native | Tauri | Android |
|---|---|---|---|---|
| enqueue no longer cuts off a playing track (idx<0 guard) | ✅ | ❓ check same bug | 🌐 | 🌐 |
| Queue reorder = pointer-drag (mouse anywhere / touch grip) | ✅ | ❓ (native has own dnd?) | 🌐 | 🌐 |
| Loved heart renders FILLED ♥ | ✅ | ❌ | 🌐 | 🌐 |
| Feedback: Enter sends + vivid gold Send + source/page tag | ✅ | ❌ (native feedback?) | 🌐 | 🌐 |
| Album-card right-click menu (play/next/queue/radio/artist) | ✅ | ✅ (build 20 had it) | 🌐 | 🌐 |
| Hover ▶/⏭/＋ next to title (not far right) | ✅ | ❓ | 🌐 | 🌐 |
| Full-track safety buffer + stall rescue | ✅ | ❓ (rodio buffering?) | 🌐 | 🌐 |
| Follow-mode ☄ radio casts to the phone | ✅ | — | 🌐 | 🌐 |
| Narrow-window top bar fixes | ✅ | — (native layout) | 🌐 | 🌐 |

- **Build 20 (Fable Round 3)** added: ⊞ Tools corner overlay (browser links, music
  keeps playing), right-click menus on album & artist cards, artist card grid w/
  monogram tiles, and responsive flex grids + scaling side panel.
- **Android mobile-UX rounds (its own Fable chat; all in `bridge.js` injected CSS,
  no player.html change, Android-only):** 2026-06-09 every-view/overlay phone pass
  (pinned bottom transport, bottom-sheet popovers, 44px targets, hover-only controls
  revealed); 2026-06-10 Library redesign (segmented tabs, collapsible 🎤 lyrics
  search, filter/quick-picks chip rails, floating A–Z rail). Web/Tauri phone-browser
  views do NOT get these; long-term plan is folding the mobile block into player.html.

## How to keep this current
When you change the player in any version:
1. Update the **Web** column (it's the source of truth).
2. Port to **Native (PC)** (`native/src/`), tick its cell, rebuild via CI.
3. Confirm **Tauri/Android** still load the page (they inherit the change).
4. Bump the "Last updated" date.
