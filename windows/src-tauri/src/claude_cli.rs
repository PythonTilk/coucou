// Chat through Claude Code instead of an API: one `claude -p` per turn, on the
// account Claude Code is already signed in to. No key involved.
//
// One more provider for chat.rs, next to Anthropic's API and the others. Claude
// Code keeps the conversation itself; we remember its session id and resume it
// on the next turn (chat.rs knows when that session is still the whole story).
//
// Local to this build: upstream declined it (#113, #138).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::chat::{self, Chat, ChatContext, ChatReply, ModelInfo};
use crate::platform;

/// `Settings::chat_provider` for this provider; matches src/core/providers.ts.
pub const PROVIDER: &str = "claudecode";
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

/// A turn with a few web searches in it can take a while.
const TIMEOUT: Duration = Duration::from_secs(180);
/// Mochi can look things up and read a dropped file. Nothing that writes or runs.
const TOOLS: &str = "WebSearch,WebFetch,Read";

const NOT_INSTALLED: &str =
    "Claude Code was not found. Install it, run `claude` once to sign in, then try again.";

/// What the picker offers. Claude Code takes any model ID; these are the usual ones.
pub fn models() -> Vec<ModelInfo> {
    [
        ("claude-opus-5-5", "Claude Opus 5.5"),
        ("claude-opus-5", "Claude Opus 5"),
        ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
        ("claude-sonnet-5", "Claude Sonnet 5"),
        ("claude-haiku-4-5", "Claude Haiku 4.5"),
        ("claude-fable-5-1", "Claude Fable 5.1"),
    ]
    .iter()
    .map(|(id, label)| ModelInfo { id: id.to_string(), label: label.to_string() })
    .collect()
}

/// Where the `claude` launcher is: on PATH, or where the native installer puts it.
pub fn find_claude() -> Option<PathBuf> {
    platform::find_on_path("claude").or_else(|| {
        let bin = platform::home_dir().join(".local").join("bin");
        ["claude.exe", "claude"].iter().map(|name| bin.join(name)).find(|p| p.is_file())
    })
}

pub async fn send(
    chat: &Chat,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let exe = find_claude().ok_or_else(|| NOT_INSTALLED.to_string())?;
    let turn = chat.begin(PROVIDER);
    let resume = chat.cli_session(&turn);

    // File / window context rides along with the first message only.
    let context = context.filter(|_| turn.first);
    let user_text = chat::plain_question(turn.first, context.as_ref(), &query);
    let (mut prompt, add_dir) = build_prompt(&query, context.as_ref());
    // Another provider answered the turns before this one: the new session is
    // told what was said.
    if resume.is_none() && !turn.history.is_empty() {
        prompt = format!("{}\n\n{prompt}", transcript(&turn.history));
    }

    // A folder of our own, so no project's CLAUDE.md leaks into Mochi's answers.
    let dir = platform::local_dir().join("chat");
    platform::ensure_private_dir(&dir).map_err(|e| e.to_string())?;
    // A file rather than an argument: the prompt has newlines and punctuation
    // that a `.cmd` launcher would not survive.
    let system_prompt = dir.join("system-prompt.txt");
    std::fs::write(&system_prompt, chat::system_prompt(true)).map_err(|e| e.to_string())?;

    let args = build_args(model, &system_prompt, resume.as_deref(), add_dir.as_deref());
    let stdout = tauri::async_runtime::spawn_blocking(move || run(&exe, &args, &dir, &prompt))
        .await
        .map_err(|e| e.to_string())??;

    let (text, session) = parse_reply(&stdout)?;
    chat.commit(
        &turn,
        json!({ "role": "user", "content": user_text }),
        json!({ "role": "assistant", "content": text }),
        &user_text,
        &text,
    );
    if let Some(id) = session {
        chat.set_cli_session(&turn, id);
    }
    Ok(ChatReply { text })
}

