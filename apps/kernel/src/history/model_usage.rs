use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::Deserialize;
use std::sync::OnceLock;

use crate::error::DaemonError;
use crate::local::AgentModelUsage;
use crate::provider::{ModelTokenCounts, ModelTurnUsage};

use super::{operational_history_error, OperationalHistoryStore};

pub(super) fn migrate(connection: &Connection) -> Result<(), DaemonError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS history_model_usage (
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                prompt_id TEXT NOT NULL,
                run_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_input_tokens INTEGER NOT NULL,
                PRIMARY KEY (session_id, agent_id, prompt_id, run_id, model)
            ) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS history_claude_model_totals (
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                provider_session_id TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_input_tokens INTEGER NOT NULL,
                PRIMARY KEY (session_id, agent_id, provider_session_id, model)
            ) WITHOUT ROWID;",
        )
        .map_err(|error| operational_history_error("create model usage table", error))
}

fn read_tokens(row: &Row<'_>, first_column: usize) -> rusqlite::Result<ModelTokenCounts> {
    let count = |index| {
        let value: i64 = row.get(index)?;
        u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
    };
    Ok(ModelTokenCounts {
        input_tokens: count(first_column)?,
        output_tokens: count(first_column + 1)?,
        cached_input_tokens: count(first_column + 2)?,
        cache_write_input_tokens: count(first_column + 3)?,
    })
}

/// A result's scope before it is attributed to a prompt.
enum ModelUsageReport<'a> {
    Prompt {
        provider: &'a str,
        usage: &'a [ModelTurnUsage],
    },
    ClaudeSession {
        id: &'a str,
        totals: &'a [ModelTurnUsage],
    },
}

