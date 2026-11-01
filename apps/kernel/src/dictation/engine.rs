//! whisper.cpp behind a small trait so the service can be tested without a model.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub(crate) struct Transcription {
    pub text: String,
    pub language: String,
}

pub(crate) trait SpeechEngine: Send + Sync {
    fn transcribe(
        &self,
        model: &Path,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<Transcription, String>;
    /// Frees the loaded model; the caller decides when the engine has been idle long enough.
    fn release(&self);
}

struct Loaded {
    model: PathBuf,
    context: Arc<WhisperContext>,
}

#[derive(Default)]
pub(crate) struct WhisperEngine {
    loaded: Mutex<Option<Loaded>>,
}

impl WhisperEngine {
    fn context(&self, model: &Path) -> Result<Arc<WhisperContext>, String> {
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = loaded.as_ref().filter(|current| current.model == model) {
            return Ok(current.context.clone());
        }
        let path = model.to_str().ok_or("the model path is not valid UTF-8")?;
        let context = Arc::new(
            WhisperContext::new_with_params(path, WhisperContextParameters::default())
                .map_err(|error| format!("cannot load the speech model: {error}"))?,
        );
        *loaded = Some(Loaded {
            model: model.to_path_buf(),
            context: context.clone(),
        });
        Ok(context)
    }
}

impl SpeechEngine for WhisperEngine {
    fn transcribe(
        &self,
        model: &Path,
        samples: &[f32],
        language: Option<&str>,
    ) -> Result<Transcription, String> {
        let context = self.context(model)?;
        let mut state = context
            .create_state()
            .map_err(|error| format!("cannot start the speech model: {error}"))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language);
        params.set_n_threads(
            std::thread::available_parallelism().map_or(2, |count| count.get().min(4)) as i32,
        );
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        state
            .full(params, samples)
            .map_err(|error| format!("transcription failed: {error}"))?;
        let mut text = String::new();
        for index in 0..state.full_n_segments() {
            if let Some(segment) = state.get_segment(index) {
                text.push_str(
                    &segment
                        .to_str_lossy()
                        .map_err(|error| format!("transcription failed: {error}"))?,
                );
            }
        }
        let detected = whisper_rs::get_lang_str(state.full_lang_id_from_state()).unwrap_or("en");
        Ok(Transcription {
            text,
            language: language.unwrap_or(detected).to_string(),
        })
    }

    fn release(&self) {
        *self
            .loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

pub(crate) fn is_language_code(code: &str) -> bool {
    whisper_rs::get_lang_id(code).is_some()
}
