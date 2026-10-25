//! The kernel sends a bounded WAV upload to the selected speech service.
//! Provider error bodies and transport errors are never returned: they can echo credentials.
use std::io::Read;
use std::time::Duration;

use super::{refuse, DaemonError};
use crate::local::{DictationProvider, DICTATION_SAMPLE_RATE_HZ};

pub(super) struct SpeechServices {
    openai: String,
    elevenlabs: String,
}

// The ignored live drill runs the real daemon against a loopback speech server.
// Ordinary kernel binaries have no endpoint override.
#[cfg(test)]
pub(super) static LIVE_DRILL_ENDPOINT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

impl Default for SpeechServices {
    fn default() -> Self {
        #[cfg(test)]
        if let Some(url) = LIVE_DRILL_ENDPOINT.get() {
            return Self::for_test(url.clone());
        }
        Self {
            openai: "https://api.openai.com/v1/audio/transcriptions".into(),
            elevenlabs: "https://api.elevenlabs.io/v1/speech-to-text".into(),
        }
    }
}

impl SpeechServices {
    pub(super) fn transcribe(
        &self,
        provider: DictationProvider,
        key: &str,
        pcm: &[u8],
        language: Option<&str>,
    ) -> Result<String, DaemonError> {
        let key = key.trim();
        if !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(refuse("dictation_bad_key", "The speech service API key contains invalid characters. Replace it in Settings → Dictation.", false));
        }
        let (url, model_field, model) = match provider {
            DictationProvider::Openai => (&self.openai, "model", "gpt-transcribe"),
            DictationProvider::Elevenlabs => (&self.elevenlabs, "model_id", "scribe_v2"),
            DictationProvider::Local => unreachable!("local speech uses whisper.cpp"),
        };
        let boundary = format!("chariox-{:032x}", rand::random::<u128>());
        let mut body = Vec::with_capacity(pcm.len() + 1024);
        field(&mut body, &boundary, model_field, model);
        if let Some(language) = language {
            field(
                &mut body,
                &boundary,
                if provider == DictationProvider::Openai {
                    "language"
                } else {
                    "language_code"
                },
                language,
            );
        }
        if provider == DictationProvider::Elevenlabs {
            field(&mut body, &boundary, "tag_audio_events", "false");
            field(&mut body, &boundary, "timestamps_granularity", "none");
        }
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"dictation.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(&wav(pcm));
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(100))
            // Never forward an API key to a redirect destination.
            .redirects(0)
            .build();
        let request = agent.post(url).set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        );
        let request = match provider {
            DictationProvider::Openai => request.set("Authorization", &format!("Bearer {key}")),
            DictationProvider::Elevenlabs => request.set("xi-api-key", key),
            DictationProvider::Local => unreachable!(),
        };
        let response = match request.send_bytes(&body) {
            Ok(response) if response.status() == 200 => response,
            Ok(_) => return Err(failed()),
            Err(ureq::Error::Status(status, response)) => {
                let mut bytes = Vec::new();
                let _ = response.into_reader().take(65_536).read_to_end(&mut bytes);
                let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
                let code = json
                    .pointer("/detail/status")
                    .or_else(|| json.pointer("/error/code"))
                    .and_then(|code| code.as_str())
                    .unwrap_or("");
                return Err(
                    if status == 429
                        || status == 402
                        || matches!(
                            code,
                            "quota_exceeded" | "insufficient_quota" | "payment_required"
                        )
                    {
                        refuse("dictation_quota", "The speech service quota or rate limit was reached. Check your account's billing and limits, then try again.", true)
                    } else if status == 401 || status == 403 {
                        refuse("dictation_bad_key", "The speech service rejected the API key. Replace it in Settings → Dictation.", false)
                    } else if status >= 500 {
                        network()
                    } else {
                        failed()
                    },
                );
            }
            Err(ureq::Error::Transport(_)) => return Err(network()),
        };
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|_| network())?;
        if bytes.len() > 1_048_576 {
            return Err(failed());
        }
        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| failed())?;
        json.get("text")
            .and_then(|text| text.as_str())
            .map(|text| text.trim().to_owned())
            .ok_or_else(failed)
    }

    #[cfg(test)]
    pub(super) fn for_test(url: String) -> Self {
        Self {
            openai: url.clone(),
            elevenlabs: url,
        }
    }
}

fn field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(pcm.len() as u32 + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&DICTATION_SAMPLE_RATE_HZ.to_le_bytes());
    wav.extend_from_slice(&(DICTATION_SAMPLE_RATE_HZ * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

fn network() -> DaemonError {
    refuse("dictation_network", "The kernel could not reach the speech service. Check its network connection and try again.", true)
}
fn failed() -> DaemonError {
    refuse(
        "dictation_transcription_failed",
        "The speech service could not transcribe that recording. Try again.",
        true,
    )
}