fn token_count_for_storage(value: u64) -> Result<i64, rusqlite::Error> {
    i64::try_from(value).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn claude_prompt_delta(
    transaction: &Transaction<'_>,
    room_and_agent: (&str, &str),
    provider_session_id: &str,
    model: &str,
    total: ModelTokenCounts,
) -> rusqlite::Result<ModelTokenCounts> {
    let (room, agent) = room_and_agent;
    let previous = transaction
        .query_row(
            "SELECT input_tokens, output_tokens, cached_input_tokens, cache_write_input_tokens
             FROM history_claude_model_totals
             WHERE session_id = ?1 AND agent_id = ?2 AND provider_session_id = ?3 AND model = ?4",
            params![room, agent, provider_session_id, model],
            |row| read_tokens(row, 0),
        )
        .optional()?;
    transaction.execute(
        "INSERT INTO history_claude_model_totals VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT (session_id, agent_id, provider_session_id, model) DO UPDATE SET
             input_tokens = excluded.input_tokens, output_tokens = excluded.output_tokens,
             cached_input_tokens = excluded.cached_input_tokens,
             cache_write_input_tokens = excluded.cache_write_input_tokens",
        params![
            room,
            agent,
            provider_session_id,
            model,
            token_count_for_storage(total.input_tokens)?,
            token_count_for_storage(total.output_tokens)?,
            token_count_for_storage(total.cached_input_tokens)?,
            token_count_for_storage(total.cache_write_input_tokens)?,
        ],
    )?;
    // Claude can reset a model's counters within the same native session.
    Ok(previous
        .and_then(|earlier| total.checked_delta_from(earlier))
        .unwrap_or(total))
}

impl OperationalHistoryStore {
    pub(crate) fn save_model_usage(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        run_id: &str,
        provider: &str,
        usage: &[ModelTurnUsage],
    ) -> Result<(), DaemonError> {
        self.save_model_usage_report(
            session_id,
            agent_id,
            prompt_id,
            run_id,
            ModelUsageReport::Prompt { provider, usage },
        )
    }

    pub(crate) fn save_claude_model_usage(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        run_id: &str,
        provider_session_id: &str,
        totals: &[ModelTurnUsage],
    ) -> Result<(), DaemonError> {
        self.save_model_usage_report(
            session_id,
            agent_id,
            prompt_id,
            run_id,
            ModelUsageReport::ClaudeSession {
                id: provider_session_id,
                totals,
            },
        )
    }

    fn save_model_usage_report(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        run_id: &str,
        report: ModelUsageReport<'_>,
    ) -> Result<(), DaemonError> {
        let (provider, usage, native_session) = match report {
            ModelUsageReport::Prompt { provider, usage } => (provider, usage, None),
            ModelUsageReport::ClaudeSession { id, totals } => ("claude", totals, Some(id)),
        };
        if usage.is_empty() {
            return Ok(());
        }
        let mut connection =
            self.connection
                .lock()
                .map_err(|error| DaemonError::SessionHistoryFailed {
                    session_id: Some(session_id.to_string()),
                    operation: "lock model usage store",
                    message: error.to_string(),
                })?;
        let transaction = connection
            .transaction()
            .map_err(|error| operational_history_error("begin model usage write", error))?;
        let count = |value| {
            token_count_for_storage(value)
                .map_err(|error| operational_history_error("record model usage counters", error))
        };
        for ModelTurnUsage { model, tokens } in usage {
            let tokens = if let Some(native_session) = native_session {
                // The prompt row (including a zero delta) is the receipt. A replay
                // must leave the baseline intact even after newer prompts finish.
                let recorded: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM history_model_usage
                     WHERE session_id = ?1 AND agent_id = ?2 AND prompt_id = ?3 AND run_id = ?4 AND model = ?5)",
                    params![session_id, agent_id, prompt_id, run_id, model],
                    |row| row.get(0),
                ).map_err(|error| operational_history_error("read Claude usage receipt", error))?;
                if recorded {
                    continue;
                }
                claude_prompt_delta(
                    &transaction,
                    (session_id, agent_id),
                    native_session,
                    model,
                    *tokens,
                )
                .map_err(|error| operational_history_error("record Claude session totals", error))?
            } else {
                *tokens
            };
            // Poll retries and leased replays may resend an earlier observation.
            transaction
                .execute(
                    "INSERT INTO history_model_usage VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                     ON CONFLICT (session_id, agent_id, prompt_id, run_id, model) DO UPDATE SET
                         input_tokens = excluded.input_tokens,
                         output_tokens = excluded.output_tokens,
                         cached_input_tokens = excluded.cached_input_tokens,
                         cache_write_input_tokens = excluded.cache_write_input_tokens
                     WHERE excluded.input_tokens >= input_tokens
                       AND excluded.output_tokens >= output_tokens
                       AND excluded.cached_input_tokens >= cached_input_tokens
                       AND excluded.cache_write_input_tokens >= cache_write_input_tokens
                       AND (excluded.input_tokens > input_tokens
                         OR excluded.output_tokens > output_tokens
                         OR excluded.cached_input_tokens > cached_input_tokens
                         OR excluded.cache_write_input_tokens > cache_write_input_tokens)",
                    params![
                        session_id,
                        agent_id,
                        prompt_id,
                        run_id,
                        provider,
                        model,
                        count(tokens.input_tokens)?,
                        count(tokens.output_tokens)?,
                        count(tokens.cached_input_tokens)?,
                        count(tokens.cache_write_input_tokens)?
                    ],
                )
                .map_err(|error| operational_history_error("record model usage", error))?;
        }
        transaction
            .commit()
            .map_err(|error| operational_history_error("commit model usage", error))
    }

    pub(crate) fn turn_model_usage(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        run_id: &str,
    ) -> Result<Vec<ModelTurnUsage>, DaemonError> {
        let connection = self.lock_read_connection(Some(session_id))?;
        let result = (|| {
            let mut query = connection.prepare_cached(
                "SELECT model, input_tokens, output_tokens, cached_input_tokens, cache_write_input_tokens
                 FROM history_model_usage
                 WHERE session_id = ?1 AND agent_id = ?2 AND prompt_id = ?3 AND run_id = ?4
                 ORDER BY model",
            )?;
            let rows =
                query.query_map(params![session_id, agent_id, prompt_id, run_id], |row| {
                    Ok(ModelTurnUsage {
                        model: row.get(0)?,
                        tokens: read_tokens(row, 1)?,
                    })
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })();
        result.map_err(|error| operational_history_error("read prompt model usage", error))
    }

    pub(crate) fn agent_model_usage(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<AgentModelUsage>, DaemonError> {
        let connection = self.lock_read_connection(Some(session_id))?;
        let result = (|| {
            let mut query = connection.prepare_cached(
                "SELECT provider, model, SUM(input_tokens), SUM(output_tokens),
                        SUM(cached_input_tokens), SUM(cache_write_input_tokens)
                 FROM history_model_usage WHERE session_id = ?1 AND agent_id = ?2
                 GROUP BY provider, model ORDER BY provider, model",
            )?;
            let rows = query.query_map(params![session_id, agent_id], |row| {
                let provider: String = row.get(0)?;
                let model: String = row.get(1)?;
                let tokens = read_tokens(row, 2)?;
                Ok(AgentModelUsage {
                    api_equivalent_nanodollars: model_cost(&provider, &model, tokens)
                        .map(|amount| amount.to_string()),
                    provider,
                    model,
                    input_tokens: tokens.input_tokens,
                    output_tokens: tokens.output_tokens,
                    cached_input_tokens: tokens.cached_input_tokens,
                    cache_write_input_tokens: tokens.cache_write_input_tokens,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })();
        result.map_err(|error| operational_history_error("read agent model usage", error))
    }
}

#[derive(Deserialize)]
struct ApiEquivalentTokenRate {
    provider: String,
    model: String,
    input: u64,
    output: u64,
    cached_input: u64,
    cache_write: Option<u64>,
}

fn model_cost(provider: &str, model: &str, tokens: ModelTokenCounts) -> Option<u128> {
    static TOKEN_RATES: OnceLock<Vec<ApiEquivalentTokenRate>> = OnceLock::new();
    let rates = TOKEN_RATES.get_or_init(|| {
        serde_json::from_str(include_str!("api_equivalent_token_rates.json"))
            .expect("API-equivalent token rate table must be valid")
    });
    let rate = rates
        .iter()
        .find(|rate| rate.provider == provider && rate.model == model)?;
    let uncached = tokens
        .input_tokens
        .checked_sub(tokens.cached_input_tokens)?
        .checked_sub(tokens.cache_write_input_tokens)?;
    let write_cost = if tokens.cache_write_input_tokens == 0 {
        0
    } else {
        u128::from(tokens.cache_write_input_tokens) * u128::from(rate.cache_write?)
    };
    Some(
        u128::from(uncached) * u128::from(rate.input)
            + u128::from(tokens.output_tokens) * u128::from(rate.output)
            + u128::from(tokens.cached_input_tokens) * u128::from(rate.cached_input)
            + write_cost,
    )
}
