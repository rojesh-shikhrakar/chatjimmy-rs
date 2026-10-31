use std::io::Write;

use chatjimmy::{ChatRequest, Chunk, Client, Message, Stats};
use futures::StreamExt;

pub struct Opts<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub top_k: u32,
    pub stream: bool,
    pub stats: bool,
}

/// Prints the reply to stdout (all at once, or as it arrives with `stream`), then a newline.
pub async fn run(client: &Client, prompt: &str, o: &Opts<'_>) -> Result<(), String> {
    let req = ChatRequest::new([Message::user(prompt)])
        .model(o.model)
        .system_prompt(o.system)
        .top_k(o.top_k);
    let stats = if o.stream {
        let mut stream = client.chat_stream(req).await.map_err(|e| e.to_string())?;
        let mut stats = None;
        while let Some(chunk) = stream.next().await {
            match chunk.map_err(|e| e.to_string())? {
                Chunk::Text(t) => {
                    if !emit(&t)? {
                        return Ok(()); // reader gone: stop downloading
                    }
                }
                Chunk::Done(s) => stats = Some(s),
            }
        }
        stats
    } else {
        let r = client.chat(req).await.map_err(|e| e.to_string())?;
        if !emit(&r.text)? {
            return Ok(());
        }
        r.stats
    };
    if !emit("\n")? {
        return Ok(());
    }
    if let (true, Some(s)) = (o.stats, stats) {
        print_stats(&s);
    }
    Ok(())
}

/// Writes to stdout. `Ok(false)` means the reader closed the pipe (stop quietly); other write
/// errors are real failures.
fn emit(text: &str) -> Result<bool, String> {
    let mut out = std::io::stdout();
    match out.write_all(text.as_bytes()).and_then(|_| out.flush()) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn print_stats(s: &Stats) {
    eprintln!("{}", serde_json::to_string(s).expect("Stats serializes"));
}
