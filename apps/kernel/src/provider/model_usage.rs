use serde::{Deserialize, Serialize};

/// Cumulative tokens for one prompt, run and provider-reported model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTurnUsage {
    pub model: String,
    pub tokens: ModelTokenCounts,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTokenCounts {
    /// Includes input cache reads and writes.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
}
