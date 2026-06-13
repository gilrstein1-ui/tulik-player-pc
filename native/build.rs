// Re-run (and re-bake the option_env! values) whenever the per-user build
// config changes — otherwise a cached build could ship the wrong endpoint/creds.
fn main() {
    println!("cargo:rerun-if-env-changed=TULIK_BASE_URL");
    println!("cargo:rerun-if-env-changed=TULIK_AUTH_USER");
    println!("cargo:rerun-if-env-changed=TULIK_AUTH_PW");
    println!("cargo:rerun-if-env-changed=TULIK_USER_LABEL");
    println!("cargo:rerun-if-env-changed=TULIK_BUILD");
    println!("cargo:rerun-if-changed=../app-icon.png");

    #[cfg(windows)]
    embed_exe_icon();
}

/// Bake the dog logo into the .exe so Windows shows it in Explorer / taskbar /
/// when pinned. Best-effort: any failure is logged and ignored so it can never
/// break the build (the runtime window icon is set separately in main.rs).
#[cfg(windows)]
fn embed_exe_icon() {
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let ico = std::path::Path::new(&out_dir).join("app.ico");
    // build.rs runs with CWD = the crate root (native/), so the icon is ../
    let img = match image::open("../app-icon.png") {
        Ok(i) => i,
        Err(e) => {
            println!("cargo:warning=icon: could not open app-icon.png: {e}");
            return;
        }
    };
    // ICO frames must be <= 256px
    if let Err(e) = img.thumbnail(256, 256).save_with_format(&ico, image::ImageFormat::Ico) {
        println!("cargo:warning=icon: could not write app.ico: {e}");
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap_or("app.ico"));
    if let Err(e) = res.compile() {
        println!("cargo:warning=icon: winresource compile failed: {e}");
    }
}
