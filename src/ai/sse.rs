//! Shared SSE byte-stream → line buffering for provider streaming APIs.
//!
//! Reqwest delivers TCP chunks that often split mid-`data: …` line. Parsing
//! each chunk with `.lines()` alone drops incomplete events and can yield empty
//! assistant turns (especially on OpenCode Zen / DeepSeek).

use crate::ai::types::{ProviderError, StreamDelta};
use futures::StreamExt;

/// Map a reqwest byte stream into parsed SSE deltas, preserving incomplete lines
/// across chunk boundaries.
pub(crate) fn sse_delta_stream<S, B, F>(
    byte_stream: S,
    parse_line: F,
) -> impl futures::Stream<Item = Result<StreamDelta, ProviderError>>
where
    S: futures::Stream<Item = Result<B, reqwest::Error>> + Unpin,
    B: AsRef<[u8]>,
    F: Fn(&str) -> Option<StreamDelta> + Unpin,
{
    struct State<S, F> {
        stream: S,
        buf: String,
        parse_line: F,
        done: bool,
    }

    futures::stream::unfold(
        State {
            stream: byte_stream,
            buf: String::new(),
            parse_line,
            done: false,
        },
        |mut state| async move {
            if state.done {
                return None;
            }
            loop {
                if let Some(pos) = state.buf.find('\n') {
                    let mut line: String = state.buf.drain(..=pos).collect();
                    if line.ends_with('\n') {
                        line.pop();
                    }
                    if line.ends_with('\r') {
                        line.pop();
                    }
                    if let Some(delta) = (state.parse_line)(&line) {
                        return Some((Ok(delta), state));
                    }
                    continue;
                }

                match state.stream.next().await {
                    Some(Ok(bytes)) => {
                        state.buf.push_str(&String::from_utf8_lossy(bytes.as_ref()));
                    }
                    Some(Err(e)) => {
                        state.done = true;
                        return Some((Err(ProviderError::Stream(e.to_string())), state));
                    }
                    None => {
                        state.done = true;
                        let rest = std::mem::take(&mut state.buf);
                        let rest = rest.trim();
                        if !rest.is_empty() {
                            if let Some(delta) = (state.parse_line)(rest) {
                                return Some((Ok(delta), state));
                            }
                        }
                        return None;
                    }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::DeltaType;
    use futures::StreamExt;

    #[tokio::test]
    async fn reassembles_split_sse_lines() {
        let chunks: Vec<Result<Vec<u8>, reqwest::Error>> = vec![
            Ok(b"data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n".to_vec()),
            Ok(b"data: {\"choices\":[{\"del".to_vec()),
            Ok(b"ta\":{\"content\":\"llo\"}}]}\n".to_vec()),
        ];
        let byte_stream = futures::stream::iter(chunks);
        let parse = |line: &str| -> Option<StreamDelta> {
            let line = line.trim();
            if !line.starts_with("data: ") {
                return None;
            }
            let data = line.strip_prefix("data: ")?;
            let v: serde_json::Value = serde_json::from_str(data).ok()?;
            let text = v["choices"][0]["delta"]["content"].as_str()?.to_string();
            Some(StreamDelta {
                content_index: 0,
                r#type: DeltaType::Text { text },
            })
        };
        let mut out = Vec::new();
        let mut s = Box::pin(sse_delta_stream(byte_stream, parse));
        while let Some(item) = s.next().await {
            out.push(item.unwrap());
        }
        assert_eq!(out.len(), 2);
        match &out[0].r#type {
            DeltaType::Text { text } => assert_eq!(text, "he"),
            _ => panic!("expected text"),
        }
        match &out[1].r#type {
            DeltaType::Text { text } => assert_eq!(text, "llo"),
            _ => panic!("expected text"),
        }
    }
}