/// The earlier turns as plain text, for a session that was not there for them.
fn transcript(history: &[Value]) -> String {
    let mut out = String::from("Earlier in this conversation:");
    for turn in history {
        let who = if turn["role"] == "assistant" { "You" } else { "User" };
        if let Some(text) = turn["content"].as_str() {
            out.push_str(&format!("\n\n{who}: {text}"));
        }
    }
    out
}

fn build_args(
    model: &str,
    system_prompt: &Path,
    resume: Option<&str>,
    add_dir: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "json",
        "--model",
        model,
        "--tools",
        TOOLS,
        "--allowedTools",
        TOOLS,
        // None of the user's own hooks, plugins or MCP servers: this is Mochi,
        // not one of their sessions, and it should start fast. (No hooks also
        // means the relay never shows this turn as a session on the island.)
        "--setting-sources",
        "",
        "--strict-mcp-config",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push("--system-prompt-file".into());
    args.push(system_prompt.to_string_lossy().to_string());
    if let Some(id) = resume {
        args.push("--resume".into());
        args.push(id.to_string());
    }
    if let Some(dir) = add_dir {
        args.push("--add-dir".into());
        args.push(dir.to_string_lossy().to_string());
    }
    args
}

/// The text sent on stdin, and the folder Claude Code must be allowed to read
/// when a file came along.
fn build_prompt(query: &str, context: Option<&ChatContext>) -> (String, Option<PathBuf>) {
    match context {
        Some(ChatContext::File { name, path }) => {
            let file = Path::new(path);
            if !(file.is_absolute() && file.is_file()) {
                return (query.to_string(), None);
            }
            let prompt = format!(
                "The user dropped a file: {name}\nIt is saved at: {path}\nRead it before answering.\n\n{query}"
            );
            (prompt, file.parent().map(Path::to_path_buf))
        }
        Some(ChatContext::Window { app_name, title, url }) => {
            (format!("{}\n\n{query}", chat::window_line(app_name, title, url.as_deref())), None)
        }
        None => (query.to_string(), None),
    }
}

/// Runs one turn and returns what Claude Code printed.
fn run(exe: &Path, args: &[String], dir: &Path, prompt: &str) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .current_dir(dir)
        // A key in the environment would be billed instead of the subscription.
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = platform::no_console(&mut cmd)
        .spawn()
        .map_err(|e| format!("Could not start Claude Code: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
    }
    // Read on a thread so a long answer cannot fill the pipe while we wait.
    let mut stdout = child.stdout.take().ok_or("Could not read from Claude Code.")?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });

    let deadline = Instant::now() + TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Claude Code took too long to answer.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    reader.join().map_err(|_| "Could not read from Claude Code.".to_string())
}

/// `claude -p --output-format json` prints one result object. Returns the answer
/// and the session id to resume, or the message to show in the note view.
fn parse_reply(stdout: &str) -> Result<(String, Option<String>), String> {
    let reply: Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        let shown: String = stdout.trim().chars().take(200).collect();
        if shown.is_empty() {
            "Claude Code returned nothing. Run `claude` in a terminal to check you are signed in."
                .to_string()
        } else {
            format!("Claude Code: {shown}")
        }
    })?;

    let text = reply.get("result").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if reply.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        return Err(error_message(&text));
    }
    if text.is_empty() {
        return Err("No response text.".into());
    }
    let session = reply.get("session_id").and_then(Value::as_str).map(str::to_string);
    Ok((text, session))
}

