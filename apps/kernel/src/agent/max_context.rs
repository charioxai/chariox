//! The optional "max context" of an agent: how many tokens its provider lets a
//! conversation grow to before it compacts. Without it the provider's own
//! default applies; the kernel never picks a value.
use crate::error::DaemonError;

/// Claude Code's `CLAUDE_CODE_AUTO_COMPACT_WINDOW` accepts 100,000 to 1,000,000 tokens.
const CLAUDE_RANGE: std::ops::RangeInclusive<u64> = 100_000..=1_000_000;

/// A token count as a person writes it: `200000`, `200k` or `1m`.
pub fn parse_max_context_tokens(text: &str) -> Result<u64, String> {
    let lower = text.trim().to_ascii_lowercase();
    let (digits, factor) = match lower.strip_suffix('k') {
        Some(digits) => (digits, 1_000),
        None => match lower.strip_suffix('m') {
            Some(digits) => (digits, 1_000_000),
            None => (lower.as_str(), 1),
        },
    };
    digits
        .parse::<u64>()
        .ok()
        .and_then(|count| count.checked_mul(factor))
        .filter(|tokens| *tokens > 0)
        .ok_or_else(|| format!("`{text}` is not a token count; write 200000, 200k or 1m"))
}

/// Refuses a value the provider cannot honour, saying what it accepts.
pub fn ensure_max_context_supported(provider: &str, tokens: u64) -> Result<(), DaemonError> {
    let refuse = |message: String| DaemonError::LocalTransport {
        operation: "agent.max_context",
        message,
    };
    match crate::provider::canonical_provider_family(provider) {
        Some("claude") if CLAUDE_RANGE.contains(&tokens) => Ok(()),
        Some("claude") => Err(refuse(format!(
            "Claude compacts between {} and {} tokens; {tokens} is outside that range. Use a value in it or clear the setting.",
            CLAUDE_RANGE.start(),
            CLAUDE_RANGE.end()
        ))),
        Some("codex") => Ok(()),
        _ => Err(refuse(format!(
            "max context is not supported for provider `{provider}` yet; clear the setting."
        ))),
    }
}

/// What a spawn request may carry: a value the provider honours, and only for
/// an agent that runs on this kernel (another kernel does not receive it yet).
pub fn ensure_spawn_max_context(
    provider: &str,
    max_context_tokens: Option<u64>,
    on_another_kernel: bool,
) -> Result<(), DaemonError> {
    let Some(tokens) = max_context_tokens else {
        return Ok(());
    };
    if on_another_kernel {
        return Err(DaemonError::LocalTransport {
            operation: "agent.max_context",
            message: "max context is not supported for an agent that runs on another machine yet; start it without it.".to_string(),
        });
    }
    ensure_max_context_supported(provider, tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_are_read_as_people_write_them() {
        assert_eq!(parse_max_context_tokens("200000"), Ok(200_000));
        assert_eq!(parse_max_context_tokens("200k"), Ok(200_000));
        assert_eq!(parse_max_context_tokens(" 1M "), Ok(1_000_000));
        for bad in ["", "0", "k", "-5", "2.5k", "ten", "99999999999999999999k"] {
            assert!(parse_max_context_tokens(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn each_provider_refuses_what_it_cannot_honour() {
        assert!(ensure_max_context_supported("claude", 200_000).is_ok());
        assert!(ensure_max_context_supported("claude-headless", 1_000_000).is_ok());
        for outside in [99_999, 1_000_001] {
            let error = ensure_max_context_supported("claude", outside).unwrap_err();
            assert!(error.to_string().contains("100000 and 1000000"), "{error}");
        }
        assert!(ensure_max_context_supported("codex", 150_000).is_ok());
        let error = ensure_max_context_supported("opencode", 200_000).unwrap_err();
        assert!(error.to_string().contains("not supported"), "{error}");
    }
}
