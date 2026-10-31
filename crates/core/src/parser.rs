use crate::{Error, Result, Stats};

const START: &str = "<|stats|>";
const END: &str = "<|/stats|>";

/// Incremental parser for a chat body: text followed by an optional stats block.
#[derive(Default)]
pub(crate) struct Parser {
    /// Bytes of a UTF-8 character split across pushes.
    raw: Vec<u8>,
    /// Decoded text not yet emitted (may end in a partial start marker).
    held: String,
    /// Everything after the start marker.
    stats_buf: Option<String>,
    emitted: usize,
}

impl Parser {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; returns text that is safe to emit now.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<String> {
        self.raw.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.raw) {
            Ok(s) => s.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => return Err(Error::Decode),
        };
        let decoded: String = std::str::from_utf8(&self.raw[..valid]).unwrap().into();
        self.raw.drain(..valid);

        if let Some(buf) = &mut self.stats_buf {
            buf.push_str(&decoded);
            return Ok(String::new());
        }
        self.held.push_str(&decoded);

        let out = if let Some(i) = self.held.find(START) {
            self.stats_buf = Some(self.held[i + START.len()..].to_string());
            self.held.truncate(i);
            std::mem::take(&mut self.held)
        } else {
            let mut cut = self.held.len().saturating_sub(START.len() - 1);
            while !self.held.is_char_boundary(cut) {
                cut -= 1;
            }
            self.held.drain(..cut).collect()
        };
        self.emitted += out.len();
        Ok(out)
    }

    /// End of body: returns remaining text and parsed stats.
    pub(crate) fn finish(mut self) -> Result<(String, Option<Stats>)> {
        if !self.raw.is_empty() {
            return Err(Error::Decode);
        }
        let mut rest = std::mem::take(&mut self.held);
        // The real stats block is the last START..END pair at the end of the body. A START
        // inside the reply, or one with no END after it, is ordinary text. Anything after
        // END is dropped.
        let mut stats = None;
        if let Some(buf) = &self.stats_buf {
            match buf.rfind(END) {
                None => {
                    rest.push_str(START);
                    rest.push_str(buf);
                }
                Some(end) => {
                    let head = &buf[..end];
                    let json = match head.rsplit_once(START) {
                        Some((text, json)) => {
                            rest.push_str(START);
                            rest.push_str(text);
                            json
                        }
                        None => head,
                    };
                    stats = Some(serde_json::from_str(json).map_err(Error::BadStats)?);
                }
            }
        }
        self.emitted += rest.len();
        if self.emitted == 0 && stats.is_none() {
            return Err(Error::EmptyResponse);
        }
        Ok((rest, stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    fn run(chunks: &[&[u8]]) -> crate::Result<(String, Option<crate::Stats>)> {
        let mut p = Parser::new();
        let mut text = String::new();
        for c in chunks {
            text += &p.push(c)?;
        }
        let (rest, stats) = p.finish()?;
        Ok((text + &rest, stats))
    }

    const BODY: &str = "hello<|stats|>{\"decode_tokens\":3}<|/stats|>";

    #[test]
    fn text_then_stats() {
        let (t, s) = run(&[BODY.as_bytes()]).unwrap();
        assert_eq!(t, "hello");
        assert_eq!(s.unwrap().decode_tokens, 3);
    }

    #[test]
    fn split_at_every_byte_offset() {
        let b = BODY.as_bytes();
        for i in 0..=b.len() {
            let (t, s) = run(&[&b[..i], &b[i..]]).unwrap();
            assert_eq!(t, "hello", "offset {i}");
            assert_eq!(s.unwrap().decode_tokens, 3, "offset {i}");
        }
    }

    #[test]
    fn multibyte_split() {
        let b = "é😀".as_bytes();
        for i in 0..=b.len() {
            let (t, _) = run(&[&b[..i], &b[i..]]).unwrap();
            assert_eq!(t, "é😀", "offset {i}");
        }
    }

    #[test]
    fn no_stats_block() {
        let (t, s) = run(&[b"plain"]).unwrap();
        assert_eq!(t, "plain");
        assert!(s.is_none());
    }

    #[test]
    fn empty_body_is_empty_response() {
        assert!(matches!(run(&[]), Err(Error::EmptyResponse)));
        assert!(matches!(run(&[b""]), Err(Error::EmptyResponse)));
    }

    #[test]
    fn malformed_stats_is_bad_stats() {
        let r = run(&[b"x<|stats|>{nope<|/stats|>"]);
        assert!(matches!(r, Err(Error::BadStats(_))));
    }

    #[test]
    fn unknown_stats_fields_ignored() {
        let (_, s) = run(&[b"x<|stats|>{\"bogus\":1,\"decode_tokens\":2}<|/stats|>"]).unwrap();
        assert_eq!(s.unwrap().decode_tokens, 2);
    }

    #[test]
    fn partial_marker_not_emitted() {
        let mut p = Parser::new();
        let out = p.push(b"abc<|sta").unwrap();
        assert!(!out.contains("<|sta"), "emitted {out:?}");
    }

    #[test]
    fn marker_in_reply_is_text() {
        let (t, s) =
            run(&[b"use <|stats|> marker<|stats|>{\"decode_tokens\":1}<|/stats|>"]).unwrap();
        assert_eq!(t, "use <|stats|> marker");
        assert_eq!(s.unwrap().decode_tokens, 1);
    }

    #[test]
    fn marker_without_end_is_text() {
        let (t, s) = run(&[b"use <|stats|> only"]).unwrap();
        assert_eq!(t, "use <|stats|> only");
        assert!(s.is_none());
    }

    #[test]
    fn invalid_utf8_is_decode_error() {
        let mut p = Parser::new();
        assert!(matches!(p.push(&[0xff, 0xfe]), Err(Error::Decode)));
    }
}
