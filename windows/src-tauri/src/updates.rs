//! Updates, for this build. (Local to this build: upstream ships no updater.)
//!
//! This build is not upstream's release: it is built from a fork. A fork may be
//! built and run for oneself but not published under the Coucou name
//! (LICENSE-ASSETS.md), so its builds are kept in a private repository, which
//! builds each change, signs the installer and writes a `latest.json` beside it.
//! This module asks that repository whether a newer build exists, and installs
//! it when the user says so. Nothing is ever installed on its own.
//!
//! The repository is private, so every request carries a read-only token the
//! user made for it (Settings → Updates), kept in the OS keystore like the API
//! keys. It is sent to GitHub's own hosts and nowhere else: a `latest.json`
//! that points the download anywhere else is refused before anything is sent.
//!
//! The address and the public key are in tauri.conf.json (`plugins.updater`).
//! An installer whose signature does not match the key is refused by the plugin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri::http::header::{HeaderValue, ACCEPT};
use tauri_plugin_updater::{Updater, UpdaterExt};

/// Asked of GitHub's API for the build list: the file itself, not a description
/// of it. A fine-grained token is read by the API, not by the raw-file host.
const ACCEPT_FILE: &str = "application/vnd.github.raw+json";
/// Asked of the API for a release's file: its bytes.
const ACCEPT_INSTALLER: &str = "application/octet-stream";

/// The keystore entry of the token that reads the private builds.
pub const TOKEN_KEY: &str = "updates-token";
/// Where an installer may be downloaded from: the API that serves the private
/// release's file. The token is never sent to any other host.
const DOWNLOAD_HOST: &str = "api.github.com";

/// Whether `url` is somewhere the token may go.
fn download_allowed(url: &tauri::Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some(DOWNLOAD_HOST)
}

/// The updater, with the token on its requests. An error says what is missing.
fn updater(app: &AppHandle) -> Result<Updater, String> {
    let token = crate::secrets::get(TOKEN_KEY)
        .ok_or_else(|| "Add the access token for your builds below.".to_string())?;
    app.updater_builder()
        .header("Authorization", format!("Bearer {token}"))
        .map_err(|_| "The access token has characters a token cannot have.".to_string())?
        .header(ACCEPT, ACCEPT_FILE)
        .map_err(brief)?
        .build()
        .map_err(brief)
}

/// After the app starts, before the first look: the island comes first.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(90);
/// Between two looks, while the app runs.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// What Settings and the tray show.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The version that is running.
    pub current: String,
    /// A newer one that can be installed, if any.
    pub available: Option<String>,
    /// What the release says about itself.
    pub notes: Option<String>,
    /// Seconds since the epoch of the last look that got an answer or an error; 0 before the first.
    pub checked_at: u64,
    /// Why the last look failed, when it did.
    pub error: Option<String>,
    /// An install is under way: the app is about to close.
    pub installing: bool,
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    installing: AtomicBool,
}

