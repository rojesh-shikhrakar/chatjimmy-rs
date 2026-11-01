mod ask;
mod batch;
mod chat;

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "chatjimmy", about = "Command-line client for chatjimmy.ai")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
    #[arg(long, global = true, default_value = chatjimmy::DEFAULT_MODEL)]
    model: String,
    /// System prompt
    #[arg(long, global = true, default_value = "")]
    system: String,
    #[arg(long, global = true, default_value_t = chatjimmy::DEFAULT_TOP_K)]
    top_k: u32,
    #[arg(long, global = true, default_value = chatjimmy::BASE_URL)]
    base_url: String,
}

#[derive(Subcommand)]
enum Command {
    /// Ask one question. Reads the prompt from stdin if PROMPT is `-` or omitted.
    Ask {
        prompt: Option<String>,
        /// Print text as it arrives
        #[arg(long)]
        stream: bool,
        /// Print stats as JSON to stderr
        #[arg(long)]
        stats: bool,
    },
    /// Interactive multi-turn chat; reads one message per line from stdin and remembers the
    /// conversation. Set --system/--model/--top-k up front. `/reset` clears history, `/exit` quits.
    Chat,
    /// Run many prompts concurrently; one JSON record per request on stdout.
    Batch {
        /// Input file (default: stdin). One plain prompt or JSON request per line.
        file: Option<PathBuf>,
        #[arg(short, long, default_value_t = 16)]
        concurrency: usize,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    // clap exits 2 on usage errors, which would collide with batch's "some requests failed".
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(u8::from(e.use_stderr()));
        }
    };
    match run(cli).await {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("chatjimmy: error: {msg}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, String> {
    let client = chatjimmy::Client::builder()
        .base_url(&cli.base_url)
        .build()
        .map_err(|e| e.to_string())?;
    match cli.cmd {
        Command::Ask { prompt, stream, stats } => {
            let prompt = match prompt.as_deref() {
                Some(p) if p != "-" => p.to_string(),
                _ => strip_bom(&read_stdin()?).trim_end().to_string(),
            };
            if prompt.is_empty() {
                return Err("empty prompt".into());
            }
            let opts = ask::Opts { model: &cli.model, system: &cli.system, top_k: cli.top_k, stream, stats };
            ask::run(&client, &prompt, &opts).await?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Chat => {
            let opts = ask::Opts { model: &cli.model, system: &cli.system, top_k: cli.top_k, stream: true, stats: false };
            chat::run(&client, &opts, std::io::stdin().lock()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Batch { file, concurrency } => {
            let input = match file {
                Some(path) => std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
                None => read_stdin()?,
            };
            let defaults = batch::Defaults { model: cli.model, system_prompt: cli.system, top_k: cli.top_k };
            let mut out = std::io::stdout().lock();
            let lines = strip_bom(&input).lines().map(String::from);
            match batch::run_batch(&client, lines, &defaults, concurrency, &mut out).await {
                Ok(true) => Ok(ExitCode::SUCCESS),
                Ok(false) => Ok(ExitCode::from(2)),
                // The reader closed the pipe (e.g. `| head`): stop quietly.
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(ExitCode::SUCCESS),
                Err(e) => Err(e.to_string()),
            }
        }
    }
}

/// Windows tools often prepend a UTF-8 BOM, which would hide a leading `{`.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

fn read_stdin() -> Result<String, String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(|e| format!("stdin: {e}"))?;
    Ok(s)
}
