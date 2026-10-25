// The download rules of upstream's update.rs, which the WhatsApp installer
// (whatsapp/install.rs, whatsapp/gateway.rs) shares. (Local to this build:
// upstream's updater installs Louis' public builds, and this build updates from
// its private repository instead, in updates.rs, so only these helpers are kept.)

use reqwest::Url;

/// Only GitHub serves the download (its assets redirect to githubusercontent.com).
pub fn allowed_host(host: Option<&str>) -> bool {
    let Some(h) = host.map(str::to_ascii_lowercase) else { return false };
    h == "github.com" || h == "api.github.com" || h.ends_with(".githubusercontent.com")
}

pub fn allowed_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| u.scheme() == "https" && allowed_host(u.host_str()))
}

/// "sha256:E823…" → the 64 lowercase hex digits; anything else → None.
pub fn parse_digest(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_over_https() {
        for host in ["github.com", "api.github.com", "objects.githubusercontent.com", "GitHub.com"] {
            assert!(allowed_host(Some(host)), "{host}");
        }
        for host in ["github.com.evil.com", "evilgithub.com", "githubusercontent.com.x", ""] {
            assert!(!allowed_host(Some(host)), "{host}");
        }
        assert!(!allowed_host(None));
        assert!(allowed_url("https://github.com/tulir/gowa/releases/download/x/y.zip"));
        assert!(!allowed_url("http://github.com/tulir/gowa"));
        assert!(!allowed_url("file:///C:/Windows/notepad.exe"));
        assert!(!allowed_url("not a url"));
    }

    #[test]
    fn a_digest_is_sha256_and_64_hex_digits() {
        let hex = "e823c4f8a0b1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7";
        assert_eq!(parse_digest(&format!("sha256:{hex}")).as_deref(), Some(hex));
        assert_eq!(parse_digest(&format!("sha256:{}", hex.to_uppercase())).as_deref(), Some(hex));
        assert_eq!(parse_digest(hex), None);
        assert_eq!(parse_digest(&format!("sha512:{hex}")), None);
        assert_eq!(parse_digest(&format!("sha256:{}", &hex[1..])), None);
        assert_eq!(parse_digest(""), None);
    }
}
