//! In-app updater for the native Windows player.
//!
//! Windows can't overwrite a running .exe, so the flow is: check the hub for a
//! newer build, download the new exe next to the current one, write a tiny
//! relauncher script that waits for this process to exit, swaps the exe in place,
//! and restarts it — then exit. Every build is signed-by-nature identical (same
//! source), and the swap never deletes the old exe until the new one is in place,
//! so a failed update just relaunches the current version (no broken install).

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
pub struct UpdateState {
    pub available: Option<i64>, // newer build number, if one is published
    pub busy: bool,             // downloading / swapping
    pub error: Option<String>,
}

pub type Shared = Arc<Mutex<UpdateState>>;

pub fn new_state() -> Shared {
    Arc::new(Mutex::new(UpdateState::default()))
}

const MANIFEST: &str = "/hub/TulikPlayerNative.version.json";
const APK: &str = "/hub/TulikPlayerNative.exe";

/// Background: GET the hub manifest, compare its `build` to ours.
pub fn check(state: Shared, base: String, user: String, pw: String, current_build: i64) {
    std::thread::spawn(move || {
        let url = format!("{}{}", base.trim_end_matches('/'), MANIFEST);
        let cli = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let resp = cli.get(&url).basic_auth(&user, Some(&pw)).send();
        if let Ok(r) = resp {
            if r.status().is_success() {
                if let Ok(j) = r.json::<serde_json::Value>() {
                    let b = j.get("build").and_then(|v| v.as_i64()).unwrap_or(0);
                    if b > current_build {
                        if let Ok(mut s) = state.lock() {
                            s.available = Some(b);
                        }
                    }
                }
            }
        }
    });
}

/// Download the new exe and spawn the relauncher, then exit. On any failure the
/// current app keeps running (we never touch the live exe ourselves).
pub fn apply(state: Shared, base: String, user: String, pw: String) {
    if let Ok(mut s) = state.lock() {
        s.busy = true;
        s.error = None;
    }
    std::thread::spawn(move || match do_apply(&base, &user, &pw) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            if let Ok(mut s) = state.lock() {
                s.busy = false;
                s.error = Some(e);
            }
        }
    });
}

fn do_apply(base: &str, user: &str, pw: &str) -> Result<(), String> {
    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = cur.parent().ok_or("no parent dir")?.to_path_buf();
    let new_exe = dir.join("TulikPlayerNative.new.exe");
    let url = format!("{}{}", base.trim_end_matches('/'), APK);

    let cli = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = cli
        .get(&url)
        .basic_auth(user, Some(pw))
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .map_err(|e| e.to_string())?;
    if bytes.len() < 200_000 {
        return Err("downloaded update looks too small".into());
    }
    std::fs::write(&new_exe, &bytes).map_err(|e| e.to_string())?;

    // Relauncher: wait for this exe to unlock, copy the new one over it (never
    // deletes the old until the copy succeeds), restart, then clean up.
    let bat = dir.join("tulik-update.bat");
    let script = format!(
        "@echo off\r\n\
         set /a n=0\r\n\
         :wait\r\n\
         timeout /t 1 /nobreak >nul\r\n\
         copy /Y \"{new}\" \"{cur}\" >nul 2>&1\r\n\
         if not errorlevel 1 goto done\r\n\
         set /a n+=1\r\n\
         if %n% lss 40 goto wait\r\n\
         :done\r\n\
         start \"\" \"{cur}\"\r\n\
         del \"{new}\" >nul 2>&1\r\n\
         del \"%~f0\" >nul 2>&1\r\n",
        new = new_exe.display(),
        cur = cur.display()
    );
    std::fs::write(&bat, script).map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "/min", &bat.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let _ = &bat; // non-Windows: nothing to launch (CI builds Windows only)
    }
    Ok(())
}
