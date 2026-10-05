// Windows: Win32 for the island window and the cursor, %APPDATA% for files.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use tauri::{AppHandle, Manager, WebviewWindow};

use ::windows::core::{BOOL, PWSTR};
use ::windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, HWND, LPARAM, LocalFree, POINT};
use ::windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use ::windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use ::windows::Win32::System::SystemInformation::GetLocalTime;
use ::windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetCursorPos, GetWindowLongPtrW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
    SetForegroundWindow, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_RESTORE,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use super::LocalTime;
use crate::island::WINDOW_LABEL;

/// File name of the Claude Code relay.
pub const HOOK_EXE: &str = "coucou-hook.exe";

/// Environment variable holding the home directory.
pub const HOME_VAR: &str = "USERPROFILE";

/// Keeps spawned helpers from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── Files ─────────────────────────────────────────────────────────────────────

/// %APPDATA%\Coucou — preferences.
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Coucou")
}

/// %LOCALAPPDATA%\Coucou — where coucou-hook.exe, the inbox and the log live.
pub fn local_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Coucou")
}

/// %APPDATA% and %LOCALAPPDATA% are already private to the user.
pub fn ensure_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Nothing to set up before the webview starts.
pub fn prepare_environment() {}

pub fn local_time() -> LocalTime {
    let t = unsafe { GetLocalTime() };
    LocalTime {
        year: t.wYear.into(),
        month: t.wMonth.into(),
        day: t.wDay.into(),
        hour: t.wHour.into(),
        minute: t.wMinute.into(),
        second: t.wSecond.into(),
    }
}

// ── Processes ─────────────────────────────────────────────────────────────────

/// Spawned helpers must never flash a console window.
pub fn no_console(cmd: &mut Command) -> &mut Command {
    cmd.creation_flags(CREATE_NO_WINDOW)
}

pub fn open_url(url: &str) {
    let _ = no_console(Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]))
        .spawn();
}

pub fn reveal_folder(path: &str) {
    let _ = Command::new("explorer").arg(path).spawn();
}

/// Our own `where`: walks %PATH% against %PATHEXT%, no shell involved.
/// Rust quotes arguments correctly for `.cmd`/`.bat` targets since 1.77, so
/// spawning `code.cmd` directly is safe.
pub fn find_on_path(stem: &str) -> Option<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{stem}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ── Session windows ───────────────────────────────────────────────────────────

/// A visible top-level window: (handle, owning process, title).
type TopWindow = (isize, u32, String);

/// Brings forward the window a Claude Code session runs in.
///
/// `pids` is the session's process ancestry, nearest first, as reported by
/// coucou-hook: the first of them that owns a window is the terminal or editor
/// hosting it. `console` is the classic console window, when there is one.
pub fn focus_session_window(pids: &[u32], console: Option<u64>, folder: &str) -> bool {
    let mut windows: Vec<TopWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(&mut windows as *mut _ as isize));
    }
    let target = pick_session_window(&windows, pids, folder).or_else(|| {
        let hwnd = console? as isize;
        let live = unsafe {
            let h = HWND(hwnd as *mut _);
            IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool()
        };
        live.then_some(hwnd)
    });
    let Some(hwnd) = target else { return false };
    unsafe {
        let hwnd = HWND(hwnd as *mut _);
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}

/// The nearest ancestor with a window wins. An editor keeps all its windows in
/// one process, so among those the one named after the project folder is taken;
/// failing that the first, which is the one used most recently.
fn pick_session_window(windows: &[TopWindow], pids: &[u32], folder: &str) -> Option<isize> {
    let folder = folder.to_lowercase();
    for pid in pids {
        let owned: Vec<&TopWindow> = windows.iter().filter(|w| w.1 == *pid).collect();
        let Some(first) = owned.first() else { continue };
        let named = owned
            .iter()
            .find(|w| !folder.is_empty() && w.2.to_lowercase().contains(&folder));
        return Some(named.unwrap_or(first).0);
    }
    None
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = unsafe { &mut *(lparam.0 as *mut Vec<TopWindow>) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let mut title = [0u16; 256];
        let len = unsafe { GetWindowTextW(hwnd, &mut title) };
        // Untitled windows are helpers — tooltips, IME, hidden owners.
        if len > 0 {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            let title = String::from_utf16_lossy(&title[..len as usize]);
            windows.push((hwnd.0 as isize, pid, title));
        }
    }
    true.into()
}

// ── Who we are ────────────────────────────────────────────────────────────────
//
// Named pipes share one machine-wide namespace, so the SID in the name is what
// keeps two accounts on the same machine from ever meeting on `coucou-*`.
// coucou-hook computes the same string (hook/src/win.rs) and additionally checks
// that the process serving the pipe really is us.

/// The SID of the account this process runs as, as `S-1-5-21-…`.
pub fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;

        // First call sizes the buffer, second fills it.
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }

        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut _)));
        sid
    }
}

