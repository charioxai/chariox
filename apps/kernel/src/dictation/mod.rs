//! Kernel dictation: speech-to-text on the kernel the person is connected to.
//!
//! Recordings arrive as 16 kHz mono PCM, are transcribed by whisper.cpp and never
//! stored. The speech model is not part of the kernel: it downloads on first use.
mod audio;
mod catalog;
mod download;
mod engine;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use self::audio::{pcm16_to_f32, AudioError, AudioUploads};
pub(crate) use self::catalog::ModelSpec;
pub(crate) use self::engine::SpeechEngine;
#[cfg(test)]
use self::engine::Transcription;
use self::engine::WhisperEngine;
use crate::error::DaemonError;
use crate::local::{
    DictationCommand, DictationModelState, DictationResult, DictationStatus,
    DICTATION_SAMPLE_RATE_HZ,
};

const RELEASE_MODEL_AFTER: Duration = Duration::from_secs(300);
const MIN_SPEECH_BYTES: usize = DICTATION_SAMPLE_RATE_HZ as usize * 2 * 3 / 10;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    /// The language each user chose; users without an entry get automatic detection.
    languages: HashMap<String, String>,
}

enum Download {
    Idle,
    Running(Arc<AtomicU64>),
    Failed(String),
}

struct Inner {
    download: Download,
    settings: Settings,
    uploads: AudioUploads,
}

pub(crate) struct DictationService {
    dir: PathBuf,
    spec: ModelSpec,
    engine: Arc<dyn SpeechEngine>,
    inner: Mutex<Inner>,
    transcribing: tokio::sync::Semaphore,
    /// Changes whenever the model is touched, so only the newest idle timer releases it.
    model_use: Arc<AtomicU64>,
}

