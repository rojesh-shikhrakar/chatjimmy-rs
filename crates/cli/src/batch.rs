use std::io::Write;

use chatjimmy::{ChatRequest, Client, Message, Stats};
use futures::StreamExt;
use serde_json::{json, Map, Value};

/// Shared options; per-line JSON fields override them.
pub struct Defaults {
    pub model: String,
    pub system_prompt: String,
    pub top_k: u32,
}

/// A line is a request if it is a JSON object with a `prompt` key; otherwise it is a plain prompt.
/// Returns the request and its optional `id` (a string or number, echoed unchanged). `Err` carries
/// the `id` when one could be read, plus the message for the error record.
pub fn parse_line(
    line: &str,
    d: &Defaults,
) -> Result<(ChatRequest, Option<Value>), (Option<Value>, String)> {
    let line = line.trim_end_matches('\r');
    let obj = match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(o)) if o.contains_key("prompt") => o,
        _ => return Ok((request(line, &d.model, &d.system_prompt, d.top_k), None)),
    };
    let id = match obj.get("id") {
        None => None,
        Some(v @ (Value::String(_) | Value::Number(_))) => Some(v.clone()),
        Some(_) => return Err((None, "`id` must be a string or number".into())),
    };
    match request_from(&obj, d) {
        Ok(req) => Ok((req, id)),
        Err(msg) => Err((id, msg)),
    }
}

fn request_from(obj: &Map<String, Value>, d: &Defaults) -> Result<ChatRequest, String> {
    let str_field = |key: &str| -> Result<Option<String>, String> {
        match obj.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    };
    let prompt = str_field("prompt")?.expect("key checked by caller");
    let top_k = match obj.get("top_k") {
        None => d.top_k,
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("`top_k` must be a non-negative integer")?,
    };
    let model = str_field("model")?.unwrap_or_else(|| d.model.clone());
    let system = str_field("system_prompt")?.unwrap_or_else(|| d.system_prompt.clone());
    Ok(request(&prompt, &model, &system, top_k))
}

fn request(prompt: &str, model: &str, system: &str, top_k: u32) -> ChatRequest {
    ChatRequest::new([Message::user(prompt)]).model(model).system_prompt(system).top_k(top_k)
}

/// One JSONL output line.
pub enum Record {
    Ok { index: usize, id: Option<Value>, text: String, stats: Option<Stats> },
    Err { index: usize, id: Option<Value>, error: String },
}

impl Record {
    pub fn to_line(&self) -> String {
        let mut m = Map::new();
        let (index, id) = match self {
            Record::Ok { index, id, .. } | Record::Err { index, id, .. } => (index, id),
        };
        m.insert("index".into(), json!(index));
        if let Some(id) = id {
            m.insert("id".into(), id.clone());
        }
        match self {
            Record::Ok { text, stats, .. } => {
                m.insert("ok".into(), json!(true));
                m.insert("text".into(), json!(text));
                m.insert("stats".into(), json!(stats));
            }
            Record::Err { error, .. } => {
                m.insert("ok".into(), json!(false));
                m.insert("error".into(), json!(error));
            }
        }
        Value::Object(m).to_string()
    }
}

fn write_record(out: &mut impl Write, rec: &Record) -> std::io::Result<()> {
    writeln!(out, "{}", rec.to_line())?;
    out.flush()
}

