// Chat through any OpenAI-compatible endpoint: OpenRouter, Gemini's
// compatibility endpoint, a local Ollama or LM Studio — anything that answers
// `POST <base>/chat/completions`. The base URL and model ID come from Settings;
// the key, when the endpoint wants one, from the Credential Manager.
//
// Plain chat only: no web search here, and a dropped file is sent as text or
// as an image, which is what this API can carry.

use serde_json::{json, Value};

use crate::claude::{base64_for, Chat, ChatContext, ChatReply, SYSTEM_PROMPT};
use crate::secrets;

/// `Settings::backend` for this provider.
pub const BACKEND: &str = "openai";
/// Optional: a local server needs no key.
const KEY: &str = "custom-api-key";
/// Text and code files are inlined; anything larger is skipped, as elsewhere.
const MAX_INLINE_TEXT: u64 = 200_000;

pub async fn send(
    chat: &Chat,
    base_url: &str,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let url = endpoint(base_url)?;
    let model = model.trim();
    if model.is_empty() {
        return Err("No model set. Open settings and enter a model ID.".into());
    }

    // File / window context rides along with the first message only.
    let context = if chat.other_is_empty() { context.as_ref() } else { None };
    chat.other_push(json!({ "role": "user", "content": user_content(&query, context) }));

    let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    messages.extend(chat.other_snapshot());
    let body = json!({ "model": model, "messages": messages });

    let text = match call(&url, &body).await.and_then(|v| reply_text(&v)) {
        Ok(text) => text,
        Err(err) => {
            chat.other_pop(); // keep the history consistent with what the model saw
            return Err(err);
        }
    };
    chat.other_push(json!({ "role": "assistant", "content": text }));
    Ok(ChatReply { text })
}

/// `<base>/chat/completions`, whether or not the base already ends with it.
fn endpoint(base: &str) -> Result<String, String> {
    let base = base.trim().trim_end_matches('/');
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err("No base URL set. Open settings and enter one, starting with https://.".into());
    }
    if base.ends_with("/chat/completions") {
        Ok(base.to_string())
    } else {
        Ok(format!("{base}/chat/completions"))
    }
}

/// A plain string when there is only text — every server takes that — and
/// content parts when an image comes along.
fn user_content(query: &str, context: Option<&ChatContext>) -> Value {
    match context {
        Some(ChatContext::File { name, path }) => match file_part(path) {
            Some(FilePart::Text(text)) => {
                json!(format!("File: {name}\nFile contents:\n{text}\n\n{query}"))
            }
            Some(FilePart::Image(data_url)) => json!([
                { "type": "text", "text": format!("File: {name}\n\n{query}") },
                { "type": "image_url", "image_url": { "url": data_url } },
            ]),
            None => json!(format!(
                "(The file {name} could not be attached: this provider only takes text and images.)\n\n{query}"
            )),
        },
        Some(ChatContext::Window { app_name, title, url }) => {
            let mut text = format!("Context — App: {app_name}, Window: {title}");
            if let Some(url) = url {
                text.push_str(&format!(", URL: {url}"));
            }
            json!(format!("{text}\n\n{query}"))
        }
        None => json!(query),
    }
}

enum FilePart {
    Text(String),
    /// A `data:` URL.
    Image(String),
}

fn file_part(path: &str) -> Option<FilePart> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let image = match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "pdf" => return None,
        _ => None,
    };
    if let Some(media) = image {
        let bytes = std::fs::read(path).ok()?;
        return Some(FilePart::Image(format!("data:{media};base64,{}", base64_for(&bytes))));
    }
    if std::fs::metadata(path).ok()?.len() > MAX_INLINE_TEXT {
        return None;
    }
    std::fs::read_to_string(path).ok().map(FilePart::Text)
}

async fn call(url: &str, body: &Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client.post(url).header("content-type", "application/json").json(body);
    if let Some(key) = secrets::get(KEY) {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(|e| format!("Network error: {e}"))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{status}: {}", error_detail(&text)));
    }
    serde_json::from_str(&text).map_err(|e| format!("Bad response: {e}"))
}

/// The provider's own message when it sends one; the start of the body otherwise.
fn error_detail(text: &str) -> String {
    let parsed = serde_json::from_str::<Value>(text).ok();
    // Usually {"error":{"message":…}}; Gemini wraps that in an array.
    let error = parsed.as_ref().map(|v| v.get(0).unwrap_or(v)).and_then(|v| v.get("error"));
    error
        .and_then(|e| e.get("message").or(Some(e)))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| text.chars().take(200).collect())
}

/// `choices[0].message.content`, as a string or as text parts.
fn reply_text(response: &Value) -> Result<String, String> {
    let content = response
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .ok_or("Unexpected response from the provider.")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("No response text.".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_is_built_from_any_form_of_base_url() {
        let want = "https://openrouter.ai/api/v1/chat/completions";
        assert_eq!(endpoint("https://openrouter.ai/api/v1").unwrap(), want);
        assert_eq!(endpoint(" https://openrouter.ai/api/v1/ ").unwrap(), want);
        assert_eq!(endpoint(want).unwrap(), want);
        assert_eq!(
            endpoint("http://localhost:11434/v1").unwrap(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn a_missing_or_odd_base_url_is_refused() {
        assert!(endpoint("").is_err());
        assert!(endpoint("openrouter.ai/api/v1").is_err());
        assert!(endpoint("file:///etc/passwd").is_err());
    }

    #[test]
    fn plain_questions_stay_plain_strings() {
        assert_eq!(user_content("Hi", None), json!("Hi"));
    }

    #[test]
    fn a_reply_is_read_as_a_string_or_as_parts() {
        let plain = json!({ "choices": [{ "message": { "content": " hello \n" } }] });
        assert_eq!(reply_text(&plain).unwrap(), "hello");
        let parts = json!({ "choices": [{ "message": { "content": [
            { "type": "text", "text": "a" }, { "type": "text", "text": "b" }
        ] } }] });
        assert_eq!(reply_text(&parts).unwrap(), "a\nb");
        assert!(reply_text(&json!({ "choices": [] })).is_err());
        assert!(reply_text(&json!({ "choices": [{ "message": { "content": "" } }] })).is_err());
    }

    #[test]
    fn the_providers_own_error_message_is_shown() {
        assert_eq!(error_detail(r#"{"error":{"message":"No such model"}}"#), "No such model");
        assert_eq!(error_detail(r#"[{"error":{"message":"Bad key"}}]"#), "Bad key");
        assert_eq!(error_detail(r#"{"error":"Nope"}"#), "Nope");
        assert_eq!(error_detail("gateway timeout"), "gateway timeout");
    }
}
