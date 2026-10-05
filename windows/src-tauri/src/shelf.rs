// The shelf: files parked on the island to be taken away later — dragged back
// out, or copied to the clipboard.
//
// A folder of its own under %LOCALAPPDATA%\Coucou. Unlike the inbox nothing here
// is ever swept: a file stays until it is taken off the shelf.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::files::{self, DroppedFile};
use crate::{platform, settings};

/// The icon shown under the pointer while a file is dragged off the shelf.
const DRAG_ICON: &[u8] = include_bytes!("../icons/32x32.png");

pub fn dir() -> PathBuf {
    settings::local_dir().join("shelf")
}

/// What is on the shelf, oldest first.
pub fn list() -> Vec<DroppedFile> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut found: Vec<(std::time::SystemTime, DroppedFile)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let added = meta.modified().ok()?;
            Some((
                added,
                DroppedFile {
                    name: entry.file_name().to_string_lossy().to_string(),
                    path: entry.path().to_string_lossy().to_string(),
                    size: meta.len(),
                },
            ))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, file)| file).collect()
}

/// Puts a copy of `source` on the shelf.
pub fn add(source: &str) -> Result<DroppedFile, String> {
    files::copy_into(&dir(), source)
}

pub fn remove(path: &str) -> Result<(), String> {
    let file = on_shelf(path)?;
    std::fs::remove_file(file).map_err(|e| e.to_string())
}

/// Puts the file itself on the clipboard, so it can be pasted into a folder, a
/// chat or a mail like any copied file.
pub fn copy_to_clipboard(path: &str) -> Result<(), String> {
    let file = on_shelf(path)?;
    if !cfg!(windows) {
        return Err("Copying a file to the clipboard isn't available here yet.".into());
    }
    // The path travels in the environment, never in the command text, so
    // nothing in a file name can be read as PowerShell.
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", "Set-Clipboard -LiteralPath $env:COUCOU_CLIP"])
        .env("COUCOU_CLIP", &file);
    let status = platform::no_console(&mut cmd).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not copy the file.".into())
    }
}

/// Where the drag icon lives, written on first use.
pub fn drag_icon() -> Result<String, String> {
    let path = settings::local_dir().join("drag-icon.png");
    if !path.is_file() {
        platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
        std::fs::write(&path, DRAG_ICON).map_err(|e| e.to_string())?;
    }
    Ok(path.to_string_lossy().to_string())
}

/// The path, if it really is a file on the shelf. The page only ever hands back
/// paths we gave it, but nothing outside this folder may be deleted or copied
/// on its say-so.
fn on_shelf(path: &str) -> Result<PathBuf, String> {
    let file = Path::new(path).canonicalize().map_err(|_| "That file is no longer on the shelf.".to_string())?;
    let shelf = dir().canonicalize().map_err(|e| e.to_string())?;
    if file.is_file() && file.parent() == Some(shelf.as_path()) {
        Ok(file)
    } else {
        Err("That file is not on the shelf.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::on_shelf;

    #[test]
    fn nothing_outside_the_shelf_is_accepted() {
        // This test binary exists, is a file, and is not on the shelf.
        let exe = std::env::current_exe().unwrap();
        assert!(on_shelf(&exe.to_string_lossy()).is_err());
        assert!(on_shelf("").is_err());
        assert!(on_shelf("C:\\does\\not\\exist.txt").is_err());
    }
}
