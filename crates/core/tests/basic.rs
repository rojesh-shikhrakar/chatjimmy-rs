use chatjimmy::{ChatRequest, Client, Error, Message};
use wiremock::matchers::{body_json, header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK_BODY: &str = "hi<|stats|>{\"decode_tokens\":2}<|/stats|>";

async fn client(server: &MockServer) -> Client {
    Client::builder().base_url(&server.uri()).build().unwrap()
}

async fn mock_chat(server: &MockServer, body: &str) {
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn chat_returns_text_and_stats() {
    let server = MockServer::start().await;
    let req = ChatRequest::new([Message::user("hello")]);
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_json(req.body()))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK_BODY))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server).await.chat(req).await.unwrap();
    assert_eq!(r.text, "hi");
    assert_eq!(r.stats.unwrap().decode_tokens, 2);
}

#[tokio::test]
async fn chat_sends_content_type_and_user_agent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header_regex("content-type", "^application/json"))
        .and(header_regex("user-agent", "^chatjimmy-rust/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK_BODY))
        .expect(1)
        .mount(&server)
        .await;
    client(&server).await.ask("x").await.unwrap();
}

#[tokio::test]
async fn get_requests_have_no_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;
    client(&server).await.health().await.unwrap();
    let reqs = server.received_requests().await.unwrap();
    assert!(reqs[0].headers.get("content-type").is_none());
}

#[tokio::test]
async fn ask_returns_text() {
    let server = MockServer::start().await;
    mock_chat(&server, OK_BODY).await;
    assert_eq!(client(&server).await.ask("x").await.unwrap(), "hi");
}

#[tokio::test]
async fn health_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"status":"ok","backend":"healthy","backendStatus":200,"timestamp":"t"}"#,
        ))
        .mount(&server)
        .await;
    let h = client(&server).await.health().await.unwrap();
    assert!(h.healthy());
    assert_eq!(h.timestamp, "t");
}

#[tokio::test]
async fn models_parses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"data":[{"id":"llama3.1-8B","object":"model","created":1,"owned_by":"Taalas Inc."}]}"#,
        ))
        .mount(&server)
        .await;
    let m = client(&server).await.models().await.unwrap();
    assert_eq!(m[0].id, "llama3.1-8B");
    assert_eq!(m[0].owned_by, "Taalas Inc.");
}

#[tokio::test]
async fn status_error_carries_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad"))
        .mount(&server)
        .await;
    let e = client(&server).await.ask("x").await.unwrap_err();
    assert!(matches!(e, Error::Status { code: 400, ref body } if body == "bad"), "{e:?}");
}

#[tokio::test]
async fn empty_200_is_empty_response() {
    let server = MockServer::start().await;
    mock_chat(&server, "").await;
    let e = client(&server).await.ask("x").await.unwrap_err();
    assert!(matches!(e, Error::EmptyResponse), "{e:?}");
}

#[tokio::test]
async fn health_non_json_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .mount(&server)
        .await;
    assert!(client(&server).await.health().await.is_err());
}

#[tokio::test]
async fn models_wrong_shape_is_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"error":"down"}"#))
        .mount(&server)
        .await;
    assert!(client(&server).await.models().await.is_err());
}
