//! Hits the real chatjimmy.ai. Run with `cargo test -- --ignored`.
use std::process::Command;

#[test]
#[ignore]
fn live_ask_cli() {
    let o = Command::new(env!("CARGO_BIN_EXE_chatjimmy")).args(["ask", "Say hi"]).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!o.stdout.is_empty());
}
