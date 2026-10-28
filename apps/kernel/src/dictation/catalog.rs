//! The one speech model the kernel downloads, pinned by size and SHA-256.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelSpec {
    pub id: String,
    pub file_name: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

impl ModelSpec {
    /// whisper.cpp `base` multilingual, 5-bit quantized: about 57 MiB, clearly better
    /// than `tiny` on Spanish at roughly twice the time (see docs/DICTATION.md).
    /// The URL names an immutable revision of the upstream model repository.
    pub(crate) fn pinned() -> Self {
        Self {
            id: "whisper-base-q5_1".to_string(),
            file_name: "ggml-base-q5_1.bin".to_string(),
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base-q5_1.bin".to_string(),
            sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898".to_string(),
            size_bytes: 59_707_625,
        }
    }

    pub(crate) fn path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.file_name)
    }

    pub(crate) fn partial_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.part", self.file_name))
    }
}
