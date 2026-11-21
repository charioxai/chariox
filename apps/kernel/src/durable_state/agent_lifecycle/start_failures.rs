//! A delivery whose agent cannot start is retried with growing pauses and says why. Only when
//! the attempts are spent does the sweep end it, naming the last start error.
use super::*;

/// Starts of one delivery before it ends: with the pauses below, about 15 minutes.
pub(crate) const START_ATTEMPTS: u64 = 6;
const FIRST_RETRY_MS: u64 = 30_000;
const LONGEST_RETRY_MS: u64 = 8 * 60_000;
const ERROR_CHARS: usize = 240;

/// What one failed start left behind.
pub(crate) struct StartFailure {
    pub(crate) attempts: u64,
    pub(crate) error: String,
    pub(crate) retry_at_ms: u64,
}

/// The pause after the `attempts`-th failed start.
pub(crate) fn retry_pause_ms(attempts: u64) -> u64 {
    FIRST_RETRY_MS
        .saturating_mul(1 << attempts.saturating_sub(1).min(10))
        .min(LONGEST_RETRY_MS)
}

/// The start error as the person reads it. A provider usage limit is named as one, with the time
/// it lifts when the provider gave it.
pub(crate) fn describe_start_error(error: &str, now: u64) -> String {
    let text = crate::provider::sanitize_provider_diagnostic(error);
    let text = text.trim().trim_end_matches('.');
    let text: String = text.chars().take(ERROR_CHARS).collect();
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("usage allowance")
        || lowered.contains("usage limit")
        || lowered.contains("credits are exhausted")
    {
        let reset = crate::provider::usage_limit_reset_at_ms(&text, now)
            .and_then(|at| chrono::DateTime::from_timestamp_millis(i64::try_from(at).ok()?));
        return match reset {
            Some(at) => format!(
                "provider usage limit, resets {} ({text})",
                at.format("%b %-d, %H:%M UTC")
            ),
            None => format!("provider usage limit ({text})"),
        };
    }
    text
}

/// Records one more failed start of the delivery and when to try again. A start that waits on a
/// person spends no attempt and is tried again after the first pause.
pub(super) fn record(
    tx: &Transaction<'_>,
    sequence: u64,
    error: &str,
    waits_on_owner: bool,
    now: u64,
) -> Result<StartFailure, DaemonError> {
    let kept = get(tx, sequence)?.map_or(0, |f| f.attempts);
    let attempts = if waits_on_owner { kept } else { kept + 1 };
    let failure = StartFailure {
        attempts,
        error: describe_start_error(error, now),
        retry_at_ms: now.saturating_add(if waits_on_owner {
            FIRST_RETRY_MS
        } else {
            retry_pause_ms(attempts)
        }),
    };
    tx.execute(
        "INSERT INTO agent_inbox_start_failures VALUES(?1,?2,?3,?4)
         ON CONFLICT(sequence) DO UPDATE SET attempts=?2,last_error=?3,retry_at_ms=?4",
        params![
            sql_integer(sequence)?,
            sql_integer(failure.attempts)?,
            failure.error,
            sql_integer(failure.retry_at_ms)?
        ],
    )
    .map_err(sql)?;
    Ok(failure)
}

pub(super) fn get(
    tx: &Transaction<'_>,
    sequence: u64,
) -> Result<Option<StartFailure>, DaemonError> {
    tx.query_row(
        "SELECT attempts,last_error,retry_at_ms FROM agent_inbox_start_failures WHERE sequence=?1",
        [sql_integer(sequence)?],
        |r| {
            Ok(StartFailure {
                attempts: r.get::<_, i64>(0)?.max(0) as u64,
                error: r.get(1)?,
                retry_at_ms: r.get::<_, i64>(2)?.max(0) as u64,
            })
        },
    )
    .optional()
    .map_err(sql)
}

pub(super) fn clear(tx: &Transaction<'_>, sequence: u64) -> Result<(), DaemonError> {
    tx.execute(
        "DELETE FROM agent_inbox_start_failures WHERE sequence=?1",
        [sql_integer(sequence)?],
    )
    .map_err(sql)?;
    Ok(())
}

/// The notice the agent's pane shows after a failed start.
pub(crate) fn retry_notice(failure: &StartFailure) -> String {
    if failure.attempts >= START_ATTEMPTS {
        return format!(
            "The agent could not start (attempt {} of {START_ATTEMPTS}): {}. No more retries.",
            failure.attempts, failure.error
        );
    }
    format!(
        "The agent could not start (attempt {} of {START_ATTEMPTS}): {}. Next retry in {} s.",
        failure.attempts,
        failure.error,
        retry_pause_ms(failure.attempts) / 1000
    )
}
