//! Protocol-agnostic Server-Sent Events framing.
//!
//! This module only splits a byte stream into SSE events (`event` / `id` / `data`).
//! Protocol-specific JSON decoding belongs in each adapter.
//!
//! Framing operates on raw bytes so multi-byte UTF-8 characters (e.g. CJK, emoji)
//! that straddle TCP chunk boundaries are never decoded with `from_utf8_lossy`.

/// One complete SSE event after frame boundaries are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// Optional `event:` field.
    pub event: Option<String>,
    /// Optional `id:` field.
    pub id: Option<String>,
    /// Joined `data:` lines (SSE spec: multiple data lines joined by `\n`).
    pub data: String,
}

/// Incremental SSE event buffer. Safe across arbitrary TCP chunk sizes and UTF-8 boundaries.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    /// Create an empty parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push arbitrary bytes and return any complete events.
    ///
    /// Incomplete UTF-8 sequences at the end of the buffer are retained until more
    /// bytes arrive. Complete events with invalid UTF-8 field content are skipped
    /// (callers that need hard failures should validate event payloads themselves).
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        loop {
            let sep = find_event_separator(&self.buffer);
            let Some((idx, sep_len)) = sep else {
                break;
            };

            let raw_event = self.buffer[..idx].to_vec();
            self.buffer.drain(..idx + sep_len);
            if let Some(event) = parse_raw_event(&raw_event) {
                events.push(event);
            }
        }

        events
    }

    /// Flush a trailing partial event when the stream ends without a final separator.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.buffer.iter().all(|b| b.is_ascii_whitespace()) {
            self.buffer.clear();
            return None;
        }
        let raw = std::mem::take(&mut self.buffer);
        parse_raw_event(&raw)
    }
}

/// Locate the earliest SSE event separator (`\r\n\r\n` or `\n\n`).
fn find_event_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = find_subslice(buffer, b"\r\n\r\n").map(|idx| (idx, 4));
    let lf = find_subslice(buffer, b"\n\n").map(|idx| (idx, 2));
    match (crlf, lf) {
        (Some(a), Some(b)) => {
            if a.0 <= b.0 {
                Some(a)
            } else {
                Some(b)
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_raw_event(raw_event: &[u8]) -> Option<SseEvent> {
    let mut event_name: Option<String> = None;
    let mut id: Option<String> = None;
    let mut data_lines = Vec::new();

    for line in split_lines(raw_event) {
        if line.is_empty() || line.first() == Some(&b':') {
            continue;
        }
        if let Some(rest) = strip_prefix_bytes(line, b"event:") {
            event_name = Some(decode_field(strip_one_leading_space_bytes(rest))?);
            continue;
        }
        if let Some(rest) = strip_prefix_bytes(line, b"id:") {
            id = Some(decode_field(strip_one_leading_space_bytes(rest))?);
            continue;
        }
        if let Some(rest) = strip_prefix_bytes(line, b"data:") {
            data_lines.push(decode_field(strip_one_leading_space_bytes(rest))?);
            continue;
        }
        // ignore retry: and unknown fields
    }

    if data_lines.is_empty() && event_name.is_none() && id.is_none() {
        return None;
    }

    Some(SseEvent {
        event: event_name,
        id,
        data: data_lines.join("\n"),
    })
}

fn split_lines(raw: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\n' {
            let mut end = i;
            if end > start && raw[end - 1] == b'\r' {
                end -= 1;
            }
            lines.push(&raw[start..end]);
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    if start < raw.len() {
        let mut end = raw.len();
        if end > start && raw[end - 1] == b'\r' {
            end -= 1;
        }
        lines.push(&raw[start..end]);
    } else if start == raw.len() && !raw.is_empty() && raw.ends_with(b"\n") {
        // trailing newline already handled; no extra empty line needed beyond empties from \n\n
    }
    lines
}

fn strip_prefix_bytes<'a>(input: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    input.strip_prefix(prefix)
}

fn strip_one_leading_space_bytes(value: &[u8]) -> &[u8] {
    value.strip_prefix(b" ").unwrap_or(value)
}

fn decode_field(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_crlf_comments_and_data() {
        let mut parser = SseParser::new();
        let mut out = Vec::new();
        out.extend(parser.push(b": keep-alive\r\n\r\n"));
        out.extend(parser.push(b"data: hello\r\n\r\n"));
        out.extend(parser.push(b"event: delta\ndata: {\"x\":1}\n\n"));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].data, "hello");
        assert_eq!(out[1].event.as_deref(), Some("delta"));
        assert_eq!(out[1].data, "{\"x\":1}");
    }

    #[test]
    fn joins_multiple_data_lines() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn parses_event_name_for_future_protocols() {
        let mut parser = SseParser::new();
        let events = parser.push(b"event: message_stop\ndata: {}\n\n");
        assert_eq!(events[0].event.as_deref(), Some("message_stop"));
        assert_eq!(events[0].data, "{}");
    }

    #[test]
    fn chinese_utf8_split_across_tcp_chunks_does_not_produce_replacement_chars() {
        // "你" is E4 BD A0
        let prefix = b"data: {\"choices\":[{\"delta\":{\"content\":\"";
        let chinese = "你".as_bytes(); // [0xE4, 0xBD, 0xA0]
        let suffix = b"\"}}]}\n\n";

        let mut first = prefix.to_vec();
        first.extend_from_slice(&chinese[..2]); // incomplete UTF-8

        let mut second = chinese[2..].to_vec();
        second.extend_from_slice(suffix);

        let mut parser = SseParser::new();
        let mut events = parser.push(&first);
        assert!(events.is_empty(), "incomplete UTF-8 must not emit an event");
        events.extend(parser.push(&second));
        assert_eq!(events.len(), 1);
        assert!(
            !events[0].data.contains('\u{FFFD}'),
            "must not contain U+FFFD replacement char, got {:?}",
            events[0].data
        );
        assert!(
            events[0].data.contains('你'),
            "expected chinese character in data, got {:?}",
            events[0].data
        );
    }

    #[test]
    fn emoji_utf8_split_across_chunks() {
        // 😀 is F0 9F 98 80
        let emoji = "😀".as_bytes();
        assert_eq!(emoji.len(), 4);

        let mut first = b"data: ".to_vec();
        first.extend_from_slice(&emoji[..2]);
        let mut second = emoji[2..].to_vec();
        second.extend_from_slice(b"\n\n");

        let mut parser = SseParser::new();
        assert!(parser.push(&first).is_empty());
        let events = parser.push(&second);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "😀");
        assert!(!events[0].data.contains('\u{FFFD}'));
    }

    #[test]
    fn sse_frame_split_mid_json() {
        // Build as String so multi-byte UTF-8 is allowed, then frame as bytes.
        let full = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            "中"
        );
        let full = full.as_bytes();
        let split_at = full.len() / 2;
        let mut parser = SseParser::new();
        let mut events = parser.push(&full[..split_at]);
        events.extend(parser.push(&full[split_at..]));
        assert_eq!(events.len(), 1);
        assert!(events[0].data.contains('中'));
        assert!(!events[0].data.contains('\u{FFFD}'));
    }

    #[test]
    fn finish_flushes_trailing_event_without_separator() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: tail").is_empty());
        let event = parser.finish().expect("trailing event");
        assert_eq!(event.data, "tail");
    }
}
