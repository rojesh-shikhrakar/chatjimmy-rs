use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;

use crate::parser::Parser;
use crate::{ChatRequest, ChatResponse, Error, HealthStatus, Message, Model, Result};

pub const BASE_URL: &str = "https://chatjimmy.ai";

struct Inner {
    http: reqwest::Client,
    base_url: String,
    #[allow(dead_code)] // used by retries (Task 4)
    max_retries: u32,
    #[allow(dead_code)]
    retry_base_delay: Duration,
}

/// Cheap to clone; share one across tasks.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

pub struct ClientBuilder {
    base_url: String,
    connect_timeout: Duration,
    read_timeout: Duration,
    timeout: Option<Duration>,
    max_retries: u32,
    retry_base_delay: Duration,
}

impl ClientBuilder {
    pub fn base_url(mut self, url: &str) -> Self {
        self.base_url = url.trim_end_matches('/').into();
        self
    }
    pub fn connect_timeout(mut self, d: Duration) -> Self {
        self.connect_timeout = d;
        self
    }
    /// Applies per chunk read, not to the whole request.
    pub fn read_timeout(mut self, d: Duration) -> Self {
        self.read_timeout = d;
        self
    }
    /// Total request timeout. Unset by default.
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }
    /// First backoff delay (doubles per attempt, plus jitter). Default 200 ms.
    pub fn retry_base_delay(mut self, d: Duration) -> Self {
        self.retry_base_delay = d;
        self
    }
    pub fn build(self) -> Result<Client> {
        let mut http = reqwest::Client::builder()
            .user_agent(concat!("chatjimmy-rust/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(self.connect_timeout)
            .read_timeout(self.read_timeout);
        if let Some(t) = self.timeout {
            http = http.timeout(t);
        }
        Ok(Client {
            inner: Arc::new(Inner {
                http: http.build()?,
                base_url: self.base_url,
                max_retries: self.max_retries,
                retry_base_delay: self.retry_base_delay,
            }),
        })
    }
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            base_url: BASE_URL.into(),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
            timeout: None,
            max_retries: 3,
            retry_base_delay: Duration::from_millis(200),
        }
    }

    pub fn new() -> Result<Self> {
        Self::builder().build()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.inner.base_url, path)
    }

    pub(crate) async fn post_chat(&self, req: &ChatRequest) -> Result<reqwest::Response> {
        let resp = self
            .inner
            .http
            .post(self.url("/api/chat"))
            .json(&req.body())
            .send()
            .await?;
        check_status(resp).await
    }

    pub async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let resp = self.post_chat(&req).await?;
        let mut body = resp.bytes_stream();
        let mut parser = Parser::new();
        let mut text = String::new();
        while let Some(chunk) = body.next().await {
            text += &parser.push(&chunk?)?;
        }
        let (rest, stats) = parser.finish()?;
        text += &rest;
        Ok(ChatResponse { text, stats })
    }

    pub async fn ask(&self, prompt: &str) -> Result<String> {
        Ok(self.chat(ChatRequest::new([Message::user(prompt)])).await?.text)
    }

    pub async fn health(&self) -> Result<HealthStatus> {
        let resp = self.inner.http.get(self.url("/api/health")).send().await?;
        Ok(check_status(resp).await?.json().await?)
    }

    pub async fn models(&self) -> Result<Vec<Model>> {
        #[derive(serde::Deserialize)]
        struct Models {
            #[serde(default)]
            data: Vec<Model>,
        }
        let resp = self.inner.http.get(self.url("/api/models")).send().await?;
        Ok(check_status(resp).await?.json::<Models>().await?.data)
    }
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Status { code: status.as_u16(), body })
}
