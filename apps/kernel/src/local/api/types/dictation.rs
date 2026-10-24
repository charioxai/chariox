//! Kernel dictation: speech-to-text on the kernel the user is connected to.
use super::*;

/// Recorded speech is 16 kHz mono signed 16-bit little-endian PCM, so the kernel
/// needs no audio decoders and every browser can produce it.
pub const DICTATION_SAMPLE_RATE_HZ: u32 = 16_000;
pub const DICTATION_MAX_AUDIO_BYTES: usize = 120 * DICTATION_SAMPLE_RATE_HZ as usize * 2;
pub const DICTATION_MAX_CHUNK_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DictationCommand {
    /// Whether the speech model is available; never starts a download.
    Status,
    /// Starts or resumes the pinned model download after the person agreed.
    DownloadModel,
    SetLanguage {
        /// ISO 639-1 code such as `es`; `None` lets the model detect the language.
        language: Option<String>,
    },
    /// Appends PCM bytes at `offset`; the offset must equal the bytes held so far.
    AppendAudio {
        upload_id: String,
        offset: u64,
        pcm_base64: String,
    },
    /// Transcribes and discards the upload.
    Transcribe { upload_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictationRequest {
    pub command: DictationCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum DictationModelState {
    NotDownloaded,
    Downloading {
        received_bytes: u64,
        total_bytes: u64,
    },
    Ready,
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictationStatus {
    pub model_id: String,
    pub model_size_bytes: u64,
    pub model: DictationModelState,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum DictationResult {
    Status {
        status: DictationStatus,
    },
    AudioAccepted {
        upload_id: String,
        received_bytes: u64,
    },
    Transcript {
        text: String,
        language: String,
    },
}
