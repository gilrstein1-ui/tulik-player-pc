fn main() {
    // Recompile if the privately-injected endpoint list changes.
    println!("cargo:rerun-if-env-changed=TULIK_PI_ENDPOINTS");
    tauri_build::build()
}
