// The shelf: files and bits of text parked on the island to be taken away
// later — dragged back out, or copied to the clipboard.
//
// A folder of its own under %LOCALAPPDATA%\Coucou. Unlike the inbox nothing here
// is ever swept: something stays until it is taken off the shelf.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::files::{self, DroppedFile};
use crate::{platform, settings};

/// The icon shown under the pointer while a file is dragged off the shelf.
const DRAG_ICON: &[u8] = include_bytes!("../icons/32x32.png");

/// Pasted text is kept as a plain file with this name, which is how it is told
/// apart from a text file that was dropped: `Pasted text 2026-10-05 23.52.10.txt`.
const TEXT_PREFIX: &str = "Pasted text ";
const TEXT_SUFFIX: &str = ".txt";
/// How much of a pasted text the island is shown.
const PREVIEW_CHARS: usize = 80;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShelfEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// "file", or "text" for something that was pasted.
    pub kind: &'static str,
    /// The start of a pasted text, on one line.
    pub preview: Option<String>,
}

pub fn dir() -> PathBuf {
    settings::local_dir().join("shelf")
}

fn is_pasted_text(name: &str) -> bool {
    name.starts_with(TEXT_PREFIX) && name.ends_with(TEXT_SUFFIX)
}

/// The first few words of a text, whitespace collapsed.
fn preview_of(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= PREVIEW_CHARS {
        return flat;
    }
    let mut cut: String = flat.chars().take(PREVIEW_CHARS).collect();
    cut.push('…');
    cut
}

fn entry(path: &Path, size: u64) -> ShelfEntry {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let pasted = is_pasted_text(&name);
    ShelfEntry {
        preview: pasted
            .then(|| std::fs::read_to_string(path).ok())
            .flatten()
            .map(|text| preview_of(&text)),
        kind: if pasted { "text" } else { "file" },
        name,
        path: path.to_string_lossy().to_string(),
        size,
    }
}

/// What is on the shelf, oldest first.
pub fn list() -> Vec<ShelfEntry> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut found: Vec<(std::time::SystemTime, ShelfEntry)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, entry(&e.path(), meta.len())))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, e)| e).collect()
}

/// Puts a copy of `source` on the shelf. Like the inbox, the shelf only takes
/// paths a real drop just delivered: the page names the path, and without this
/// it could have any file the user can read copied.
pub fn add(source: &str) -> Result<ShelfEntry, String> {
    if !files::take_dropped(source) {
        return Err("Only files dropped on the island can be put on the shelf.".into());
    }
    let file = files::copy_into(&dir(), source)?;
    Ok(entry(Path::new(&file.path), file.size))
}

pub fn remove(path: &str) -> Result<(), String> {
    let file = on_shelf(path)?;
    std::fs::remove_file(file).map_err(|e| e.to_string())
}

/// Puts the thing back on the clipboard: a pasted text as text, a file as the
/// file itself, ready to paste into a folder, a chat or a mail.
pub fn copy_to_clipboard(path: &str) -> Result<(), String> {
    let file = on_shelf(path)?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let script = if is_pasted_text(&name) {
        "Set-Clipboard -Value ([IO.File]::ReadAllText($env:COUCOU_CLIP))"
    } else {
        "Set-Clipboard -LiteralPath $env:COUCOU_CLIP"
    };
    powershell(script, &[("COUCOU_CLIP", file.as_os_str())]).map(|_| ())
}

