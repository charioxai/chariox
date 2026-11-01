//! Bounded in-memory holding area for recordings that arrive in chunks.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine as _;

use crate::local::{DICTATION_MAX_AUDIO_BYTES, DICTATION_MAX_CHUNK_BYTES};

const MAX_UPLOADS: usize = 4;
const UPLOAD_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AudioError {
    InvalidId,
    InvalidChunk,
    OutOfOrder { held: u64 },
    TooLong,
    Busy,
    Unknown,
}

struct Upload {
    pcm: Vec<u8>,
    touched: Instant,
}

#[derive(Default)]
pub(crate) struct AudioUploads {
    uploads: HashMap<(String, String), Upload>,
}

impl AudioUploads {
    /// A recording belongs to the user who started it: the same id from another user is another recording.
    pub(crate) fn append(
        &mut self,
        owner: &str,
        upload_id: &str,
        offset: u64,
        pcm_base64: &str,
    ) -> Result<u64, AudioError> {
        let now = Instant::now();
        self.uploads
            .retain(|_, upload| now.duration_since(upload.touched) < UPLOAD_TTL);
        if upload_id.is_empty()
            || upload_id.len() > 64
            || !upload_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(AudioError::InvalidId);
        }
        if pcm_base64.len() > DICTATION_MAX_CHUNK_BYTES * 4 / 3 + 4 {
            return Err(AudioError::InvalidChunk);
        }
        let chunk = base64::engine::general_purpose::STANDARD
            .decode(pcm_base64)
            .map_err(|_| AudioError::InvalidChunk)?;
        if chunk.len() % 2 != 0 {
            return Err(AudioError::InvalidChunk);
        }
        let key = (owner.to_string(), upload_id.to_string());
        if !self.uploads.contains_key(&key) && self.uploads.len() >= MAX_UPLOADS {
            return Err(AudioError::Busy);
        }
        let upload = self.uploads.entry(key.clone()).or_insert_with(|| Upload {
            pcm: Vec::new(),
            touched: now,
        });
        if offset != upload.pcm.len() as u64 {
            return Err(AudioError::OutOfOrder {
                held: upload.pcm.len() as u64,
            });
        }
        if upload.pcm.len() + chunk.len() > DICTATION_MAX_AUDIO_BYTES {
            self.uploads.remove(&key);
            return Err(AudioError::TooLong);
        }
        upload.pcm.extend_from_slice(&chunk);
        upload.touched = now;
        Ok(upload.pcm.len() as u64)
    }

    pub(crate) fn take(&mut self, owner: &str, upload_id: &str) -> Result<Vec<u8>, AudioError> {
        self.uploads
            .remove(&(owner.to_string(), upload_id.to_string()))
            .map(|upload| upload.pcm)
            .ok_or(AudioError::Unknown)
    }
}

/// Converts little-endian signed 16-bit PCM to the float samples whisper expects.
pub(crate) fn pcm16_to_f32(pcm: &[u8]) -> Vec<f32> {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
        .collect()
}
