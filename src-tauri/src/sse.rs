#[derive(Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    event_size: usize,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        let mut events = Vec::new();
        for byte in bytes {
            self.event_size += 1;
            if self.event_size > 1_048_576 {
                return Err("Stream event exceeds 1 MiB.".into());
            }
            if *byte != b'\n' {
                self.pending.push(*byte);
                continue;
            }
            if self.pending.last() == Some(&b'\r') {
                self.pending.pop();
            }
            let line =
                std::str::from_utf8(&self.pending).map_err(|_| "Invalid UTF-8 in event stream.")?;
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
                self.event_size = 0;
            } else if let Some(value) = line.strip_prefix("data:") {
                self.data
                    .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
            }
            self.pending.clear();
        }
        Ok(events)
    }
    pub fn finish(&self) -> Result<(), String> {
        if !self.pending.is_empty() || !self.data.is_empty() {
            return Err("Model disconnected in the middle of a stream event.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unicode_across_network_boundaries_and_accepts_crlf() {
        let mut decoder = SseDecoder::default();
        let bytes = "data: {\"text\":\"你好\"}\r\n\r\ndata: [DONE]\n\n".as_bytes();
        let mut events = Vec::new();
        for byte in bytes {
            events.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(events, ["{\"text\":\"你好\"}", "[DONE]"]);
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn ignores_comments_and_joins_multiple_data_lines() {
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder
                .push(b": heartbeat\nevent: message\ndata: one\ndata: two\n\n")
                .unwrap(),
            ["one\ntwo"]
        );
    }

    #[test]
    fn rejects_invalid_utf8_oversized_events_and_truncated_streams() {
        assert!(SseDecoder::default().push(b"data: \xff\n\n").is_err());
        assert!(SseDecoder::default().push(&vec![b'x'; 1_048_577]).is_err());
        let mut decoder = SseDecoder::default();
        decoder.push(b"data: incomplete").unwrap();
        assert!(decoder.finish().is_err());
    }
}
