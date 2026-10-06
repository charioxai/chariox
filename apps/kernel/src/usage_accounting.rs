//! MP-08 / MP-10 / MP-11: API-equivalent accounting primitives.
//!
//! This module has no wire types or credential access. Provider/runtime/history
//! integration is gated on a coordinator-allocated protocol version. Inputs are
//! official harness counters, never estimates from text length. Input includes
//! cache; output includes reasoning. None means unreported, not zero.

use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: Option<u64>,
    pub cached_input: Option<u64>,
    pub cache_write: Option<u64>,
    pub cache_write_5m: Option<u64>,
    pub cache_write_1h: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
}

/// Parse one Codex cumulative usage breakdown. The runtime must difference
/// thread totals against the previous turn, not sum repeated update events or
/// use `last` (which represents the most recent model request/context).
pub fn codex_usage(value: &Value) -> Option<Usage> {
    let input = value.get("inputTokens")?.as_u64()?;
    let output = value.get("outputTokens")?.as_u64()?;
    let cached_input = counter(value, "cachedInputTokens")?;
    let reasoning = counter(value, "reasoningOutputTokens")?;
    if cached_input.is_some_and(|n| n > input) || reasoning.is_some_and(|n| n > output) {
        return None;
    }
    Some(Usage {
        input: Some(input),
        cached_input,
        output: Some(output),
        reasoning,
        ..Usage::default()
    })
}

/// Claude result usage excludes cached reads/writes from `input_tokens`.
/// Result counters aggregate a CLI turn; do not also add streamed message usage.
pub fn claude_usage(value: &Value) -> Option<Usage> {
    let base = value.get("input_tokens")?.as_u64()?;
    let output = value.get("output_tokens")?.as_u64()?;
    let cached = counter(value, "cache_read_input_tokens")?;
    let write = counter(value, "cache_creation_input_tokens")?;
    let input = match (cached, write) {
        (Some(cached), Some(write)) => Some(base.checked_add(cached)?.checked_add(write)?),
        _ => None,
    };
    let creation = value.get("cache_creation").unwrap_or(&Value::Null);
    let five = counter(creation, "ephemeral_5m_input_tokens")?;
    let hour = counter(creation, "ephemeral_1h_input_tokens")?;
    if let (Some(write), Some(five), Some(hour)) = (write, five, hour) {
        if five.checked_add(hour)? != write {
            return None;
        }
    }
    Some(Usage {
        input,
        cached_input: cached,
        cache_write: write,
        cache_write_5m: five,
        cache_write_1h: hour,
        output: Some(output),
        reasoning: None,
    })
}

/// OpenCode assistant message counters exclude cache and reasoning from their
/// input/output fields. Deduplicate by message ID before aggregating a turn.
pub fn opencode_usage(value: &Value) -> Option<Usage> {
    let base = value.get("input")?.as_u64()?;
    let visible_output = value.get("output")?.as_u64()?;
    let reasoning = counter(value, "reasoning")?;
    let cache = value.get("cache").unwrap_or(&Value::Null);
    let cached = counter(cache, "read")?;
    let write = counter(cache, "write")?;
    let input = match (cached, write) {
        (Some(cached), Some(write)) => Some(base.checked_add(cached)?.checked_add(write)?),
        _ => None,
    };
    Some(Usage {
        input,
        cached_input: cached,
        cache_write: write,
        output: match reasoning {
            Some(n) => Some(visible_output.checked_add(n)?),
            None => None,
        },
        reasoning,
        ..Usage::default()
    })
}

fn counter(value: &Value, key: &str) -> Option<Option<u64>> {
    match value.get(key) {
        None => Some(None),
        Some(value) => value.as_u64().map(Some),
    }
}

#[derive(Deserialize)]
struct PriceTable {
    models: Vec<Price>,
}

#[derive(Deserialize)]
struct Price {
    model: String,
    band: String,
    input: u64,
    cached_input: u64,
    cache_write: Option<u64>,
    cache_write_5m: Option<u64>,
    cache_write_1h: Option<u64>,
    output: u64,
}

pub const PRICE_TABLE_VERSION: u32 = 1;
pub const PRICE_TABLE_DATE: &str = "2026-10-06";
pub const PRICE_TABLE_JSON: &str = include_str!("usage_accounting/prices-2026-10-06.json");

/// Exact integer nanodollars at standard global API list prices. Unknown model,
/// cache-write TTL, context band or necessary counter returns None. This is not
/// the amount billed by a provider subscription. Reasoning is already in output.
pub fn quote(model: &str, band: Option<&str>, usage: &Usage) -> Option<u128> {
    static TABLE: OnceLock<PriceTable> = OnceLock::new();
    let table = TABLE.get_or_init(|| serde_json::from_str(PRICE_TABLE_JSON).expect("price table"));
    let price = table.models.iter().find(|price| {
        price.model == model && (price.band == "all" || band == Some(price.band.as_str()))
    })?;
    let input = usage.input?;
    let cached = usage.cached_input?;
    let output = usage.output?;
    let write_rate = price.cache_write.or(price.cache_write_5m);
    let write = match (usage.cache_write, write_rate) {
        (Some(n), _) => n,
        (None, None) => 0, // This model has no cache-write price category.
        (None, Some(_)) => return None,
    };
    if usage.reasoning.is_some_and(|n| n > output) {
        return None;
    }
    let uncached = input.checked_sub(cached)?.checked_sub(write)?;
    let write_cost = if write == 0 {
        0
    } else if let Some(rate) = price.cache_write {
        u128::from(write) * u128::from(rate)
    } else {
        let five = usage.cache_write_5m?;
        let hour = usage.cache_write_1h?;
        if five.checked_add(hour)? != write {
            return None;
        }
        u128::from(five) * u128::from(price.cache_write_5m?)
            + if hour == 0 {
                0
            } else {
                u128::from(hour) * u128::from(price.cache_write_1h?)
            }
    };
    Some(
        u128::from(uncached) * u128::from(price.input)
            + u128::from(cached) * u128::from(price.cached_input)
            + u128::from(output) * u128::from(price.output)
            + write_cost,
    )
}

#[cfg(test)]
#[path = "usage_accounting/tests.rs"]
mod tests;
