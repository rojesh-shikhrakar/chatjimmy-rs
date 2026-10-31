use std::io::Write;

use chatjimmy::{ChatRequest, Chunk, Client, Message, Result, Stats};
use futures::StreamExt;

pub struct Opts<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub top_k: u32,
    pub stream: bool,
    pub stats: bool,
}

/// Prints the reply to stdout (all at once, or as it arrives with `stream`), then a newline.
pub async fn run(client: &Client, prompt: &str, o: &Opts<'_>) -> Result<()> {
    let req = ChatRequest::new([Message::user(prompt)])
        .model(o.model)
        .system_prompt(o.system)
        .top_k(o.top_k);
    let stats = if o.stream {
        let mut stream = client.chat_stream(req).await?;
        let mut stats = None;
        while let Some(chunk) = stream.next().await {
            match chunk? {
                Chunk::Text(t) => emit(&t),
                Chunk::Done(s) => stats = Some(s),
            }
        }
        stats
    } else {
        let r = client.chat(req).await?;
        emit(&r.text);
        r.stats
    };
    emit("\n");
    if let (true, Some(s)) = (o.stats, stats) {
        print_stats(&s);
    }
    Ok(())
}

fn emit(text: &str) {
    let mut out = std::io::stdout();
    // A closed pipe is the reader's choice, not an error.
    let _ = out.write_all(text.as_bytes()).and_then(|_| out.flush());
}

fn print_stats(s: &Stats) {
    eprintln!("{}", serde_json::to_string(s).expect("Stats serializes"));
}
