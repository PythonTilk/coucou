//! Updates, for this build. (Local to this build: upstream ships no updater.)
//!
//! This build is not upstream's release: it is built from a fork, by that fork's
//! own workflow (.github/workflows/fork-release.yml), which publishes each build
//! as a release with a signed installer and a `latest.json` beside it. This
//! module asks that one address whether a newer build exists, and installs it
//! when the user says so. Nothing is ever installed on its own.
//!
//! The address and the public key are in tauri.conf.json (`plugins.updater`).
//! An installer whose signature does not match the key is refused by the plugin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

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

/// Tells Settings and the tray what is known now.
fn publish(app: &AppHandle) -> UpdateStatus {
    let status = app.state::<Updates>().status(app);
    let _ = app.emit("update-status", status.clone());
    crate::tray::show_update(app, status.available.as_deref());
    status
}

/// Asks the fork's releases whether a newer build exists.
pub async fn check(app: &AppHandle) -> UpdateStatus {
    let found = match app.updater() {
        Ok(updater) => updater.check().await.map_err(brief),
        Err(err) => Err(brief(err)),
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
        let update = app
            .updater()
            .map_err(brief)?
            .check()
            .await
            .map_err(brief)?
            .ok_or_else(|| "This is already the newest build.".to_string())?;
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
            if wanted {
                check(&app).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::brief;

    #[test]
    fn an_error_is_one_short_line() {
        assert_eq!(brief("first\nsecond"), "first");
        assert_eq!(brief(""), "The update server could not be reached.");
        assert_eq!(brief("x".repeat(500)).chars().count(), 200);
    }
}
