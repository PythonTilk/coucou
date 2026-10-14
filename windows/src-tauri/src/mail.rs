//! The Mail pill: the newest messages of an inbox, read over IMAP. (Local to
//! this build. The Mac's Mail pill reads Apple Mail through Apple Events, which
//! Windows does not have; this reads the mailbox itself.)
//!
//! Read-only by construction: the inbox is opened with EXAMINE and the headers
//! are fetched with PEEK, so nothing is ever marked as read, moved or deleted.
//! Only the sender, the subject, the date and the flags are asked for — never a
//! message's body. One short connection per look, over TLS checked against the
//! usual certificate authorities; the address and the app password live in the
//! OS keystore.
//!
//! The protocol is spoken by hand: the few commands needed do not justify a
//! mail library, and everything that comes back is bounded before it is kept.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

/// Gmail's, when the user names no other server.
pub const DEFAULT_HOST: &str = "imap.gmail.com";

/// The keystore entries of the Mail pill.
pub const ADDRESS_KEY: &str = "mail-address";
pub const PASSWORD_KEY: &str = "mail-password";
pub const HOST_KEY: &str = "mail-host";

/// The server the stored host names: Gmail's when there is none.
pub fn host_of(stored: Option<&str>) -> String {
    match stored.map(str::trim).filter(|h| !h.is_empty()) {
        Some(host) => host.to_ascii_lowercase(),
        None => DEFAULT_HOST.to_string(),
    }
}

/// The password entry as it is stored: the server it was entered for, a line
/// break, then the password. Kept together so that where it is sent and what is
/// sent are always read from the same place, in one read.
pub fn bind(host: &str, password: &str) -> Result<String, String> {
    if password.is_empty() || has_line_break(password) {
        return Err("The app password is not valid.".into());
    }
    let host = host_of(Some(host));
    if !valid_host(&host) {
        return Err("The mail server's name is not valid.".into());
    }
    Ok(format!("{host}\n{password}"))
}

/// The server and the password of a stored entry. None for an entry that does
/// not say its server (none was ever stored that way; it is asked for again).
pub fn bound(stored: &str) -> Option<(String, String)> {
    let (host, password) = stored.split_once('\n')?;
    (valid_host(host) && !password.is_empty()).then(|| (host.to_string(), password.to_string()))
}

/// True when storing `new_host` would send the password somewhere other than
/// where it goes today. A password belongs to the server it was entered for:
/// when the server changes, the password is forgotten and asked for again, so
/// nothing that can change the server can redirect a password already stored.
pub fn host_change_forgets_password(old_host: Option<&str>, new_host: Option<&str>) -> bool {
    host_of(old_host) != host_of(new_host)
}
const PORT: u16 = 993;
/// How many of the newest messages are read.
const NEWEST: u32 = 12;
/// One look may take this long, connection included.
const BUDGET: Duration = Duration::from_secs(20);
/// A response line, a header block, and all the lines of one answer, at most.
const MAX_LINE: usize = 16 * 1024;
const MAX_LITERAL: usize = 16 * 1024;
const MAX_LINES: usize = 400;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub from: String,
    pub subject: String,
    /// When the server received it, epoch milliseconds (0 when it did not say).
    pub date_ms: i64,
    pub unread: bool,
    /// Gmail's own id of the conversation, as it appears in a Gmail address.
    pub thread: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Inbox {
    pub unread: u32,
    /// Newest first.
    pub messages: Vec<Message>,
}

/// One untagged or tagged answer: its text, and the literals it carried.
#[derive(Debug, Default)]
struct Answer {
    text: String,
    literals: Vec<Vec<u8>>,
}

/// Reads the inbox. An error is one sentence fit to show on the card; it never
/// contains the password or anything the server echoed of the login.
pub async fn fetch(host: &str, address: &str, password: &str) -> Result<Inbox, String> {
    if !valid_host(host) {
        return Err("The mail server's name is not valid.".into());
    }
    if address.is_empty() || password.is_empty() || has_line_break(address) || has_line_break(password) {
        return Err("The address or the app password is not valid.".into());
    }
    match tokio::time::timeout(BUDGET, look(host, address, password)).await {
        Ok(result) => result,
        Err(_) => Err("The mail server took too long to answer.".into()),
    }
}

