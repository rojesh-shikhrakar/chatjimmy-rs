#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned status {code}: {body}")]
    Status { code: u16, body: String },
    /// 200 with no text and no stats (typically input over the ~6k token limit).
    #[error("empty response (input may exceed the ~6k token limit)")]
    EmptyResponse,
    #[error("malformed stats block: {0}")]
    BadStats(serde_json::Error),
    #[error("invalid utf-8 in response")]
    Decode,
}

pub type Result<T> = std::result::Result<T, Error>;