fn refuse(code: &str, message: &str, retryable: bool) -> DaemonError {
    DaemonError::RelayTransport {
        operation: "dictation",
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

impl DictationService {
    pub(crate) fn new(dir: PathBuf) -> Arc<Self> {
        Self::with_parts(dir, ModelSpec::pinned(), Arc::new(WhisperEngine::default()))
    }

    pub(crate) fn with_parts(
        dir: PathBuf,
        spec: ModelSpec,
        engine: Arc<dyn SpeechEngine>,
    ) -> Arc<Self> {
        let settings = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Arc::new(Self {
            dir,
            spec,
            engine,
            inner: Mutex::new(Inner {
                download: Download::Idle,
                settings,
                uploads: AudioUploads::default(),
            }),
            transcribing: tokio::sync::Semaphore::new(1),
            model_use: Arc::new(AtomicU64::new(0)),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) async fn handle(
        self: &Arc<Self>,
        owner: &str,
        command: DictationCommand,
    ) -> Result<DictationResult, DaemonError> {
        match command {
            DictationCommand::Status => Ok(DictationResult::Status {
                status: self.status(owner),
            }),
            DictationCommand::DownloadModel => Ok(DictationResult::Status {
                status: self.start_download(owner),
            }),
            DictationCommand::SetLanguage { language } => self.set_language(owner, language),
            DictationCommand::AppendAudio {
                upload_id,
                offset,
                pcm_base64,
            } => {
                let received_bytes = self
                    .lock()
                    .uploads
                    .append(owner, &upload_id, offset, &pcm_base64)
                    .map_err(audio_refusal)?;
                Ok(DictationResult::AudioAccepted {
                    upload_id,
                    received_bytes,
                })
            }
            DictationCommand::Transcribe { upload_id } => self.transcribe(owner, &upload_id).await,
        }
    }

    fn model_ready(&self) -> bool {
        std::fs::metadata(self.spec.path(&self.dir))
            .is_ok_and(|meta| meta.len() == self.spec.size_bytes)
    }

    fn status(&self, owner: &str) -> DictationStatus {
        let inner = self.lock();
        let model = if self.model_ready() {
            DictationModelState::Ready
        } else {
            match &inner.download {
                Download::Running(received) => DictationModelState::Downloading {
                    received_bytes: received.load(Ordering::Relaxed),
                    total_bytes: self.spec.size_bytes,
                },
                Download::Failed(reason) => DictationModelState::Failed {
                    reason: reason.clone(),
                },
                Download::Idle => DictationModelState::NotDownloaded,
            }
        };
        DictationStatus {
            model_id: self.spec.id.clone(),
            model_size_bytes: self.spec.size_bytes,
            model,
            language: inner.settings.languages.get(owner).cloned(),
        }
    }

    fn start_download(self: &Arc<Self>, owner: &str) -> DictationStatus {
        let received = Arc::new(AtomicU64::new(0));
        {
            let mut inner = self.lock();
            if self.model_ready() || matches!(inner.download, Download::Running(_)) {
                drop(inner);
                return self.status(owner);
            }
            inner.download = Download::Running(received.clone());
        }
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let outcome = download::download_model(&service.spec, &service.dir, &received);
            service.lock().download = match outcome {
                Ok(()) => Download::Idle,
                Err(reason) => Download::Failed(reason),
            };
        });
        self.status(owner)
    }

    fn set_language(
        &self,
        owner: &str,
        language: Option<String>,
    ) -> Result<DictationResult, DaemonError> {
        if language
            .as_deref()
            .is_some_and(|code| !engine::is_language_code(code))
        {
            return Err(refuse(
                "dictation_language_invalid",
                "Choose a two-letter language code such as `en` or `es`, or none to detect it.",
                false,
            ));
        }
        {
            let mut inner = self.lock();
            match language {
                Some(code) => inner.settings.languages.insert(owner.to_string(), code),
                None => inner.settings.languages.remove(owner),
            };
            let bytes = serde_json::to_vec(&inner.settings).map_err(|error| {
                refuse(
                    "dictation_settings_failed",
                    &format!("cannot save the dictation settings: {error}"),
                    false,
                )
            })?;
            std::fs::create_dir_all(&self.dir)
                .and_then(|()| {
                    crate::config::write_private_file(&self.dir.join("settings.json"), &bytes)
                })
                .map_err(|error| {
                    refuse(
                        "dictation_settings_failed",
                        &format!("cannot save the dictation settings: {error}"),
                        true,
                    )
                })?;
        }
        Ok(DictationResult::Status {
            status: self.status(owner),
        })
    }

    async fn transcribe(
        self: &Arc<Self>,
        owner: &str,
        upload_id: &str,
    ) -> Result<DictationResult, DaemonError> {
        if !self.model_ready() {
            return Err(refuse(
                "dictation_model_missing",
                "The speech model is not downloaded. Download it, then dictate again.",
                false,
            ));
        }
        let (pcm, language) = {
            let mut inner = self.lock();
            (
                inner
                    .uploads
                    .take(owner, upload_id)
                    .map_err(audio_refusal)?,
                inner.settings.languages.get(owner).cloned(),
            )
        };
        if pcm.len() < MIN_SPEECH_BYTES {
            return Ok(DictationResult::Transcript {
                text: String::new(),
                language: language.unwrap_or_default(),
            });
        }
        let _permit = self.transcribing.acquire().await.map_err(|_| {
            refuse(
                "dictation_busy",
                "Dictation is shutting down. Try again.",
                true,
            )
        })?;
        self.model_use.fetch_add(1, Ordering::SeqCst);
        let service = self.clone();
        let attempt = tokio::task::spawn_blocking(move || {
            service.engine.transcribe(
                &service.spec.path(&service.dir),
                &pcm16_to_f32(&pcm),
                language.as_deref(),
            )
        })
        .await;
        self.release_model_when_idle();
        let heard = attempt
            .map_err(|_| {
                refuse(
                    "dictation_transcription_failed",
                    "Transcription stopped unexpectedly. Try again.",
                    true,
                )
            })?
            .map_err(|message| refuse("dictation_transcription_failed", &message, true))?;
        Ok(DictationResult::Transcript {
            text: spoken_text(&heard.text),
            language: heard.language,
        })
    }
}

impl DictationService {
    /// Runs after every attempt that can have loaded the model, failed or not.
    fn release_model_when_idle(&self) {
        let latest = self.model_use.fetch_add(1, Ordering::SeqCst) + 1;
        let engine = self.engine.clone();
        let model_use = self.model_use.clone();
        tokio::spawn(async move {
            tokio::time::sleep(RELEASE_MODEL_AFTER).await;
            if model_use.load(Ordering::SeqCst) == latest {
                engine.release();
            }
        });
    }
}

/// Drops whisper's non-speech annotations such as `[BLANK_AUDIO]` and `(music)`.
fn spoken_text(raw: &str) -> String {
    let mut text = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find(['[', '(']) {
        let closer = if rest.as_bytes()[start] == b'[' {
            ']'
        } else {
            ')'
        };
        match rest[start..].find(closer) {
            Some(end) => {
                text.push_str(&rest[..start]);
                rest = &rest[start + end + 1..];
            }
            None => break,
        }
    }
    text.push_str(rest);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn audio_refusal(error: AudioError) -> DaemonError {
    match error {
        AudioError::InvalidId => refuse("dictation_audio_invalid", "The recording id is invalid.", false),
        AudioError::InvalidChunk => refuse("dictation_audio_invalid", "The recording chunk is not 16 kHz 16-bit PCM or is too large.", false),
        AudioError::OutOfOrder { held } => refuse("dictation_audio_out_of_order", &format!("The kernel holds {held} bytes of this recording; send the next chunk at that offset."), false),
        AudioError::TooLong => refuse("dictation_audio_too_long", "The recording is longer than two minutes. Dictate in shorter parts.", false),
        AudioError::Busy => refuse("dictation_busy", "Too many recordings are in progress. Try again in a moment.", true),
        AudioError::Unknown => refuse("dictation_audio_unknown", "The kernel has no recording with this id. Record again.", false),
    }
}

#[cfg(test)]
mod tests;