async fn look(host: &str, address: &str, password: &str) -> Result<Inbox, String> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // The provider is named: the process may hold more than one.
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| "TLS could not be set up.".to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string()).map_err(|_| "The mail server's name is not valid.".to_string())?;
    let tcp = TcpStream::connect((host, PORT)).await.map_err(|_| "Could not reach the mail server.".to_string())?;
    let tls = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|_| "The mail server's certificate could not be verified.".to_string())?;
    let mut imap = Imap { stream: BufReader::new(tls), tag: 0 };

    let greeting = imap.read_answer().await?;
    if !greeting.text.starts_with("* OK") {
        return Err("The mail server did not greet as expected.".into());
    }

    // The password is sent as a quoted string, inside TLS, and nowhere else.
    let login = format!("LOGIN {} {}", quoted(address), quoted(password));
    let (ok, _) = imap.command(&login).await?;
    if !ok {
        return Err("The mail server refused the address or the app password.".into());
    }

    // Gmail says so in its capabilities; its conversation ids exist only there.
    let (_, capabilities) = imap.command("CAPABILITY").await?;
    let gmail = capabilities.iter().any(|a| a.text.to_ascii_uppercase().contains("X-GM-EXT-1"));

    let (ok, status) = imap.command("STATUS INBOX (UNSEEN)").await?;
    let unread = if ok { status.iter().find_map(|a| number_after(&a.text, "UNSEEN")).unwrap_or(0) } else { 0 };

    // EXAMINE, not SELECT: the mailbox is opened read-only.
    let (ok, examined) = imap.command("EXAMINE INBOX").await?;
    if !ok {
        return Err("The inbox could not be opened.".into());
    }
    let exists = examined.iter().find_map(|a| count_before(&a.text, "EXISTS")).unwrap_or(0);

    let mut messages = Vec::new();
    if exists > 0 {
        let first = exists.saturating_sub(NEWEST - 1).max(1);
        let items = if gmail {
            "(FLAGS INTERNALDATE X-GM-THRID BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])"
        } else {
            "(FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])"
        };
        let (ok, fetched) = imap.command(&format!("FETCH {first}:{exists} {items}")).await?;
        if !ok {
            return Err("The messages could not be read.".into());
        }
        messages = fetched.iter().filter_map(message_of).collect();
        // The server lists them oldest first.
        messages.reverse();
    }

    let _ = imap.command("LOGOUT").await;
    Ok(Inbox { unread, messages })
}