/// An API error arrives as `API Error: 400 {…json…}`. The message inside is the
/// useful part — "this model needs a newer Claude Code", say.
fn error_message(text: &str) -> String {
    let inner = text
        .find('{')
        .and_then(|start| serde_json::from_str::<Value>(&text[start..]).ok())
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    match inner {
        Some(message) => message,
        None if text.is_empty() => "Claude Code reported an error.".into(),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_turn_has_no_resume_and_later_turns_do() {
        let prompt = Path::new("sp.txt");
        let first = build_args("claude-sonnet-5", prompt, None, None);
        assert!(!first.contains(&"--resume".to_string()));
        assert_eq!(first[0], "-p");
        let at = first.iter().position(|a| a == "--model").unwrap();
        assert_eq!(first[at + 1], "claude-sonnet-5");

        let later = build_args("claude-sonnet-5", prompt, Some("abc-123"), None);
        let at = later.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(later[at + 1], "abc-123");
    }

    #[test]
    fn only_read_only_tools_are_offered() {
        let args = build_args("m", Path::new("sp.txt"), None, None);
        for flag in ["--tools", "--allowedTools"] {
            let at = args.iter().position(|a| a == flag).unwrap();
            assert_eq!(args[at + 1], "WebSearch,WebFetch,Read");
        }
    }

    #[test]
    fn a_dropped_file_is_named_and_its_folder_allowed() {
        let file = std::env::current_exe().unwrap();
        let path = file.to_string_lossy().to_string();
        let context = ChatContext::File { name: "notes.pdf".into(), path: path.clone() };
        let (prompt, dir) = build_prompt("What is this?", Some(&context));
        assert!(prompt.contains("notes.pdf"));
        assert!(prompt.contains(&path));
        assert!(prompt.ends_with("What is this?"));
        assert_eq!(dir.as_deref(), file.parent());
    }

    #[test]
    fn a_file_that_is_not_there_is_left_out() {
        let context = ChatContext::File { name: "x".into(), path: "not/absolute.txt".into() };
        assert_eq!(build_prompt("Hi", Some(&context)), ("Hi".to_string(), None));
    }

    #[test]
    fn a_successful_reply_gives_text_and_session() {
        let out = r#"{"type":"result","is_error":false,"result":" ok \n","session_id":"s-1"}"#;
        assert_eq!(parse_reply(out), Ok(("ok".to_string(), Some("s-1".to_string()))));
    }

    #[test]
    fn an_api_error_shows_the_message_inside() {
        let out = r#"{"is_error":true,"result":"API Error: 400 {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"Claude Code 2.1.90 does not support this model.\"}}"}"#;
        assert_eq!(
            parse_reply(out),
            Err("Claude Code 2.1.90 does not support this model.".to_string())
        );
    }

    #[test]
    fn a_plain_error_is_shown_as_is() {
        let out = r#"{"is_error":true,"result":"Invalid API key · Please run /login"}"#;
        assert_eq!(parse_reply(out), Err("Invalid API key · Please run /login".to_string()));
    }

    #[test]
    fn output_that_is_not_json_is_reported() {
        assert!(parse_reply("boom").unwrap_err().contains("boom"));
        assert!(parse_reply("").unwrap_err().contains("signed in"));
    }

    #[test]
    fn a_session_is_resumed_only_while_it_holds_the_whole_conversation() {
        let chat = Chat::default();
        let commit = |turn: &chat::Turn, provider_text: &str| {
            chat.commit(
                turn,
                json!({ "role": "user", "content": "q" }),
                json!({ "role": "assistant", "content": provider_text }),
                "q",
                provider_text,
            )
        };

        // First turn: nothing to resume.
        let first = chat.begin(PROVIDER);
        assert_eq!(chat.cli_session(&first), None);
        commit(&first, "a1");
        chat.set_cli_session(&first, "s-1".into());

        // Second turn with Claude Code: the session has seen it all.
        let second = chat.begin(PROVIDER);
        assert_eq!(chat.cli_session(&second), Some("s-1".to_string()));
        commit(&second, "a2");
        chat.set_cli_session(&second, "s-1".into());

        // Another provider answers a turn: the session is now behind.
        let other = chat.begin("openai");
        commit(&other, "a3");
        let back = chat.begin(PROVIDER);
        assert_eq!(chat.cli_session(&back), None);
        assert!(transcript(&back.history).contains("You: a3"));

        // "New chat" forgets it too.
        chat.reset();
        assert_eq!(chat.cli_session(&chat.begin(PROVIDER)), None);
    }
}
