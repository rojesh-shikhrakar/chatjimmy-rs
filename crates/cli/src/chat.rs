use std::io::{BufRead, IsTerminal, Write};

use chatjimmy::{ChatRequest, Chunk, Client, Message};
use futures::StreamExt;

use crate::ask::{emit, Opts};

/// Interactive multi-turn chat: every request carries the full history.
/// `/reset` clears it, `/exit` (or EOF) quits. The prompt goes to stderr and only on a terminal.
pub async fn run(client: &Client, o: &Opts<'_>, input: impl BufRead) -> Result<(), String> {
    let tty = std::io::stdin().is_terminal();
    let mut history: Vec<Message> = Vec::new();
    let prompt = || {
        if tty {
            eprint!("> ");
            let _ = std::io::stderr().flush();
        }
    };
    prompt();
    for line in input.lines() {
        let line = line.map_err(|e| format!("stdin: {e}"))?;
        match line.trim() {
            "" => {}
            "/exit" | "/quit" => break,
            "/reset" => {
                history.clear();
                if tty {
                    eprintln!("(history cleared)");
                }
            }
            text => {
                history.push(Message::user(text));
                match turn(client, &history, o).await {
                    Ok(Some(reply)) => history.push(Message::assistant(reply)),
                    Ok(None) => return Ok(()), // reader gone
                    Err(e) => {
                        history.pop(); // failed turn must not poison the context
                        eprintln!("chatjimmy: error: {e}");
                    }
                }
            }
        }
        prompt();
    }
    Ok(())
}

/// Streams the reply to stdout and returns it. `None` = stdout closed.
async fn turn(client: &Client, history: &[Message], o: &Opts<'_>) -> Result<Option<String>, String> {
    let req = ChatRequest::new(history.iter().cloned())
        .model(o.model)
        .system_prompt(o.system)
        .top_k(o.top_k);
    let mut stream = client.chat_stream(req).await.map_err(|e| e.to_string())?;
    let mut reply = String::new();
    while let Some(chunk) = stream.next().await {
        if let Chunk::Text(t) = chunk.map_err(|e| e.to_string())? {
            if !emit(&t)? {
                return Ok(None);
            }
            reply += &t;
        }
    }
    Ok(emit("\n")?.then_some(reply))
}