struct Imap<S> {
    stream: BufReader<S>,
    tag: u32,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> Imap<S> {
    /// Sends one command and reads up to its tagged answer. Returns whether it
    /// was OK, and the untagged answers that came before.
    async fn command(&mut self, command: &str) -> Result<(bool, Vec<Answer>), String> {
        self.tag += 1;
        let tag = format!("c{}", self.tag);
        let line = format!("{tag} {command}\r\n");
        self.stream.get_mut().write_all(line.as_bytes()).await.map_err(|_| lost())?;
        self.stream.get_mut().flush().await.map_err(|_| lost())?;
        let mut untagged = Vec::new();
        for _ in 0..MAX_LINES {
            let answer = self.read_answer().await?;
            if let Some(rest) = answer.text.strip_prefix(&format!("{tag} ")) {
                return Ok((rest.to_ascii_uppercase().starts_with("OK"), untagged));
            }
            untagged.push(answer);
        }
        Err("The mail server sent more than expected.".into())
    }

    /// One answer: a line, and for each `{n}` it ends with, the n bytes that
    /// follow and the rest of the line after them.
    async fn read_answer(&mut self) -> Result<Answer, String> {
        let mut answer = Answer::default();
        loop {
            let mut raw = Vec::new();
            let read = (&mut self.stream)
                .take(MAX_LINE as u64 + 1)
                .read_until(b'\n', &mut raw)
                .await
                .map_err(|_| lost())?;
            if read == 0 {
                return Err(lost());
            }
            if raw.len() > MAX_LINE || !raw.ends_with(b"\n") {
                return Err("The mail server sent a line that is too long.".into());
            }
            let line = String::from_utf8_lossy(&raw).trim_end_matches(['\r', '\n']).to_string();
            let literal = literal_size(&line);
            answer.text.push_str(&line);
            if answer.text.len() > MAX_LINE * 2 {
                return Err("The mail server sent more than expected.".into());
            }
            let Some(size) = literal else { return Ok(answer) };
            if size > MAX_LITERAL || answer.literals.len() >= 4 {
                return Err("The mail server sent more than expected.".into());
            }
            let mut bytes = vec![0u8; size];
            self.stream.read_exact(&mut bytes).await.map_err(|_| lost())?;
            answer.literals.push(bytes);
        }
    }
}

fn lost() -> String {
    "The connection to the mail server was lost.".to_string()
}

fn has_line_break(text: &str) -> bool {
    text.contains('\r') || text.contains('\n')
}

/// A host name: letters, digits, dots and dashes, nothing a URL or a command could hide in.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// An IMAP quoted string. The caller has refused line breaks already.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The `n` of a line ending in `{n}`: a literal of n bytes follows.
fn literal_size(line: &str) -> Option<usize> {
    let open = line.rfind('{')?;
    line.strip_suffix('}')?.get(open + 1..)?.parse().ok()
}

/// `… UNSEEN 4)` → 4.
fn number_after(text: &str, word: &str) -> Option<u32> {
    let at = text.to_ascii_uppercase().find(word)? + word.len();
    text[at..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// `* 123 EXISTS` → 123.
fn count_before(text: &str, word: &str) -> Option<u32> {
    let mut parts = text.split_whitespace();
    if parts.next()? != "*" {
        return None;
    }
    let count = parts.next()?.parse().ok()?;
    parts.next()?.eq_ignore_ascii_case(word).then_some(count)
}

/// What sits between `KEY (` and the next `)`.
fn list_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(key)? + key.len();
    let rest = text[at..].trim_start().strip_prefix('(')?;
    Some(&rest[..rest.find(')')?])
}

/// What sits between the quotes after `KEY`.
fn quoted_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(key)? + key.len();
    let rest = text[at..].trim_start().strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

fn message_of(answer: &Answer) -> Option<Message> {
    if !answer.text.contains(" FETCH ") {
        return None;
    }
    let headers = answer.literals.first().map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
    let unread = !list_after(&answer.text, "FLAGS").unwrap_or("").split_whitespace().any(|f| f.eq_ignore_ascii_case("\\Seen"));
    let date_ms = quoted_after(&answer.text, "INTERNALDATE").and_then(internal_date).unwrap_or(0);
    let thread = number_text_after(&answer.text, "X-GM-THRID").and_then(|n| n.parse::<u64>().ok()).map(|n| format!("{n:x}"));
    Some(Message {
        from: sender(&header(&headers, "from")),
        subject: clip(&decode_words(&header(&headers, "subject")), 160),
        date_ms,
        unread,
        thread,
    })
}

fn number_text_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let at = text.find(key)? + key.len();
    let rest = text[at..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// `14-Oct-2026 01:02:03 +0000` → epoch milliseconds.
fn internal_date(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_str(text.trim(), "%d-%b-%Y %H:%M:%S %z").ok().map(|d| d.timestamp_millis())
}

/// The value of one header, its folded lines joined.
fn header(block: &str, name: &str) -> String {
    let mut value = String::new();
    let mut inside = false;
    for line in block.lines() {
        if line.starts_with([' ', '\t']) {
            if inside {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        inside = false;
        if let Some((key, rest)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                inside = true;
                value = rest.trim().to_string();
            }
        }
    }
    value
}

/// `"Ada Lovelace" <ada@example.org>` → `Ada Lovelace`; a bare address stays.
fn sender(from: &str) -> String {
    let decoded = decode_words(from);
    let name = match decoded.find('<') {
        Some(at) => decoded[..at].trim().trim_matches('"').trim().to_string(),
        None => String::new(),
    };
    let shown = if name.is_empty() {
        decoded.trim().trim_matches(['<', '>']).to_string()
    } else {
        name
    };
    clip(&shown, 80)
}

fn clip(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= max {
        clean
    } else {
        let mut out: String = clean.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

/// RFC 2047: `=?UTF-8?B?…?=` and `=?UTF-8?Q?…?=` inside a header, as text.
/// UTF-8 and the Latin-1 family are read; another charset is read as UTF-8,
/// with what does not fit replaced.
pub fn decode_words(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut last_was_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, tail) = rest.split_at(start);
        match encoded_word(tail) {
            Some((decoded, used)) => {
                // Blanks between two encoded words are not part of the text.
                if !(last_was_word && before.trim().is_empty()) {
                    out.push_str(before);
                }
                out.push_str(&decoded);
                rest = &tail[used..];
                last_was_word = true;
            }
            None => {
                out.push_str(before);
                out.push_str("=?");
                rest = &tail[2..];
                last_was_word = false;
            }
        }
    }
    out.push_str(rest);
    out
}

/// One encoded word at the start of `text`: what it says, and how long it is.
fn encoded_word(text: &str) -> Option<(String, usize)> {
    let inner = text.strip_prefix("=?")?;
    let mut parts = inner.splitn(3, '?');
    let charset = parts.next()?;
    let encoding = parts.next()?;
    let tail = parts.next()?;
    let end = tail.find("?=")?;
    let payload = &tail[..end];
    let bytes = match encoding {
        "B" | "b" => base64(payload)?,
        "Q" | "q" => q_encoded(payload),
        _ => return None,
    };
    let used = 2 + charset.len() + 1 + encoding.len() + 1 + end + 2;
    let lower = charset.to_ascii_lowercase();
    let decoded = if lower.starts_with("iso-8859-1") || lower == "windows-1252" || lower == "latin1" || lower == "us-ascii" {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };
    Some((decoded, used))
}

fn q_encoded(text: &str) -> Vec<u8> {
    let raw = text.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'_' => out.push(b' '),
            b'=' if i + 2 < raw.len() => {
                let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'='),
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    out
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fetch_answer_becomes_a_message() {
        let answer = Answer {
            text: "* 12 FETCH (X-GM-THRID 1827349873498734 FLAGS (\\Flagged) INTERNALDATE \"14-Oct-2026 01:02:03 +0000\" BODY[HEADER.FIELDS (FROM SUBJECT)] {64})".into(),
            literals: vec![b"From: \"Ada Lovelace\" <ada@example.org>\r\nSubject: Notes on\r\n the engine\r\n\r\n".to_vec()],
        };
        let message = message_of(&answer).unwrap();
        assert_eq!(message.from, "Ada Lovelace");
        assert_eq!(message.subject, "Notes on the engine");
        assert!(message.unread);
        assert_eq!(message.thread, Some(format!("{:x}", 1827349873498734u64)));
        assert_eq!(message.date_ms, internal_date("14-Oct-2026 01:02:03 +0000").unwrap());
        assert!(message.date_ms > 1_700_000_000_000);
        // Read, no Gmail id, a bare address.
        let seen = Answer {
            text: "* 3 FETCH (FLAGS (\\Seen \\Answered) INTERNALDATE \" 4-Oct-2026 10:00:00 +0200\" BODY[HEADER.FIELDS (FROM SUBJECT)] {30})".into(),
            literals: vec![b"From: bob@example.org\r\n\r\n".to_vec()],
        };
        let message = message_of(&seen).unwrap();
        assert!(!message.unread);
        assert_eq!(message.from, "bob@example.org");
        assert_eq!(message.subject, "");
        assert_eq!(message.thread, None);
        assert!(message.date_ms > 0);
        assert!(message_of(&Answer { text: "* 4 EXISTS".into(), literals: vec![] }).is_none());
    }

    #[test]
    fn counts_are_read_from_their_answers() {
        assert_eq!(count_before("* 123 EXISTS", "EXISTS"), Some(123));
        assert_eq!(count_before("* 0 RECENT", "EXISTS"), None);
        assert_eq!(number_after("* STATUS \"INBOX\" (UNSEEN 4)", "UNSEEN"), Some(4));
        assert_eq!(number_after("* STATUS INBOX (MESSAGES 9)", "UNSEEN"), None);
        assert_eq!(literal_size("* 1 FETCH (BODY[] {57}"), Some(57));
        assert_eq!(literal_size("* 1 FETCH (FLAGS ())"), None);
    }

    #[test]
    fn encoded_words_are_read_as_text() {
        assert_eq!(decode_words("=?UTF-8?B?R3LDvMOfZQ==?="), "Grüße");
        assert_eq!(decode_words("=?utf-8?Q?Gr=C3=BC=C3=9Fe_aus_Wien?="), "Grüße aus Wien");
        assert_eq!(decode_words("=?ISO-8859-1?Q?Gr=FC=DFe?="), "Grüße");
        // Two words in a row are one text; plain text around them stays.
        assert_eq!(decode_words("Re: =?UTF-8?B?R3LDvA==?= =?UTF-8?B?w59l?= !"), "Re: Grüße !");
        // What is not an encoded word is left as it is.
        assert_eq!(decode_words("a =? b"), "a =? b");
        assert_eq!(decode_words("=?UTF-8?X?abc?="), "=?UTF-8?X?abc?=");
    }

    #[test]
    fn what_goes_into_a_command_cannot_end_it() {
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert!(has_line_break("x\r\nc2 DELETE INBOX"));
        assert!(valid_host("imap.gmail.com"));
        assert!(!valid_host("imap.gmail.com/x"));
        assert!(!valid_host("localhost"));
        assert!(!valid_host("a b.com"));
    }

    #[test]
    fn a_password_is_kept_with_its_server() {
        let stored = bind("", "abcd efgh ijkl mnop").unwrap();
        assert_eq!(bound(&stored), Some(("imap.gmail.com".into(), "abcd efgh ijkl mnop".into())));
        let stored = bind(" IMAP.Fastmail.com ", "pw").unwrap();
        assert_eq!(bound(&stored).unwrap().0, "imap.fastmail.com");
        // Nothing that could break the entry, or name no server, is stored.
        assert!(bind("", "a\nb").is_err());
        assert!(bind("evil/x", "pw").is_err());
        assert!(bind("", "").is_err());
        // An entry without its server is not used.
        assert_eq!(bound("just-a-password"), None);
        assert_eq!(bound("not a host\npw"), None);
    }

    #[test]
    fn a_password_does_not_follow_the_server_to_another_one() {
        assert!(host_change_forgets_password(None, Some("imap.evil.example")));
        assert!(host_change_forgets_password(Some("imap.fastmail.com"), None));
        assert!(host_change_forgets_password(Some("imap.fastmail.com"), Some("imap.other.example")));
        // The same server, said another way, is the same server.
        assert!(!host_change_forgets_password(None, Some("imap.gmail.com")));
        assert!(!host_change_forgets_password(None, Some("  ")));
        assert!(!host_change_forgets_password(Some("IMAP.Fastmail.com"), Some("imap.fastmail.com ")));
    }

    #[test]
    fn a_sender_is_a_name_when_there_is_one() {
        assert_eq!(sender("=?UTF-8?Q?J=C3=BCrgen?= <j@example.org>"), "Jürgen");
        assert_eq!(sender("<only@example.org>"), "only@example.org");
        assert_eq!(clip("abc\u{7}def", 4), "abc…");
    }

    /// The whole exchange against a stand-in server speaking over a plain pipe.
    #[tokio::test]
    async fn a_look_reads_answers_with_literals() {
        let (client, mut server) = tokio::io::duplex(4096);
        let script = tokio::spawn(async move {
            let mut buf = vec![0u8; 512];
            let n = server.read(&mut buf).await.unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("c1 FETCH"));
            server
                .write_all(b"* 1 FETCH (FLAGS () BODY[HEADER.FIELDS (FROM SUBJECT)] {21}\r\nSubject: Hi\r\n\r\nxxxxxx)\r\nc1 OK done\r\n")
                .await
                .unwrap();
        });
        let mut imap = Imap { stream: BufReader::new(client), tag: 0 };
        let (ok, answers) = imap.command("FETCH 1 (FLAGS)").await.unwrap();
        script.await.unwrap();
        assert!(ok);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].literals[0].len(), 21);
        assert!(answers[0].text.ends_with(')'));
    }
}