/// Runs every non-blank line as a request and writes one record per line as results arrive.
/// Returns whether every request succeeded. Write errors (e.g. a closed pipe) are returned.
pub async fn run_batch(
    client: &Client,
    lines: impl IntoIterator<Item = String>,
    d: &Defaults,
    concurrency: usize,
    out: &mut impl Write,
) -> std::io::Result<bool> {
    let mut all_ok = true;
    let mut reqs = Vec::new();
    let mut meta: Vec<(usize, Option<Value>)> = Vec::new(); // submission index -> (line index, id)
    let mut index = 0;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let i = index;
        index += 1;
        match parse_line(&line, d) {
            Ok((req, id)) => {
                reqs.push(req);
                meta.push((i, id));
            }
            Err((id, error)) => {
                all_ok = false;
                write_record(out, &Record::Err { index: i, id, error })?;
            }
        }
    }
    let mut results = std::pin::pin!(client.chat_many_stream(reqs, concurrency));
    while let Some((k, res)) = results.next().await {
        let (index, id) = meta[k].clone();
        let rec = match res {
            Ok(r) => Record::Ok { index, id, text: r.text, stats: r.stats },
            Err(e) => {
                all_ok = false;
                Record::Err { index, id, error: e.to_string() }
            }
        };
        write_record(out, &rec)?;
    }
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
        assert_eq!(id, Some(json!("x")));
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
        assert_eq!(id, Some(json!("i")));
    }

    #[test]
    fn escaped_newline_preserved() {
        let (r, _) = parse_line(r#"{"prompt":"a\nb"}"#, &d()).unwrap();
        assert_eq!(prompt(&r), "a\nb");
    }

    #[test]
    fn record_ok_shape() {
        let r = Record::Ok { index: 3, id: Some(json!("x")), text: "t".into(), stats: None };
        assert_eq!(r.to_line(), r#"{"index":3,"id":"x","ok":true,"text":"t","stats":null}"#);
    }

    #[test]
    fn record_err_shape_omits_missing_id() {
        let r = Record::Err { index: 0, id: None, error: "boom".into() };
        assert_eq!(r.to_line(), r#"{"index":0,"ok":false,"error":"boom"}"#);
    }

    #[test]
    fn stats_serializes() {
        let v = serde_json::to_value(chatjimmy::Stats::default()).unwrap();
        assert!(v.get("decode_tokens").is_some());
    }

    async fn run(client: &chatjimmy::Client, lines: &[&str]) -> (bool, Vec<Value>) {
        let mut out = Vec::new();
        let ok = run_batch(client, lines.iter().map(|s| s.to_string()), &d(), 4, &mut out)
            .await
            .unwrap();
        let recs = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        (ok, recs)
    }

    #[tokio::test]
    async fn blank_lines_skipped_and_indices_counted() {
        let client = chatjimmy::Client::new().unwrap(); // no network: both lines fail to parse
        let (ok, recs) = run(&client, &["", r#"{"prompt":5}"#, "   ", r#"{"prompt":6}"#]).await;
        assert!(!ok);
        let idx: Vec<_> = recs.iter().map(|r| r["index"].as_u64().unwrap()).collect();
        assert_eq!(idx, [0, 1]);
        assert!(recs.iter().all(|r| r["ok"] == false));
    }

    #[tokio::test]
    async fn empty_input_writes_nothing() {
        let client = chatjimmy::Client::new().unwrap();
        let (ok, recs) = run(&client, &[]).await;
        assert!(ok && recs.is_empty());
    }

    struct Closed;
    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn broken_pipe_stops_quietly() {
        let client = chatjimmy::Client::new().unwrap();
        let e = run_batch(&client, [r#"{"prompt":5}"#.to_string()], &d(), 1, &mut Closed)
            .await
            .unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[tokio::test]
    async fn ids_and_indices_map_back() {
        let server = MockServer::start().await;
        for (p, reply) in [("a", "RA"), ("b", "RB")] {
            Mock::given(method("POST"))
                .and(body_string_contains(format!(r#""content":"{p}""#)))
                .respond_with(ResponseTemplate::new(200).set_body_string(format!("{reply}<|stats|>{{}}<|/stats|>")))
                .mount(&server)
                .await;
        }
        let client = chatjimmy::Client::builder().base_url(&server.uri()).build().unwrap();
        let (ok, recs) = run(&client, &["a", r#"{"prompt":"b","id":"B"}"#]).await;
        assert!(ok);
        let by = |i: u64| recs.iter().find(|r| r["index"] == i).unwrap();
        assert_eq!((by(0)["text"].as_str(), by(0).get("id")), (Some("RA"), None));
        assert_eq!((by(1)["text"].as_str(), by(1)["id"].as_str()), (Some("RB"), Some("B")));
    }

    #[test]
    fn error_keeps_the_id() {
        let (id, msg) = parse_line(r#"{"prompt":5,"id":"bad0"}"#, &d()).unwrap_err();
        assert_eq!(id, Some(json!("bad0")));
        assert!(msg.contains("prompt"));
    }

    #[test]
    fn numeric_id_is_echoed_unchanged() {
        let (_, id) = parse_line(r#"{"prompt":"p","id":7}"#, &d()).unwrap();
        assert_eq!(id, Some(json!(7)));
        let r = Record::Err { index: 0, id, error: "e".into() };
        assert_eq!(r.to_line(), r#"{"index":0,"id":7,"ok":false,"error":"e"}"#);
    }

    #[test]
    fn non_scalar_id_is_error() {
        assert!(parse_line(r#"{"prompt":"p","id":[1]}"#, &d()).is_err());
    }
}
