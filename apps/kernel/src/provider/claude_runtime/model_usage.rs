use serde_json::Value;

use crate::provider::{ModelTokenCounts, ModelTurnUsage};

/// Claude reports session totals here. The usage store converts these to
/// prompt deltas using the last result for the same native session.
pub(super) fn result_model_totals(result: &Value) -> Vec<ModelTurnUsage> {
    let Some(models) = result.get("modelUsage").and_then(Value::as_object) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|(reported_model, usage)| {
            let model = usage
                .get("canonicalModel")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(reported_model);
            let model = model.strip_prefix("claude/").unwrap_or(model);
            let uncached_input = usage.get("inputTokens")?.as_u64()?;
            let cached_input_tokens = usage.get("cacheReadInputTokens")?.as_u64()?;
            let cache_write_input_tokens = usage.get("cacheCreationInputTokens")?.as_u64()?;
            let input_tokens = uncached_input
                .checked_add(cached_input_tokens)?
                .checked_add(cache_write_input_tokens)?;
            Some(ModelTurnUsage {
                model: model.to_string(),
                tokens: ModelTokenCounts {
                    input_tokens,
                    output_tokens: usage.get("outputTokens")?.as_u64()?,
                    cached_input_tokens,
                    cache_write_input_tokens,
                },
            })
        })
        .collect()
}
