use futures::stream::{self, Stream, StreamExt};

use crate::{ChatRequest, ChatResponse, Client, Result};

impl Client {
    /// Run requests with at most `concurrency` in flight (clamped to >= 1).
    /// Yields `(input index, result)` in completion order.
    pub fn chat_many_stream(
        &self,
        reqs: Vec<ChatRequest>,
        concurrency: usize,
    ) -> impl Stream<Item = (usize, Result<ChatResponse>)> {
        let client = self.clone();
        stream::iter(reqs)
            .enumerate()
            .map(move |(i, req)| {
                let client = client.clone();
                async move { (i, client.chat(req).await) }
            })
            .buffer_unordered(concurrency.max(1))
    }

    /// Like `chat_many_stream`, but returns per-item results in input order.
    pub async fn chat_many(
        &self,
        reqs: Vec<ChatRequest>,
        concurrency: usize,
    ) -> Vec<Result<ChatResponse>> {
        let mut out: Vec<Option<_>> = (0..reqs.len()).map(|_| None).collect();
        let mut results = std::pin::pin!(self.chat_many_stream(reqs, concurrency));
        while let Some((i, r)) = results.next().await {
            out[i] = Some(r);
        }
        out.into_iter().map(|r| r.expect("every index yields once")).collect()
    }
}
