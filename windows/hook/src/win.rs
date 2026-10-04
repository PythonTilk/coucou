//! The little bit of Win32 the relay needs: who we are, and who is on the other
//! end of the pipe.
//!
//! Named pipes live in a machine-wide namespace, so `\\.\pipe\coucou-<name>` can
//! be created by *any* account that gets there first. Two defences, both cheap:
//! the pipe name carries our SID, and once connected we check the server process
//! really belongs to us before sending anything.

use std::time::{Duration, Instant};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

use crate::CONNECT_TIMEOUT;

/// `ERROR_PIPE_BUSY` — every instance is serving someone else right now. This is
/// the one error worth retrying: the server exists and a slot will free up.
const ERROR_PIPE_BUSY: i32 = 231;

/// `\\.\pipe\coucou-<sid>`. The SID keeps two accounts on the same machine from
/// ever meeting on the same pipe; the name falls back to the user name only if
/// the SID cannot be read at all, which should not happen.
fn pipe_path() -> String {
    let key = current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\coucou-{key}")
}

/// Opens the pipe. Retries only while the server is busy: any other error means
/// there is nothing to talk to, and waiting would only delay Claude Code.
pub fn connect() -> Option<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    let path = pipe_path();
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => {
                let handle = HANDLE(file.as_raw_handle());
                // Somebody else's server on our pipe name gets nothing from us.
                return pipe_server_is_same_user(handle).then_some(file);
            }
            Err(err) => {
                if err.raw_os_error() != Some(ERROR_PIPE_BUSY) || Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

/// The SID of the account this process runs as, as `S-1-5-21-…`.
pub fn current_user_sid() -> Option<String> {
    unsafe { token_sid(GetCurrentProcess()) }
}

/// True when the process serving `handle` runs as the same user we do.
///
/// A failure to answer is treated as "not ours": refusing to talk to a pipe we
/// cannot vouch for costs one hook event, while trusting it could hand another
/// account on this machine the contents of every tool call.
pub fn pipe_server_is_same_user(handle: HANDLE) -> bool {
    let Some(mine) = current_user_sid() else { return false };
    unsafe {
        let mut pid = 0u32;
        if GetNamedPipeServerProcessId(handle, &mut pid).is_err() || pid == 0 {
            return false;
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let theirs = token_sid(process);
        let _ = CloseHandle(process);
        theirs.as_deref() == Some(mine.as_str())
    }
}

/// The user SID behind a process handle. `process` is borrowed, never closed.
unsafe fn token_sid(process: HANDLE) -> Option<String> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;

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

/// How far up the process tree we look for the window hosting the session.
const MAX_ANCESTORS: usize = 16;

/// Our ancestors, nearest first: Claude Code, its shell, then the terminal or
/// editor that owns the window. Stops at the desktop shell — everything is a
/// descendant of explorer.exe, and its windows are never the session's.
pub fn ancestor_pids() -> Vec<u32> {
    // (pid, parent pid, exe name)
    let mut processes: Vec<(u32, u32, String)> = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return Vec::new();
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
                processes.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    ancestors_of(unsafe { GetCurrentProcessId() }, &processes)
}

fn ancestors_of(start: u32, processes: &[(u32, u32, String)]) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = start;
    while chain.len() < MAX_ANCESTORS {
        let Some(parent) = processes.iter().find(|p| p.0 == current).map(|p| p.1) else { break };
        let Some(entry) = processes.iter().find(|p| p.0 == parent) else { break };
        // A recycled pid can close the chain into a loop.
        if parent == 0 || parent == start || chain.contains(&parent) || entry.2 == "explorer.exe" {
            break;
        }
        chain.push(parent);
        current = parent;
    }
    chain
}

/// The classic console window, when there is one. Its owner is conhost.exe — a
/// child of the shell, not an ancestor of ours — so the process tree never finds
/// it. Under Windows Terminal this is a hidden stand-in, hence the visibility check.
pub fn console_window() -> Option<u64> {
    unsafe {
        let hwnd = GetConsoleWindow();
        if hwnd.0.is_null() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        Some(hwnd.0 as usize as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::ancestors_of;

    fn table(rows: &[(u32, u32, &str)]) -> Vec<(u32, u32, String)> {
        rows.iter().map(|r| (r.0, r.1, r.2.to_string())).collect()
    }

    #[test]
    fn walks_up_to_the_terminal_and_stops_at_the_shell() {
        let t = table(&[
            (10, 9, "coucou-hook.exe"),
            (9, 8, "claude.exe"),
            (8, 7, "pwsh.exe"),
            (7, 6, "windowsterminal.exe"),
            (6, 1, "explorer.exe"),
        ]);
        assert_eq!(ancestors_of(10, &t), vec![9, 8, 7]);
    }

    #[test]
    fn a_missing_parent_ends_the_chain() {
        let t = table(&[(10, 9, "coucou-hook.exe"), (9, 4242, "claude.exe")]);
        assert_eq!(ancestors_of(10, &t), vec![9]);
    }

    #[test]
    fn a_recycled_pid_cannot_loop_forever() {
        let t = table(&[(10, 9, "a.exe"), (9, 8, "b.exe"), (8, 9, "c.exe")]);
        assert_eq!(ancestors_of(10, &t), vec![9, 8]);
    }
}
