//! Per-user build config. Values are baked at compile time from env vars set by
//! the CI matrix (one binary per user) out of GitHub Secrets. NOTHING real lives
//! in source: a plain checkout builds with an empty endpoint + no credentials
//! (this is a client for your own logged-in backend — see README). The password
//! only ever reaches the binary via the `TULIK_AUTH_PW` secret at build time.

#[derive(Clone)]
pub struct Config {
    pub base_url: String, // e.g. https://your-server.example:PORT  (no trailing slash)
    pub user: String,     // basic-auth username
    pub pw: String,       // basic-auth password (baked)
    pub label: String,    // friendly user label for the window title
}

pub fn load() -> Config {
    let base_url = option_env!("TULIK_BASE_URL")
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string();
    let user = option_env!("TULIK_AUTH_USER").unwrap_or("").to_string();
    let pw = option_env!("TULIK_AUTH_PW").unwrap_or("").to_string();
    let label = option_env!("TULIK_USER_LABEL").unwrap_or("Player").to_string();
    Config { base_url, user, pw, label }
}
