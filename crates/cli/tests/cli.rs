use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK_BODY: &str = "hi there<|stats|>{\"decode_tokens\":2}<|/stats|>";

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(r#""content":"two""#))
        .respond_with(ResponseTemplate::new(400).set_body_string("nope"))
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK_BODY))
        .with_priority(2)
        .mount(&s)
        .await;
    s
}

fn cli(server: &MockServer, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_chatjimmy"))
        .args(args)
        .args(["--base-url", &server.uri()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_for_output()
}

trait Wait {
    fn wait_for_output(self) -> Output;
}
impl Wait for std::process::Child {
    fn wait_for_output(self) -> Output {
        self.wait_with_output().unwrap()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}
fn err(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}
fn records(o: &Output) -> Vec<Value> {
    out(o).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_prints_reply() {
    let s = server().await;
    let o = cli(&s, &["ask", "hello"], "");
    assert_eq!((out(&o).as_str(), o.status.code()), ("hi there\n", Some(0)), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_stream_prints_same_text() {
    let s = server().await;
    let o = cli(&s, &["ask", "hello", "--stream"], "");
    assert_eq!((out(&o).as_str(), o.status.code()), ("hi there\n", Some(0)), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_reads_stdin_when_dash() {
    let s = server().await;
    let o = cli(&s, &["ask", "-"], "from stdin\n");
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let reqs = s.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"], "from stdin");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_empty_prompt_is_error() {
    let s = server().await;
    let o = cli(&s, &["ask"], "");
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("chatjimmy: error:"), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_stats_goes_to_stderr_as_json() {
    let s = server().await;
    let o = cli(&s, &["ask", "hello", "--stats"], "");
    assert_eq!(out(&o), "hi there\n");
    assert!(err(&o).contains(r#""decode_tokens":2"#), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mixed_records_and_exit_2() {
    let s = server().await;
    let input = "ok one\n{\"prompt\":\"two\",\"id\":\"B\"}\n{\"prompt\":5}\n";
    let o = cli(&s, &["batch"], input);
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    let recs = records(&o);
    assert_eq!(recs.len(), 3);
    let by = |i: u64| recs.iter().find(|r| r["index"] == i).unwrap();
    assert_eq!((by(0)["ok"].clone(), by(0)["text"].clone()), (true.into(), "hi there".into()));
    assert_eq!((by(1)["ok"].clone(), by(1)["id"].clone()), (false.into(), "B".into()));
    assert_eq!(by(2)["ok"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_all_ok_exit_0() {
    let s = server().await;
    let o = cli(&s, &["batch"], "a\nb\n");
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(records(&o).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_empty_input_exit_0_no_output() {
    let s = server().await;
    let o = cli(&s, &["batch"], "");
    assert_eq!((o.status.code(), out(&o).as_str()), (Some(0), ""));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_concurrency_zero_completes() {
    let s = server().await;
    let o = cli(&s, &["batch", "-c", "0"], "a\nb\n");
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(records(&o).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_missing_file_exit_1() {
    let s = server().await;
    let o = cli(&s, &["batch", "/nonexistent/prompts.txt"], "");
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("chatjimmy: error:") && err(&o).contains("/nonexistent/prompts.txt"), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_invalid_utf8_file_exit_1() {
    let s = server().await;
    let path = std::env::temp_dir().join(format!("chatjimmy-bad-utf8-{}.txt", std::process::id()));
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    let o = cli(&s, &["batch", path.to_str().unwrap()], "");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("chatjimmy: error:") && !err(&o).contains("panicked"), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_reads_file_argument() {
    let s = server().await;
    let path = std::env::temp_dir().join(format!("chatjimmy-prompts-{}.txt", std::process::id()));
    std::fs::write(&path, "a\nb\n").unwrap();
    let o = cli(&s, &["batch", path.to_str().unwrap()], "");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(records(&o).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn closed_stdout_exits_quietly() {
    use std::io::{BufRead, BufReader};
    let s = MockServer::start().await;
    let big = format!("{}<|stats|>{{}}<|/stats|>", "x".repeat(200_000)); // exceeds the pipe buffer
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(big))
        .mount(&s)
        .await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_chatjimmy"))
        .args(["batch", "-c", "2", "--base-url", &s.uri()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all("p\n".repeat(20).as_bytes()).unwrap();
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut first).unwrap(); // then drop: pipe closes
    let o = child.wait_with_output().unwrap();
    assert!(!first.is_empty());
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(err(&o), "");
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_error_exits_1_and_help_exits_0() {
    let s = server().await;
    assert_eq!(cli(&s, &["bogus"], "").status.code(), Some(1));
    assert_eq!(cli(&s, &["batch", "-c", "-1"], "").status.code(), Some(1));
    assert_eq!(cli(&s, &["--help"], "").status.code(), Some(0));
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn ask_write_error_is_fatal() {
    let s = server().await;
    let o = Command::new(env!("CARGO_BIN_EXE_chatjimmy"))
        .args(["ask", "hi", "--base-url", &s.uri()])
        .stdout(std::fs::File::create("/dev/full").unwrap())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(err(&o).starts_with("chatjimmy: error:"), "{}", err(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_ignores_utf8_bom() {
    let s = server().await;
    let path = std::env::temp_dir().join(format!("chatjimmy-bom-{}.txt", std::process::id()));
    std::fs::write(&path, b"\xEF\xBB\xBF{\"prompt\":\"p\",\"id\":\"X\"}\r\n").unwrap();
    let o = cli(&s, &["batch", path.to_str().unwrap()], "");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(records(&o)[0]["id"], "X");
}