// ── Cursor ────────────────────────────────────────────────────────────────────

/// The 60 Hz poll reads the cursor and flips click-through from it.
pub const CURSOR_POLL: bool = true;

/// Cursor position in physical screen pixels.
pub fn cursor_physical() -> Option<(f64, f64)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f64, p.y as f64))
}

/// True while the left mouse button is held — the only signal we get that a
/// drag might be in flight before it reaches the window.
pub fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

// ── Island window ─────────────────────────────────────────────────────────────

fn hwnd_of(win: &WebviewWindow) -> Option<HWND> {
    let raw = win.hwnd().ok()?.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut _))
}

/// True while the foreground window is being moved or resized by its title bar
/// or border — a held button that is not a drag the island should answer.
pub fn moving_window() -> bool {
    use ::windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO, GUI_INMOVESIZE};
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(0, &mut info).is_ok() && (info.flags.0 & GUI_INMOVESIZE.0) != 0 }
}

/// True while the shell is drawing a drag picture: something was picked up in
/// Explorer or in an app that uses the shell's drag helper, which is how files
/// are usually dragged. A text selection or a slider being dragged has none.
pub fn shell_drag_in_progress() -> bool {
    drag_image().is_some()
}

/// The window showing the picture of what is being dragged, if there is one.
fn drag_image() -> Option<HWND> {
    use ::windows::core::{w, PCWSTR};
    use ::windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
    // The picture is a top-level window of this class. A stale, hidden one can
    // linger between drags, so every one of them is checked.
    let mut after: Option<HWND> = None;
    for _ in 0..16 {
        let hwnd = unsafe { FindWindowExW(None, after, w!("SysDragImage"), PCWSTR::null()) }.ok()?;
        if hwnd.0.is_null() {
            return None;
        }
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return Some(hwnd);
        }
        after = Some(hwnd);
    }
    None
}

/// Puts the drag picture back above the island, which is topmost as well and
/// may have been raised over it.
pub fn raise_drag_image() {
    if let Some(hwnd) = drag_image() {
        raise_hwnd(hwnd);
    }
}

/// Puts the island back on top of the other always-on-top windows.
///
/// Every topmost window shares one band, and whichever was raised last wins.
/// A full-width status bar along the top of the screen is often topmost too, and
/// then covers the strip that wakes the island and takes dropped files. Raising
/// the island again hands that edge back to it. Never activates, moves or resizes.
pub fn raise_topmost(win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(win) else { return };
    raise_hwnd(hwnd);
}

fn raise_hwnd(hwnd: HWND) {
    use ::windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
}

/// Such bars can raise themselves again later (on a redraw, or coming back from
/// a full-screen app), so the island checks back every couple of seconds. One
/// cheap call, no wake-up of the page.
pub fn keep_topmost(app: &AppHandle) {
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else { return };
    let Some(hwnd) = hwnd_of(&win) else { return };
    let raw = hwnd.0 as isize; // HWND isn't Send; the handle itself is just a number
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        // Not over the picture of a file being carried: see `island::place`.
        if !shell_drag_in_progress() {
            raise_hwnd(HWND(raw as *mut _));
        }
    });
}

/// WS_EX_NOACTIVATE keeps clicks from stealing focus; WS_EX_TOOLWINDOW keeps the
/// island out of Alt-Tab.
pub fn make_non_activating(win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Temporarily allow activation so a text field inside the island can be typed in.
pub fn set_activating(win: &WebviewWindow, activating: bool) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = if activating {
            ex & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            ex | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Click-through here is the poll's WS_EX_TRANSPARENT toggle, not a region.
pub fn set_input_region(_win: &WebviewWindow, _rect: Option<(f64, f64, f64, f64)>) {}

#[cfg(test)]
mod tests {
    use super::pick_session_window;

    fn windows() -> Vec<super::TopWindow> {
        vec![
            (1, 700, "notes — Zed".into()),
            (2, 700, "coucou — Zed".into()),
            (3, 500, "Windows PowerShell".into()),
        ]
    }

    #[test]
    fn the_nearest_ancestor_with_a_window_wins() {
        // 900 and 800 are Claude Code and its shell: no windows of their own.
        assert_eq!(pick_session_window(&windows(), &[900, 800, 500, 700], ""), Some(3));
    }

    #[test]
    fn an_editor_window_is_picked_by_project_folder() {
        assert_eq!(pick_session_window(&windows(), &[900, 700], "Coucou"), Some(2));
    }

    #[test]
    fn an_unknown_folder_falls_back_to_the_most_recent_window() {
        assert_eq!(pick_session_window(&windows(), &[700], "elsewhere"), Some(1));
    }

    #[test]
    fn no_window_means_nothing_to_focus() {
        assert_eq!(pick_session_window(&windows(), &[900, 800], "coucou"), None);
    }
}