impl Updates {
    pub fn status(&self, app: &AppHandle) -> UpdateStatus {
        let mut status = self.status.lock().unwrap().clone();
        status.current = app.package_info().version.to_string();
        status.installing = self.installing.load(Ordering::Relaxed);
        status
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One sentence of an updater error, without the page of detail behind it.
fn brief(err: impl std::fmt::Display) -> String {
    let text = err.to_string();
    let line = text.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(200).collect();
    if out.is_empty() {
        out = "The update server could not be reached.".into();
    }
    out
}

/// What a failed look most likely means, when the bare error does not say it:
/// GitHub answers "not found" for a private repository the token cannot read.
fn explain(err: String) -> String {
    let lower = err.to_lowercase();
    if lower.contains("404") || lower.contains("401") || lower.contains("403") || lower.contains("valid release json") {
        "GitHub did not let this token read the builds. Check that it is still valid and may read the builds repository.".to_string()
    } else {
        err
    }
}

/// Tells Settings and the tray what is known now.
fn publish(app: &AppHandle) -> UpdateStatus {
    let status = app.state::<Updates>().status(app);
    let _ = app.emit("update-status", status.clone());
    crate::tray::show_update(app, status.available.as_deref());
    status
}

/// Asks the fork's releases whether a newer build exists.
pub async fn check(app: &AppHandle) -> UpdateStatus {
    let found = match updater(app) {
        Ok(updater) => match updater.check().await {
            Ok(Some(update)) if !download_allowed(&update.download_url) => {
                Err("The build list points somewhere other than GitHub. Nothing was downloaded.".to_string())
            }
            Ok(found) => Ok(found),
            Err(err) => {
                // What it really said, for the log: the sentence shown is a guess at why.
                crate::log::line(format!("update: the check answered: {}", brief(&err)));
                Err(explain(brief(err)))
            }
        },
        Err(err) => Err(err),
    };
    {
        let updates = app.state::<Updates>();
        let mut status = updates.status.lock().unwrap();
        status.checked_at = now_secs();
        match found {
            Ok(Some(update)) => {
                crate::log::line(format!("update: {} is available", update.version));
                status.available = Some(update.version.clone());
                status.notes = update.body.clone().filter(|b| !b.trim().is_empty());
                status.error = None;
            }
            Ok(None) => {
                status.available = None;
                status.notes = None;
                status.error = None;
            }
            Err(err) => {
                crate::log::line(format!("update: check failed: {err}"));
                // What was found earlier is still true; only the look failed.
                status.error = Some(err);
            }
        }
    }
    publish(app)
}

/// Downloads the newer build, checks its signature and runs its installer. On
/// Windows the installer takes over and the app closes; elsewhere it restarts.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    if updates.installing.swap(true, Ordering::SeqCst) {
        return Ok(()); // already under way
    }
    publish(app);
    let result = async {
        let mut update = updater(app)?
            .check()
            .await
            .map_err(|e| explain(brief(e)))?
            .ok_or_else(|| "This is already the newest build.".to_string())?;
        if !download_allowed(&update.download_url) {
            return Err("The build list points somewhere other than GitHub. Nothing was downloaded.".to_string());
        }
        // The same requests' headers go with the download: there it is the
        // installer's bytes that are asked for, not a file of the repository.
        update.headers.insert(ACCEPT, HeaderValue::from_static(ACCEPT_INSTALLER));
        crate::log::line(format!("update: installing {}", update.version));
        update.download_and_install(|_, _| {}, || {}).await.map_err(brief)
    }
    .await;
    match result {
        Ok(()) => {
            app.restart();
        }
        Err(err) => {
            crate::log::line(format!("update: install failed: {err}"));
            updates.installing.store(false, Ordering::SeqCst);
            updates.status.lock().unwrap().error = Some(err.clone());
            publish(app);
            Err(err)
        }
    }
}

/// Looks once shortly after launch and then a few times a day, while the user
/// has left "Check for updates" on. One small request each time; nothing else.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            let wanted = app
                .try_state::<crate::Shared>()
                .map(|s| s.settings.lock().unwrap().check_for_updates)
                .unwrap_or(false);
            // Without a token there is nothing to ask: Settings says so, quietly.
            if wanted && crate::secrets::present(TOKEN_KEY) {
                check(&app).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{brief, download_allowed, explain};

    #[test]
    fn the_token_goes_to_githubs_api_and_nowhere_else() {
        let url = |s: &str| tauri::Url::parse(s).unwrap();
        assert!(download_allowed(&url("https://api.github.com/repos/o/r/releases/assets/1")));
        assert!(!download_allowed(&url("http://api.github.com/repos/o/r/releases/assets/1")));
        assert!(!download_allowed(&url("https://api.github.com.evil.example/x")));
        assert!(!download_allowed(&url("https://evil.example/Coucou.msi")));
        assert!(!download_allowed(&url("https://github.com/o/r/releases/download/v1/x.msi")));
    }

    #[test]
    fn a_refusal_from_github_is_said_as_what_it_is() {
        assert!(explain("status 404 Not Found".into()).contains("token"));
        assert_eq!(explain("connection reset".into()), "connection reset");
    }

    #[test]
    fn an_error_is_one_short_line() {
        assert_eq!(brief("first\nsecond"), "first");
        assert_eq!(brief(""), "The update server could not be reached.");
        assert_eq!(brief("x".repeat(500)).chars().count(), 200);
    }
}
