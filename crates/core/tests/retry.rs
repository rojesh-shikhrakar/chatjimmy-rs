use std::time::Duration;

use chatjimmy::{Client, Error};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK_BODY: &str = "hi<|stats|>{}<|/stats|>";

fn client(server: &MockServer) -> Client {
    Client::builder()
        .base_url(&server.uri())
        .max_retries(2)
        .retry_base_delay(Duration::from_millis(1))
        .build()
        .unwrap()
}

async fn calls(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

async fn fail_n_then(server: &MockServer, n: u64, code: u16, then: ResponseTemplate) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(code))
        .up_to_n_times(n)
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("POST")).respond_with(then).with_priority(2).mount(server).await;
}

#[tokio::test]
async fn retries_503_then_succeeds() {
    let server = MockServer::start().await;
    fail_n_then(&server, 2, 503, ResponseTemplate::new(200).set_body_string(OK_BODY)).await;
    assert_eq!(client(&server).ask("x").await.unwrap(), "hi");
    assert_eq!(calls(&server).await, 3);
}

#[tokio::test]
async fn retries_429() {
    let server = MockServer::start().await;
    fail_n_then(&server, 1, 429, ResponseTemplate::new(200).set_body_string(OK_BODY)).await;
    assert_eq!(client(&server).ask("x").await.unwrap(), "hi");
    assert_eq!(calls(&server).await, 2);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
    let e = client(&server).ask("x").await.unwrap_err();
    assert!(matches!(e, Error::Status { code: 503, .. }), "{e:?}");
    assert_eq!(calls(&server).await, 3);
}

#[tokio::test]
async fn no_retry_on_400() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(400)).mount(&server).await;
    assert!(client(&server).ask("x").await.is_err());
    assert_eq!(calls(&server).await, 1);
}

#[tokio::test]
async fn no_retry_on_empty_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
    let e = client(&server).ask("x").await.unwrap_err();
    assert!(matches!(e, Error::EmptyResponse), "{e:?}");
    assert_eq!(calls(&server).await, 1);
}
