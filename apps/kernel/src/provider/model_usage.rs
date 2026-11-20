use serde::{Deserialize, Serialize};

/// Tokens attributed to a provider-reported model.
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

impl ModelTokenCounts {
    pub(crate) fn checked_delta_from(self, earlier: Self) -> Option<Self> {
        let tokens = Self {
            input_tokens: self.input_tokens.checked_sub(earlier.input_tokens)?,
            output_tokens: self.output_tokens.checked_sub(earlier.output_tokens)?,
            cached_input_tokens: self
                .cached_input_tokens
                .checked_sub(earlier.cached_input_tokens)?,
            cache_write_input_tokens: self
                .cache_write_input_tokens
                .checked_sub(earlier.cache_write_input_tokens)?,
        };
        tokens
            .input_tokens
            .checked_sub(tokens.cached_input_tokens)?
            .checked_sub(tokens.cache_write_input_tokens)?;
        Some(tokens)
    }
}
