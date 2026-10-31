use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_MODEL: &str = "llama3.1-8B";
pub const DEFAULT_TOP_K: u32 = 8;

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Attachment {
    pub name: String,
    pub size: u64,
    pub content: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub created_at: f64,
    pub done: bool,
    pub done_reason: String,
    pub total_duration: f64,
    pub ttft: f64,
    pub prefill_tokens: u64,
    pub prefill_rate: f64,
    pub decode_tokens: u64,
    pub decode_rate: f64,
    pub total_tokens: u64,
    pub total_time: f64,
    pub roundtrip_time: f64,
    pub topk: u64,
    pub status: u64,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub model: String,
    pub system_prompt: String,
    pub top_k: u32,
    pub attachment: Option<Attachment>,
}

impl ChatRequest {
    pub fn new(messages: impl IntoIterator<Item = Message>) -> Self {
        Self {
            messages: messages.into_iter().collect(),
            model: DEFAULT_MODEL.into(),
            system_prompt: String::new(),
            top_k: DEFAULT_TOP_K,
            attachment: None,
        }
    }
    pub fn model(mut self, model: &str) -> Self {
        self.model = model.into();
        self
    }
    pub fn system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = prompt.into();
        self
    }
    pub fn top_k(mut self, top_k: u32) -> Self {
        self.top_k = top_k;
        self
    }
    pub fn attachment(mut self, attachment: Attachment) -> Self {
        self.attachment = Some(attachment);
        self
    }
    pub fn body(&self) -> Value {
        json!({
            "messages": self.messages,
            "chatOptions": {
                "selectedModel": self.model,
                "systemPrompt": self.system_prompt,
                "topK": self.top_k,
            },
            "attachment": self.attachment,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub stats: Option<Stats>,
}

#[derive(Debug, Clone)]
pub enum Chunk {
    Text(String),
    Done(Stats),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HealthStatus {
    pub status: String,
    pub nextjs: String,
    pub backend: String,
    pub backend_status: u64,
    pub backend_details: Value,
    pub timestamp: String,
}

impl HealthStatus {
    pub fn healthy(&self) -> bool {
        self.status == "ok" && self.backend == "healthy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_body_defaults() {
        let body = ChatRequest::new([Message::user("hi")]).body();
        assert_eq!(
            body,
            json!({
                "messages": [{"role": "user", "content": "hi"}],
                "chatOptions": {"selectedModel": "llama3.1-8B", "systemPrompt": "", "topK": 8},
                "attachment": null
            })
        );
    }

    #[test]
    fn stats_ignores_unknown_and_defaults_missing() {
        let s: Stats = serde_json::from_str(r#"{"decode_tokens":5,"bogus":1}"#).unwrap();
        assert_eq!(s.decode_tokens, 5);
        assert_eq!(s.prefill_tokens, 0);
        assert_eq!(s.done_reason, "");
    }

    #[test]
    fn health_healthy() {
        let h: HealthStatus =
            serde_json::from_str(r#"{"status":"ok","backend":"healthy","backendStatus":200}"#).unwrap();
        assert!(h.healthy());
        assert_eq!(h.backend_status, 200);
    }
}