/// Takes what is on the clipboard onto the shelf: copied files as files, text
/// as a pasted text. Returns how many things arrived.
pub fn paste() -> Result<usize, String> {
    let shelf = dir();
    platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&shelf).map_err(|e| e.to_string())?;

    let t = platform::local_time();
    let text_file = shelf.join(format!(
        "{TEXT_PREFIX}{:04}-{:02}-{:02} {:02}.{:02}.{:02}{TEXT_SUFFIX}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    ));
    // Files win over text: copying a file in Explorer also puts its name on the
    // clipboard as text. The text is written by PowerShell straight to its file
    // and never passes through a pipe, so no encoding can mangle it.
    let out = powershell(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
         $f = Get-Clipboard -Format FileDropList; \
         if ($f) { $f | ForEach-Object { 'F:' + $_.FullName } } \
         else { $t = Get-Clipboard -Raw; \
           if ($t -and $t.Trim()) { [IO.File]::WriteAllText($env:COUCOU_OUT, $t, (New-Object Text.UTF8Encoding $false)); 'T:' } }",
        &[("COUCOU_OUT", text_file.as_os_str())],
    )?;

    let mut arrived = 0;
    for line in out.lines() {
        if let Some(source) = line.trim().strip_prefix("F:") {
            if files::copy_into(&shelf, source).is_ok() {
                arrived += 1;
            }
        } else if line.trim() == "T:" && text_file.is_file() {
            arrived += 1;
        }
    }
    if arrived == 0 {
        return Err("Nothing to paste: the clipboard holds no text and no files.".into());
    }
    Ok(arrived)
}

/// What the chat takes when something other than text is pasted into it: the
/// first copied file, or a copied picture (a screenshot), saved as a PNG. It
/// lands in the inbox like a dropped file. None when the clipboard holds neither.
pub fn clipboard_file() -> Result<Option<DroppedFile>, String> {
    let inbox = files::inbox_dir();
    platform::ensure_private_dir(&settings::local_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&inbox).map_err(|e| e.to_string())?;

    let t = platform::local_time();
    let name = format!(
        "Pasted image {:04}-{:02}-{:02} {:02}.{:02}.{:02}.png",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let image = inbox.join(&name);
    let out = powershell(
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8;          $f = Get-Clipboard -Format FileDropList;          if ($f) { 'F:' + @($f)[0].FullName }          else { $i = Get-Clipboard -Format Image;            if ($i) { $i.Save($env:COUCOU_OUT, [System.Drawing.Imaging.ImageFormat]::Png); 'I:' } }",
        &[("COUCOU_OUT", image.as_os_str())],
    )?;

    for line in out.lines() {
        let line = line.trim();
        if let Some(source) = line.strip_prefix("F:") {
            let file = files::copy_into(&inbox, source)?;
            files::sweep_inbox();
            return Ok(Some(file));
        }
        if line == "I:" && image.is_file() {
            let size = std::fs::metadata(&image).map(|m| m.len()).unwrap_or(0);
            files::sweep_inbox();
            return Ok(Some(DroppedFile { name, path: image.to_string_lossy().to_string(), size }));
        }
    }
    Ok(None)
}

/// Runs a fixed script. Anything that varies travels in the environment, never
/// in the command text, so nothing in a file name can be read as PowerShell.
fn powershell(script: &str, env: &[(&str, &std::ffi::OsStr)]) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("The clipboard isn't available here yet.".into());
    }
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = platform::no_console(&mut cmd).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Could not reach the clipboard.".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
    let file = Path::new(path).canonicalize().map_err(|_| "That is no longer on the shelf.".to_string())?;
    let shelf = dir().canonicalize().map_err(|e| e.to_string())?;
    if file.is_file() && file.parent() == Some(shelf.as_path()) {
        Ok(file)
    } else {
        Err("That is not on the shelf.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_outside_the_shelf_is_accepted() {
        // This test binary exists, is a file, and is not on the shelf.
        let exe = std::env::current_exe().unwrap();
        assert!(on_shelf(&exe.to_string_lossy()).is_err());
        assert!(on_shelf("").is_err());
        assert!(on_shelf("C:\\does\\not\\exist.txt").is_err());
    }

    #[test]
    fn only_our_own_naming_counts_as_pasted_text() {
        assert!(is_pasted_text("Pasted text 2026-10-05 23.52.10.txt"));
        assert!(!is_pasted_text("notes.txt"));
        assert!(!is_pasted_text("Pasted text 2026-10-05.pdf"));
    }

    #[test]
    fn a_preview_is_one_short_line() {
        assert_eq!(preview_of("  two\n\n lines\tof text "), "two lines of text");
        let long = "word ".repeat(60);
        let shown = preview_of(&long);
        assert_eq!(shown.chars().count(), PREVIEW_CHARS + 1);
        assert!(shown.ends_with('…'));
        // Cut on a character, never inside one.
        assert!(preview_of(&"é".repeat(200)).ends_with('…'));
    }

    #[test]
    fn a_pasted_text_is_listed_with_its_preview() {
        let tmp = std::env::temp_dir().join(format!("coucou-shelf-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let text = tmp.join("Pasted text 2026-10-05 23.52.10.txt");
        std::fs::write(&text, "Exam on Friday,\nroom B 051").unwrap();
        let file = tmp.join("sheet3.pdf");
        std::fs::write(&file, b"%PDF").unwrap();

        let pasted = entry(&text, 26);
        assert_eq!(pasted.kind, "text");
        assert_eq!(pasted.preview.as_deref(), Some("Exam on Friday, room B 051"));
        let dropped = entry(&file, 4);
        assert_eq!(dropped.kind, "file");
        assert_eq!(dropped.preview, None);
        assert_eq!(dropped.name, "sheet3.pdf");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
