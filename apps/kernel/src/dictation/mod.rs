//! Kernel dictation: speech-to-text on the kernel the person is connected to.
//!
//! Recordings arrive as 16 kHz mono PCM and are never stored. The kernel runs
//! whisper.cpp locally or sends a WAV to the selected speech service using the
//! authenticated user’s Vault key. The local model downloads only on first use.
mod audio;
mod catalog;
mod download;
mod engine;
mod remote;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use self::audio::{pcm16_to_f32, AudioError, AudioUploads};
pub(crate) use self::catalog::ModelSpec;
pub(crate) use self::engine::SpeechEngine;
use self::engine::Transcription;
use self::engine::WhisperEngine;
use crate::error::DaemonError;
use crate::local::{
    DictationCommand, DictationKeyState, DictationModelState, DictationProvider, DictationResult,
    DictationStatus, DICTATION_SAMPLE_RATE_HZ,
};

const RELEASE_MODEL_AFTER: Duration = Duration::from_secs(300);
const MIN_SPEECH_BYTES: usize = DICTATION_SAMPLE_RATE_HZ as usize * 2 * 3 / 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    /// None migrates an already downloaded model or remote selection from earlier releases.
    configured: Option<bool>,
    service: DictationProvider,
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
    services: remote::SpeechServices,
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
            services: remote::SpeechServices::default(),
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

    #[cfg(test)]
    pub(crate) async fn handle(
        self: &Arc<Self>,
        owner: &str,
        command: DictationCommand,
    ) -> Result<DictationResult, DaemonError> {
        self.handle_with_vault(owner, command, None).await
    }

    #[cfg(test)]
    pub(crate) async fn handle_with_vault(
        self: &Arc<Self>,
        owner: &str,
        command: DictationCommand,
        vault: Option<crate::config::UserCredentialVaultConfig>,
    ) -> Result<DictationResult, DaemonError> {
        self.handle_with_provider(owner, command, vault, self.selected_service())
            .await
    }

    pub(crate) async fn handle_with_provider(
        self: &Arc<Self>,
        owner: &str,
        command: DictationCommand,
        vault: Option<crate::config::UserCredentialVaultConfig>,
        provider: DictationProvider,
    ) -> Result<DictationResult, DaemonError> {
        let result = match command {
            DictationCommand::Status => Ok(DictationResult::Status {
                status: self.status(owner),
            }),
            DictationCommand::DownloadModel => {
                self.set_configured(owner, true)?;
                Ok(DictationResult::Status {
                    status: self.start_download(owner),
                })
            }
            DictationCommand::SetService { service } => self.set_service(owner, service),
            DictationCommand::ClearService => self.set_configured(owner, false),
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
            DictationCommand::Transcribe { upload_id, .. } => {
                self.transcribe_with_provider(owner, &upload_id, vault.clone(), provider)
                    .await
            }
        }?;
        Ok(match result {
            DictationResult::Status { mut status } => {
                if let Some(key) = status.vault_key.as_deref() {
                    status.key_state = match read_key(vault.as_ref(), key) {
                        Ok(_) => DictationKeyState::Saved,
                        Err(DaemonError::RelayTransport { code, .. })
                            if code == "dictation_vault_locked" =>
                        {
                            DictationKeyState::Locked
                        }
                        Err(_) => DictationKeyState::Missing,
                    };
                }
                DictationResult::Status { status }
            }
            other => other,
        })
    }

    pub(crate) fn selected_service(&self) -> DictationProvider {
        self.lock().settings.service
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
            configured: inner.settings.configured.unwrap_or_else(|| {
                inner.settings.service != DictationProvider::Local || self.model_ready()
            }),
            key_state: if inner.settings.service == DictationProvider::Local {
                DictationKeyState::NotRequired
            } else {
                DictationKeyState::Locked
            },
            service: inner.settings.service,
            vault_key: vault_key(owner, inner.settings.service),
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
            let mut next = inner.settings.clone();
            match language {
                Some(code) => next.languages.insert(owner.to_string(), code),
                None => next.languages.remove(owner),
            };
            self.save_settings(&next)?;
            inner.settings = next;
        }
        Ok(DictationResult::Status {
            status: self.status(owner),
        })
    }

    fn set_configured(
        &self,
        owner: &str,
        configured: bool,
    ) -> Result<DictationResult, DaemonError> {
        {
            let mut inner = self.lock();
            let mut next = inner.settings.clone();
            next.configured = Some(configured);
            if !configured {
                next.service = DictationProvider::Local;
            }
            self.save_settings(&next)?;
            inner.settings = next;
        }
        Ok(DictationResult::Status {
            status: self.status(owner),
        })
    }

    fn set_service(
        &self,
        owner: &str,
        service: DictationProvider,
    ) -> Result<DictationResult, DaemonError> {
        {
            let mut inner = self.lock();
            let mut next = inner.settings.clone();
            next.service = service;
            next.configured = Some(true);
            self.save_settings(&next)?;
            inner.settings = next;
        }
        Ok(DictationResult::Status {
            status: self.status(owner),
        })
    }

    fn save_settings(&self, settings: &Settings) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec(settings).map_err(|_| {
            refuse(
                "dictation_settings_failed",
                "Cannot encode the dictation settings. Try again.",
                false,
            )
        })?;
        std::fs::create_dir_all(&self.dir)
            .and_then(|()| crate::config::write_private_file(&self.dir.join("settings.json"), &bytes))
            .map_err(|_| refuse("dictation_settings_failed", "Cannot save the dictation settings. Check the kernel's disk space and permissions, then try again.", true))
    }

    #[cfg(test)]
    async fn transcribe(
        self: &Arc<Self>,
        owner: &str,
        upload_id: &str,
        vault: Option<crate::config::UserCredentialVaultConfig>,
    ) -> Result<DictationResult, DaemonError> {
        self.transcribe_with_provider(owner, upload_id, vault, self.selected_service())
            .await
    }

    async fn transcribe_with_provider(
        self: &Arc<Self>,
        owner: &str,
        upload_id: &str,
        vault: Option<crate::config::UserCredentialVaultConfig>,
        provider: DictationProvider,
    ) -> Result<DictationResult, DaemonError> {
        if self.selected_service() != provider {
            return Err(refuse("dictation_service_changed", "The dictation service changed while waiting for the Vault. Record again with the selected service.", true));
        }

        if provider == DictationProvider::Local && !self.model_ready() {
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
        if provider == DictationProvider::Local {
            self.model_use.fetch_add(1, Ordering::SeqCst);
        }
        let service = self.clone();
        let key = vault_key(owner, provider);
        let attempt = tokio::task::spawn_blocking(move || {
            if provider == DictationProvider::Local {
                return service
                    .engine
                    .transcribe(
                        &service.spec.path(&service.dir),
                        &pcm16_to_f32(&pcm),
                        language.as_deref(),
                    )
                    .map_err(|message| refuse("dictation_transcription_failed", &message, true));
            }
            let key = read_key(
                vault.as_ref(),
                key.as_deref().expect("remote service has a Vault handle"),
            )?;
            let text = service
                .services
                .transcribe(provider, &key, &pcm, language.as_deref())?;
            Ok(Transcription {
                text,
                language: language.unwrap_or_default(),
            })
        })
        .await;
        if provider == DictationProvider::Local {
            self.release_model_when_idle();
        }
        let heard = attempt.map_err(|_| {
            refuse(
                "dictation_transcription_failed",
                "Transcription stopped unexpectedly. Try again.",
                true,
            )
        })??;
        Ok(DictationResult::Transcript {
            text: if provider == DictationProvider::Local {
                spoken_text(&heard.text)
            } else {
                heard.text
            },
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

/// Different authenticated users never share a speech credential, even on one kernel.
fn vault_key(owner: &str, provider: DictationProvider) -> Option<String> {
    use sha2::{Digest, Sha256};
    let name = match provider {
        DictationProvider::Local => return None,
        DictationProvider::Openai => "openai",
        DictationProvider::Elevenlabs => "elevenlabs",
    };
    let owner = Sha256::digest(owner.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Some(format!("dictation-{owner}-{name}"))
}

fn read_key(
    vault: Option<&crate::config::UserCredentialVaultConfig>,
    key: &str,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    let missing = || {
        refuse(
            "dictation_no_key",
            "Add your speech service API key in Settings → Dictation, then try again.",
            false,
        )
    };
    let vault = vault.ok_or_else(missing)?;
    let service = crate::secret::RuntimeSecretService::with_vault_config(Vec::new(), vault)
        .map_err(|_| missing())?;
    service
        .vault_secret_for_dictation(key)
        .map_err(|error| {
            if crate::secret::is_chariox_vault_locked_error(&error) {
                refuse(
                    "dictation_vault_locked",
                    "Unlock your kernel's Vault, then try dictation again.",
                    false,
                )
            } else {
                missing()
            }
        })?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(missing)
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
