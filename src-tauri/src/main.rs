// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

// Comma-separated "host:port" list of the player backend, injected privately at
// build time (never stored in source). Absent in a plain checkout, in which
// case the app falls back to the manual address box in the UI.
const ENDPOINTS: Option<&str> = option_env!("TULIK_PI_ENDPOINTS");

// Baked basic-auth credentials for this per-user build (injected privately at
// build time; absent in a plain checkout). Used to auto-sign-in to the public
// (password-protected) player so the user never sees a prompt.
const AUTH_USER: Option<&str> = option_env!("TULIK_AUTH_USER");
const AUTH_PW: Option<&str> = option_env!("TULIK_AUTH_PW");

/// Register a WebView2 handler that answers the player's HTTPS basic-auth
/// challenge with this build's baked credentials — no sign-in prompt.
#[cfg(target_os = "windows")]
fn install_basic_auth(app: &tauri::AppHandle) {
    let (user, pw) = match (AUTH_USER, AUTH_PW) {
        (Some(u), Some(p)) if !p.is_empty() => (u.to_string(), p.to_string()),
        _ => return,
    };
    let Some(win) = app.get_webview_window("main") else { return };
    let _ = win.with_webview(move |webview| unsafe {
        use webview2_com::BasicAuthenticationRequestedEventHandler;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2, ICoreWebView2BasicAuthenticationRequestedEventArgs, ICoreWebView2_10,
        };
        use windows::core::{Interface, HSTRING};

        let core = match webview.controller().CoreWebView2() {
            Ok(c) => c,
            Err(_) => return,
        };
        let core10: ICoreWebView2_10 = match core.cast() {
            Ok(c) => c,
            Err(_) => return,
        };
        let u = user.clone();
        let p = pw.clone();
        let handler = BasicAuthenticationRequestedEventHandler::create(Box::new(
            move |_sender: Option<ICoreWebView2>,
                  args: Option<ICoreWebView2BasicAuthenticationRequestedEventArgs>| {
                if let Some(args) = args.as_ref() {
                    if let Ok(resp) = args.Response() {
                        let _ = resp.SetUserName(&HSTRING::from(&u));
                        let _ = resp.SetPassword(&HSTRING::from(&p));
                    }
                }
                Ok(())
            },
        ));
        let mut token = std::mem::zeroed();
        let _ = core10.add_BasicAuthenticationRequested(&handler, &mut token);
    });
}

/// Probe each configured endpoint and return the first reachable player URL.
/// Done natively (not in the web layer) so it is free of the webview's
/// http/secure-context restrictions and is fully reliable.
#[tauri::command]
fn find_pi() -> Option<String> {
    let raw = ENDPOINTS?;
    for entry in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        // An entry is either "host:port" (LAN/Tailscale → http) or a full
        // "https://host:port" / "http://host:port" (public address). Probe the
        // host:port for reachability but PRESERVE the scheme for navigation.
        let (scheme, hostport) = if let Some(rest) = entry.strip_prefix("https://") {
            ("https", rest.trim_end_matches('/'))
        } else if let Some(rest) = entry.strip_prefix("http://") {
            ("http", rest.trim_end_matches('/'))
        } else {
            ("http", entry)
        };
        if let Ok(addrs) = hostport.to_socket_addrs() {
            for addr in addrs {
                if TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).is_ok() {
                    return Some(format!("{}://{}/player", scheme, hostport));
                }
            }
        }
    }
    None
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![find_pi])
        .setup(|app| {
            // System-wide media keys -> click the player's transport buttons, so
            // the keyboard's play/pause/next/prev control playback even when the
            // app is in the background.
            let play = Shortcut::new(None, Code::MediaPlayPause);
            let next = Shortcut::new(None, Code::MediaTrackNext);
            let prev = Shortcut::new(None, Code::MediaTrackPrevious);
            let stop = Shortcut::new(None, Code::MediaStop);
            let (h_play, h_next, h_prev, h_stop) =
                (play.clone(), next.clone(), prev.clone(), stop.clone());

            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |app, sc, event| {
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        let sel = if *sc == h_play || *sc == h_stop {
                            "#b-play"
                        } else if *sc == h_next {
                            "#b-next"
                        } else if *sc == h_prev {
                            "#b-prev"
                        } else {
                            return;
                        };
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.eval(format!(
                                "document.querySelector('{}')?.click()",
                                sel
                            ));
                        }
                    })
                    .build(),
            )?;

            let gs = app.global_shortcut();
            let _ = gs.register(play);
            let _ = gs.register(next);
            let _ = gs.register(prev);
            let _ = gs.register(stop);

            #[cfg(target_os = "windows")]
            install_basic_auth(app.handle());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tulik player");
}
