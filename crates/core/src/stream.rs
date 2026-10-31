use std::collections::VecDeque;
use std::pin::Pin;

use bytes::Bytes;
use futures::stream::{self, Stream, StreamExt};

use crate::parser::Parser;
use crate::{ChatRequest, Chunk, Client, Result};

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<Chunk>> + Send>>;

struct State {
    body: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    parser: Option<Parser>,
    queue: VecDeque<Result<Chunk>>,
}

impl Client {
    /// Opt-in streaming. Yields `Chunk::Text` as it arrives, then `Chunk::Done(stats)`.
    /// Retries only happen before the first byte; a mid-body failure is yielded as `Err`.
    pub async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream> {
        let resp = self.post_chat(&req).await?;
        let state = State {
            body: Box::pin(resp.bytes_stream()),
            parser: Some(Parser::new()),
            queue: VecDeque::new(),
        };
        Ok(Box::pin(stream::unfold(state, |mut st| async move {
            loop {
                if let Some(item) = st.queue.pop_front() {
                    return Some((item, st));
                }
                let parser = st.parser.as_mut()?;
                match st.body.next().await {
                    Some(Ok(bytes)) => match parser.push(&bytes) {
                        Ok(t) if t.is_empty() => {}
                        Ok(t) => st.queue.push_back(Ok(Chunk::Text(t))),
                        Err(e) => {
                            st.parser = None;
                            st.queue.push_back(Err(e));
                        }
                    },
                    Some(Err(e)) => {
                        st.parser = None;
                        st.queue.push_back(Err(e.into()));
                    }
                    None => match st.parser.take().unwrap().finish() {
                        Ok((rest, stats)) => {
                            if !rest.is_empty() {
                                st.queue.push_back(Ok(Chunk::Text(rest)));
                            }
                            if let Some(s) = stats {
                                st.queue.push_back(Ok(Chunk::Done(s)));
                            }
                        }
                        Err(e) => st.queue.push_back(Err(e)),
                    },
                }
            }
        })))
    }
}
