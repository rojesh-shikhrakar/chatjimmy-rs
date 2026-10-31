use chatjimmy::{ChatRequest, Message};
use serde_json::Value;

/// Shared options; per-line JSON fields override them.
pub struct Defaults {
    pub model: String,
    pub system_prompt: String,
    pub top_k: u32,
}

/// A line is a request if it is a JSON object with a `prompt` key; otherwise it is a plain prompt.
/// Returns the request and its optional `id`; `Err` is the message for the error record.
pub fn parse_line(line: &str, d: &Defaults) -> Result<(ChatRequest, Option<String>), String> {
    let line = line.trim_end_matches('\r');
    let obj = match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(o)) if o.contains_key("prompt") => o,
        _ => return Ok((request(line, &d.model, &d.system_prompt, d.top_k), None)),
    };
    let str_field = |key: &str| -> Result<Option<String>, String> {
        match obj.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    };
    let prompt = str_field("prompt")?.expect("key checked above");
    let top_k = match obj.get("top_k") {
        None => d.top_k,
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("`top_k` must be a non-negative integer")?,
    };
    let model = str_field("model")?.unwrap_or_else(|| d.model.clone());
    let system = str_field("system_prompt")?.unwrap_or_else(|| d.system_prompt.clone());
    Ok((request(&prompt, &model, &system, top_k), str_field("id")?))
}

fn request(prompt: &str, model: &str, system: &str, top_k: u32) -> ChatRequest {
    ChatRequest::new([Message::user(prompt)]).model(model).system_prompt(system).top_k(top_k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> Defaults {
        Defaults { model: "dm".into(), system_prompt: "ds".into(), top_k: 8 }
    }

    fn prompt(r: &chatjimmy::ChatRequest) -> &str {
        &r.messages[0].content
    }

    #[test]
    fn plain_line_uses_defaults() {
        let (r, id) = parse_line("hello", &d()).unwrap();
        assert_eq!(prompt(&r), "hello");
        assert_eq!((r.model.as_str(), r.system_prompt.as_str(), r.top_k), ("dm", "ds", 8));
        assert!(id.is_none());
    }

    #[test]
    fn json_line_all_fields() {
        let line = r#"{"prompt":"p","id":"x","system_prompt":"s","model":"m","top_k":3}"#;
        let (r, id) = parse_line(line, &d()).unwrap();
        assert_eq!(prompt(&r), "p");
        assert_eq!((r.model.as_str(), r.system_prompt.as_str(), r.top_k), ("m", "s", 3));
        assert_eq!(id.as_deref(), Some("x"));
    }

    #[test]
    fn object_without_prompt_is_plain() {
        let (r, _) = parse_line(r#"{"a":1}"#, &d()).unwrap();
        assert_eq!(prompt(&r), r#"{"a":1}"#);
    }

    #[test]
    fn non_json_is_plain() {
        let (r, _) = parse_line("{oops", &d()).unwrap();
        assert_eq!(prompt(&r), "{oops");
    }

    #[test]
    fn non_string_prompt_is_error() {
        assert!(parse_line(r#"{"prompt":5}"#, &d()).is_err());
    }

    #[test]
    fn bad_top_k_is_error() {
        assert!(parse_line(r#"{"prompt":"p","top_k":-1}"#, &d()).is_err());
    }

    #[test]
    fn overrides_beat_defaults() {
        let (r, _) = parse_line(r#"{"prompt":"p","model":"m"}"#, &d()).unwrap();
        assert_eq!((r.model.as_str(), r.system_prompt.as_str()), ("m", "ds"));
    }

    #[test]
    fn crlf_is_trimmed() {
        assert_eq!(prompt(&parse_line("hi\r", &d()).unwrap().0), "hi");
        let (_, id) = parse_line("{\"prompt\":\"p\",\"id\":\"i\"}\r", &d()).unwrap();
        assert_eq!(id.as_deref(), Some("i"));
    }

    #[test]
    fn escaped_newline_preserved() {
        let (r, _) = parse_line(r#"{"prompt":"a\nb"}"#, &d()).unwrap();
        assert_eq!(prompt(&r), "a\nb");
    }
}
