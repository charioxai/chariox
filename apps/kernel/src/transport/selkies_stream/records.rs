use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;

use super::stream_error;
use crate::error::DaemonError;
use crate::transport::secure_display::{DisplayMessage, DisplayMessageKind};

const MAX_LINE_BYTES: usize = 6 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PrivateRecord {
    Ready { protocol: String, read_only: bool },
    Text { text: String },
    Binary { data_base64: String },
    Closed { reason: String },
}

impl PrivateRecord {
    pub(super) fn into_message(self) -> Result<DisplayMessage, DaemonError> {
        match self {
            Self::Binary { data_base64 } => {
                let data = BASE64
                    .decode(data_base64)
                    .map_err(|_| stream_error("invalid private video encoding"))?;
                if data.len() <= 10 || data.len() > 4 * 1024 * 1024 || data[0] != 4 {
                    return Err(stream_error("invalid private video packet"));
                }
                Ok(DisplayMessage {
                    kind: DisplayMessageKind::Binary,
                    data,
                })
            }
            Self::Text { text }
                if matches!(
                    text.as_str(),
                    "PIPELINE_RESETTING primary" | "VIDEO_STARTED" | "VIDEO_STOPPED"
                ) =>
            {
                Ok(DisplayMessage {
                    kind: DisplayMessageKind::Text,
                    data: text.into_bytes(),
                })
            }
            Self::Closed { reason } => {
                // Do not propagate arbitrary helper text through public errors.
                let _ = reason;
                Err(stream_error("private stream closed"))
            }
            _ => Err(stream_error("unexpected private stream record")),
        }
    }
}

pub(super) struct RecordReader {
    reader: BufReader<ChildStdout>,
    pending: Vec<u8>,
}

impl RecordReader {
    pub(super) fn new(output: ChildStdout) -> Self {
        Self {
            reader: BufReader::new(output),
            pending: Vec::new(),
        }
    }

    /// The partial record lives on the reader, so select cancellation cannot
    /// discard bytes. Check the limit before extending, including without LF.
    pub(super) async fn next(&mut self) -> Result<PrivateRecord, DaemonError> {
        loop {
            let chunk = self
                .reader
                .fill_buf()
                .await
                .map_err(|_| stream_error("private stream read failed"))?;
            if chunk.is_empty() {
                return Err(stream_error("private stream ended"));
            }
            let end = chunk.iter().position(|byte| *byte == b'\n');
            let count = end.map(|index| index + 1).unwrap_or(chunk.len());
            if self.pending.len() + count > MAX_LINE_BYTES {
                return Err(stream_error("private stream record exceeds limit"));
            }
            self.pending.extend_from_slice(&chunk[..count]);
            self.reader.consume(count);
            if end.is_some() {
                let record = serde_json::from_slice(&self.pending)
                    .map_err(|_| stream_error("malformed private stream record"));
                self.pending.clear();
                return record;
            }
        }
    }

    pub(super) async fn drain(&mut self) {
        self.pending.clear();
        let _ = tokio::io::copy(&mut self.reader, &mut tokio::io::sink()).await;
    }
}
