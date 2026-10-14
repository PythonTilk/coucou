//! Phone alerts through ntfy. (Local to this build.)
//!
//! A sideloaded iPhone app gets no push from Apple, so when the user is away
//! from the computer the alert takes another road: a short notice posted to an
//! ntfy topic of the user's own, which the ntfy app on the phone shows like any
//! other notification; tapping it opens Islet.
//!
//! Only while nobody is at the computer (no keyboard or mouse for a minute):
//! someone at the desk has the island. The notice is generic — who wants what,
//! never the command, the question or a path — because it passes through the
//! ntfy server and shows on a lock screen. The topic's address lives in the OS
//! keystore: whoever knows a topic can read it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The keystore entry: the topic's full address, `https://ntfy.sh/<topic>`.
pub const KEY: &str = "ntfy-topic-url";
/// Away means nobody touched the keyboard or the mouse for this long.
const AWAY_AFTER: Duration = Duration::from_secs(60);
/// The same kind of notice at most this often.
const SPACING: Duration = Duration::from_secs(20);
/// Tapping the notice opens the iPhone app.
const CLICK: &str = "islet://open";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Approval,
    Question,
    Finished,
    Error,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Kind> {
        match text {
            "approval" => Some(Kind::Approval),
            "question" => Some(Kind::Question),
            "finished" => Some(Kind::Finished),
            "error" => Some(Kind::Error),
            _ => None,
        }
    }

    /// The notice: its title, its text, its ntfy priority and tag.
    pub fn notice(self, agent: &str) -> (String, &'static str, &'static str, &'static str) {
        let agent = clean_agent(agent);
        match self {
            Kind::Approval => (agent, "Needs your permission", "high", "warning"),
            Kind::Question => (agent, "Has a question for you", "high", "question"),
            Kind::Finished => (agent, "Finished", "default", "white_check_mark"),
            Kind::Error => (agent, "Stopped with an error", "default", "x"),
        }
    }
}

/// An agent's name as a notice title: one short line of plain text.
fn clean_agent(agent: &str) -> String {
    let name: String = agent.chars().filter(|c| !c.is_control()).take(40).collect();
    let name = name.trim();
    if name.is_empty() { "Coucou".to_string() } else { name.to_string() }
}

/// An address a notice may be posted to: https, a host, a topic.
pub fn valid_topic_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else { return false };
    parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|h| h.contains('.'))
        && parsed.path().trim_matches('/').len() >= 1
        && parsed.query().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
}

static LAST: Mutex<Vec<(Kind, Instant)>> = Mutex::new(Vec::new());

/// Whether a notice of `kind` may go now, given when the last one went.
fn spaced(kind: Kind, now: Instant) -> bool {
    let mut last = LAST.lock().unwrap();
    if let Some(at) = last.iter().find(|(k, _)| *k == kind).map(|(_, at)| *at) {
        if now.duration_since(at) < SPACING {
            return false;
        }
    }
    last.retain(|(k, _)| *k != kind);
    last.push((kind, now));
    true
}

/// How long nobody has touched the keyboard or the mouse.
#[cfg(windows)]
pub fn idle_for() -> Duration {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return Duration::ZERO;
        }
        Duration::from_millis(GetTickCount().wrapping_sub(info.dwTime) as u64)
    }
}

#[cfg(not(windows))]
pub fn idle_for() -> Duration {
    Duration::ZERO
}

/// Posts the notice if alerts are set up, nobody is at the computer and the
/// same kind did not just go. `force` skips the last two (the Test button).
pub async fn send(kind: Kind, agent: &str, force: bool) -> Result<bool, String> {
    let Some(url) = crate::secrets::get(KEY) else { return Ok(false) };
    if !valid_topic_url(&url) {
        return Err("The ntfy address must be an https link to a topic.".into());
    }
    if !force && (idle_for() < AWAY_AFTER || !spaced(kind, Instant::now())) {
        return Ok(false);
    }
    let (title, text, priority, tag) = kind.notice(agent);
    let url = reqwest::Url::parse(url.trim()).map_err(|_| "The ntfy address is not valid.".to_string())?;
    let response = crate::net::client(&url, Duration::from_secs(10))?
        .post(url.clone())
        .header("Title", title)
        .header("Priority", priority)
        .header("Tags", tag)
        .header("Click", CLICK)
        .body(text)
        .send()
        .await
        .map_err(|_| "ntfy could not be reached.".to_string())?;
    if response.status().is_success() {
        Ok(true)
    } else {
        Err(format!("ntfy answered {}.", response.status().as_u16()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_says_who_and_what_never_the_details() {
        let (title, text, priority, _) = Kind::Approval.notice("Claude Desktop");
        assert_eq!((title.as_str(), text, priority), ("Claude Desktop", "Needs your permission", "high"));
        assert_eq!(Kind::Finished.notice("").0, "Coucou");
        assert_eq!(Kind::Error.notice("a\nb\u{7}").0, "ab");
        assert_eq!(Kind::parse("question"), Some(Kind::Question));
        assert_eq!(Kind::parse("rm -rf"), None);
    }

    #[test]
    fn only_an_https_topic_is_an_address() {
        assert!(valid_topic_url("https://ntfy.sh/my-topic-1234"));
        assert!(valid_topic_url(" https://ntfy.example.org/alerts "));
        assert!(!valid_topic_url("http://ntfy.sh/topic"));
        assert!(!valid_topic_url("https://ntfy.sh/"));
        assert!(!valid_topic_url("https://user:pw@ntfy.sh/topic"));
        assert!(!valid_topic_url("https://ntfy.sh/topic?x=1"));
        assert!(!valid_topic_url("ntfy.sh/topic"));
    }

    #[test]
    fn the_same_kind_waits_its_turn() {
        let start = Instant::now();
        assert!(spaced(Kind::Error, start));
        assert!(!spaced(Kind::Error, start + Duration::from_secs(5)));
        assert!(spaced(Kind::Question, start + Duration::from_secs(5)));
        assert!(spaced(Kind::Error, start + SPACING + Duration::from_secs(1)));
    }
}
