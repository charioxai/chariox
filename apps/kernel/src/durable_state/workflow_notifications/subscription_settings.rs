//! What a subscription keeps on its subscriber's kernel beside the shared row: the option values
//! it picked, whether it ends after one delivery, the person's pause, its counters and the trace
//! of what each occurrence did. The gate runs where the occurrence is accepted, so an occurrence
//! that does not match wakes no one and does not use up `once`. Pause is checked again where the
//! prompt is queued: an occurrence accepted before the pause waits there until resume or expiry.
use super::*;
use crate::local::{
    NotificationOptionValue, NotificationSourceOption, NotificationTraceLine,
    NotificationTraceOutcome, NotificationTraceTurn,
};
use serde_json::{Map, Value};

/// Trace lines one subscription keeps; older ones are dropped.
const TRACE_LINES: i64 = 50;
/// Longest occurrence title a trace line keeps.
const TITLE_CHARS: usize = 120;

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS notification_subscription_settings (
      subscription_id TEXT PRIMARY KEY, options_json TEXT NOT NULL DEFAULT '{}',
      once INTEGER NOT NULL DEFAULT 0, paused INTEGER NOT NULL DEFAULT 0,
      fired INTEGER NOT NULL DEFAULT 0, last_fired_at_ms INTEGER,
      loop_stopped_at_ms INTEGER);
    CREATE TABLE IF NOT EXISTS notification_trace (
      subscription_id TEXT NOT NULL, occurrence_id TEXT NOT NULL, at_ms INTEGER NOT NULL,
      outcome TEXT NOT NULL, title TEXT NOT NULL, detail TEXT, turn_agent_id TEXT,
      turn_prompt_id TEXT,
      PRIMARY KEY(subscription_id, occurrence_id));",
    )
}

/// The picked values, checked against what the source declares. Each refusal lists the
/// declared options so the caller can pick again.
pub(crate) fn picked_options(
    declared: &[NotificationSourceOption],
    picked: &[NotificationOptionValue],
) -> Result<Map<String, Value>, DaemonError> {
    let offered = || {
        if declared.is_empty() {
            return "this source declares no options".to_owned();
        }
        let listed = declared
            .iter()
            .map(|option| {
                format!(
                    "{} ({}, {})",
                    option.name,
                    option.label,
                    kind_word(option.kind)
                )
            })
            .collect::<Vec<_>>();
        format!("this source declares {}", listed.join(", "))
    };
    let mut values = Map::new();
    for pick in picked {
        let Some(option) = declared.iter().find(|option| option.name == pick.name) else {
            return Err(error(format!(
                "option {} is not declared; {}",
                pick.name,
                offered()
            )));
        };
        if !option.kind.admits(&pick.value) {
            return Err(error(format!(
                "option {} takes a {} value, not {}; {}",
                pick.name,
                kind_word(option.kind),
                pick.value,
                offered()
            )));
        }
        if values
            .insert(pick.name.clone(), pick.value.clone())
            .is_some()
        {
            return Err(error(format!(
                "option {} is picked twice; {}",
                pick.name,
                offered()
            )));
        }
    }
    Ok(values)
}

fn kind_word(kind: chariox_app_package::EventOptionType) -> &'static str {
    match kind {
        chariox_app_package::EventOptionType::Number => "number",
        chariox_app_package::EventOptionType::Text => "text",
        chariox_app_package::EventOptionType::Boolean => "boolean",
    }
}

pub(super) fn save_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    options: &Map<String, Value>,
    once: bool,
) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_subscription_settings (subscription_id, options_json, once)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(subscription_id) DO UPDATE SET options_json=excluded.options_json, once=excluded.once",
        params![subscription_id, encode(options)?, once],
    )
    .map_err(sql)?;
    Ok(())
}

/// Carries a subscription's settings, counters and trace over to the one that replaces it.
pub(super) fn carry_in(tx: &Transaction<'_>, from: &str, to: &str) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT OR REPLACE INTO notification_subscription_settings
         SELECT ?2, options_json, once, paused, fired, last_fired_at_ms, loop_stopped_at_ms
         FROM notification_subscription_settings WHERE subscription_id=?1",
        params![from, to],
    )
    .map_err(sql)?;
    tx.execute(
        "UPDATE notification_trace SET subscription_id=?2 WHERE subscription_id=?1",
        params![from, to],
    )
    .map_err(sql)?;
    tx.execute(
        "DELETE FROM notification_subscription_settings WHERE subscription_id=?1",
        [from],
    )
    .map_err(sql)?;
    Ok(())
}

/// Retired lines kept in all, for the subscriptions their owners removed and may make again.
const RETIRED_LINES: i64 = 500;

/// Where a removed subscription's lines wait for the one its owner makes again: the same
/// source and target, whatever the new subscription's id.
fn retired_key(sub: &WorkflowNotificationSubscription) -> String {
    format!(
        "retired:{}:{}:{:?}:{}:{}",
        sub.owner_user_id, sub.source_id, sub.target_kind, sub.endpoint_id, sub.workflow_id
    )
}

/// A removed subscription forgets its settings and counters; its trace waits under its source
/// and target for a new subscription to the same pair.
pub(super) fn retire_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
) -> Result<(), DaemonError> {
    let key = retired_key(sub);
    tx.execute(
        "UPDATE OR REPLACE notification_trace SET subscription_id=?2 WHERE subscription_id=?1",
        params![sub.subscription_id, key],
    )
    .map_err(sql)?;
    tx.execute(
        "DELETE FROM notification_subscription_settings WHERE subscription_id=?1",
        [&sub.subscription_id],
    )
    .map_err(sql)?;
    prune_in(tx, &key)?;
    tx.execute(
        "DELETE FROM notification_trace WHERE subscription_id LIKE 'retired:%' AND rowid NOT IN
         (SELECT rowid FROM notification_trace WHERE subscription_id LIKE 'retired:%'
          ORDER BY at_ms DESC, rowid DESC LIMIT ?1)",
        [RETIRED_LINES],
    )
    .map_err(sql)?;
    Ok(())
}

/// A subscription made again for a source and target that had one gets that one's lines back.
pub(super) fn restore_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
) -> Result<(), DaemonError> {
    tx.execute(
        "UPDATE OR REPLACE notification_trace SET subscription_id=?1 WHERE subscription_id=?2",
        params![sub.subscription_id, retired_key(sub)],
    )
    .map_err(sql)?;
    prune_in(tx, &sub.subscription_id)
}

pub(super) fn set_paused_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    paused: bool,
) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_subscription_settings (subscription_id, paused) VALUES (?1, ?2)
         ON CONFLICT(subscription_id) DO UPDATE SET paused=excluded.paused",
        params![subscription_id, paused],
    )
    .map_err(sql)?;
    Ok(())
}

/// What the gate decided for one new occurrence.
pub(super) enum Gate {
    Deliver,
    /// Recorded in the trace and settled without waking anyone.
    Skip,
}

/// Paused, options the source no longer declares, or a picked value the occurrence does not
/// carry: the occurrence is traced and skipped.
pub(super) fn gate_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
    now: u64,
) -> Result<Gate, DaemonError> {
    let Some((outcome, detail)) = verdict(tx, sub, env)? else {
        return Ok(Gate::Deliver);
    };
    trace_in(tx, &sub.subscription_id, env, outcome, detail, now)?;
    Ok(Gate::Skip)
}

/// Why the subscription would not deliver this occurrence, if it would not. Reads only, so a
/// dry run asks the same question delivery does.
fn verdict(
    db: &Connection,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
) -> Result<Option<(NotificationTraceOutcome, Option<String>)>, DaemonError> {
    let settings: Option<(String, bool)> = db
        .query_row(
            "SELECT options_json, paused FROM notification_subscription_settings WHERE subscription_id=?1",
            [&sub.subscription_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    let Some((options_json, paused)) = settings else {
        return Ok(None);
    };
    if paused {
        return Ok(Some((NotificationTraceOutcome::Paused, None)));
    }
    let picked: Map<String, Value> = decode(&options_json)?;
    if picked.is_empty() {
        return Ok(None);
    }
    let declared = declared_options_in(db, &sub.owner_user_id, &env.source_id)?;
    if !still_declared(&declared, &picked) {
        return Ok(Some((NotificationTraceOutcome::OptionsChanged, None)));
    }
    Ok(picked
        .iter()
        .find(|(name, value)| env.output.data().get(*name) != Some(*value))
        .map(|(name, value)| {
            (
                NotificationTraceOutcome::Filtered,
                Some(not_matching(&declared, name, value)),
            )
        }))
}

fn still_declared(declared: &[NotificationSourceOption], picked: &Map<String, Value>) -> bool {
    picked.iter().all(|(name, value)| {
        declared
            .iter()
            .any(|option| option.name == *name && option.kind.admits(value))
    })
}

/// "Not PR number 976": the picked value the occurrence does not carry, in the source's words.
fn not_matching(declared: &[NotificationSourceOption], name: &str, value: &Value) -> String {
    let label = declared
        .iter()
        .find(|option| option.name == name)
        .map_or(name, |option| option.label.as_str());
    match value {
        Value::String(text) => format!("Not {label} {text}"),
        other => format!("Not {label} {other}"),
    }
}

/// A delivered occurrence counts as fired.
pub(super) fn delivered_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
    now: u64,
) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_subscription_settings (subscription_id, fired, last_fired_at_ms)
         VALUES (?1, 1, ?2)
         ON CONFLICT(subscription_id) DO UPDATE SET fired=fired+1, last_fired_at_ms=excluded.last_fired_at_ms",
        params![sub.subscription_id, now as i64],
    )
    .map_err(sql)?;
    trace_in(
        tx,
        &sub.subscription_id,
        env,
        NotificationTraceOutcome::Delivered,
        None,
        now,
    )
}

/// The occurrence came back to the subscription that caused it.
pub(super) fn loop_stopped_in(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
    now: u64,
) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_subscription_settings (subscription_id, loop_stopped_at_ms)
         VALUES (?1, ?2)
         ON CONFLICT(subscription_id) DO UPDATE SET loop_stopped_at_ms=excluded.loop_stopped_at_ms",
        params![sub.subscription_id, now as i64],
    )
    .map_err(sql)?;
    trace_in(
        tx,
        &sub.subscription_id,
        env,
        NotificationTraceOutcome::LoopStopped,
        None,
        now,
    )
}

/// The agent's turn a delivered occurrence started. An occurrence held by a pause reads as
/// delivered again once its prompt is queued.
pub(super) fn woke_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    occurrence_id: &str,
    turn: &NotificationTraceTurn,
) -> Result<(), DaemonError> {
    tx.execute(
        "UPDATE notification_trace SET outcome=?3, turn_agent_id=?4, turn_prompt_id=?5
         WHERE subscription_id=?1 AND occurrence_id=?2",
        params![
            subscription_id,
            occurrence_id,
            outcome_word(NotificationTraceOutcome::Delivered)?,
            turn.agent_id,
            turn.prompt_id
        ],
    )
    .map_err(sql)?;
    Ok(())
}

/// A workflow endpoint's prompt is queued. It runs no agent's turn, so the line names none.
pub(crate) fn handed_off_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    occurrence_id: &str,
) -> Result<(), DaemonError> {
    tx.execute(
        "UPDATE notification_trace SET outcome=?3 WHERE subscription_id=?1 AND occurrence_id=?2",
        params![
            subscription_id,
            occurrence_id,
            outcome_word(NotificationTraceOutcome::Delivered)?
        ],
    )
    .map_err(sql)?;
    Ok(())
}

/// An App event's prompt is queued: it counts as fired and its line names the turn. An App
/// event has no gate at acceptance, so this is its first line.
pub(crate) fn app_delivered_in(
    tx: &Transaction<'_>,
    automation_id: &str,
    occurrence_id: &str,
    title: &str,
    turn: &NotificationTraceTurn,
    now: u64,
) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_subscription_settings (subscription_id, fired, last_fired_at_ms)
         VALUES (?1, 1, ?2)
         ON CONFLICT(subscription_id) DO UPDATE SET fired=fired+1, last_fired_at_ms=excluded.last_fired_at_ms",
        params![automation_id, now as i64],
    )
    .map_err(sql)?;
    line_in(
        tx,
        automation_id,
        occurrence_id,
        title,
        NotificationTraceOutcome::Delivered,
        None,
        now,
    )?;
    woke_in(tx, automation_id, occurrence_id, turn)
}

/// Where the prompt would be queued: a paused subscription keeps the occurrence for resume or
/// expiry and its line says it waits. True when the subscription is paused; the line is only
/// written when it did not already say so, so a held occurrence retried each pass moves no
/// revision.
pub(super) fn hold_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    occurrence_id: &str,
    title: &str,
    now: u64,
) -> Result<Hold, DaemonError> {
    if !paused_in(tx, subscription_id)? {
        return Ok(Hold::Queue);
    }
    let paused = outcome_word(NotificationTraceOutcome::Paused)?;
    let said: Option<String> = tx
        .query_row(
            "SELECT outcome FROM notification_trace WHERE subscription_id=?1 AND occurrence_id=?2",
            [subscription_id, occurrence_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    if said.as_deref() == Some(paused.as_str()) {
        return Ok(Hold::Held { traced: false });
    }
    line_in(
        tx,
        subscription_id,
        occurrence_id,
        title,
        NotificationTraceOutcome::Paused,
        None,
        now,
    )?;
    Ok(Hold::Held { traced: true })
}

/// What the queue boundary does with one accepted occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hold {
    Queue,
    /// Paused: the occurrence stays for resume or expiry. `traced` when its line changed.
    Held {
        traced: bool,
    },
}

fn paused_in(db: &Connection, subscription_id: &str) -> Result<bool, DaemonError> {
    Ok(db
        .query_row(
            "SELECT paused FROM notification_subscription_settings WHERE subscription_id=?1",
            [subscription_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or(false))
}

/// Whether the subscription ends after its first delivery.
pub(super) fn once_in(tx: &Transaction<'_>, subscription_id: &str) -> Result<bool, DaemonError> {
    Ok(tx
        .query_row(
            "SELECT once FROM notification_subscription_settings WHERE subscription_id=?1",
            [subscription_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?
        .unwrap_or(false))
}

fn trace_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    env: &WorkflowNotificationEnvelope,
    outcome: NotificationTraceOutcome,
    detail: Option<String>,
    now: u64,
) -> Result<(), DaemonError> {
    line_in(
        tx,
        subscription_id,
        &env.occurrence_id,
        &title_of(env),
        outcome,
        detail,
        now,
    )
}

fn line_in(
    tx: &Transaction<'_>,
    subscription_id: &str,
    occurrence_id: &str,
    title: &str,
    outcome: NotificationTraceOutcome,
    detail: Option<String>,
    now: u64,
) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_trace (subscription_id, occurrence_id, at_ms, outcome, title, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(subscription_id, occurrence_id) DO UPDATE SET
           at_ms=excluded.at_ms, outcome=excluded.outcome, detail=excluded.detail",
        params![
            subscription_id,
            occurrence_id,
            now as i64,
            outcome_word(outcome)?,
            title,
            detail
        ],
    )
    .map_err(sql)?;
    prune_in(tx, subscription_id)
}

fn prune_in(tx: &Transaction<'_>, subscription_id: &str) -> Result<(), DaemonError> {
    tx.execute(
        "DELETE FROM notification_trace WHERE subscription_id=?1 AND rowid NOT IN
         (SELECT rowid FROM notification_trace WHERE subscription_id=?1 ORDER BY at_ms DESC, rowid DESC LIMIT ?2)",
        params![subscription_id, TRACE_LINES],
    )
    .map_err(sql)?;
    Ok(())
}

/// What the source declares now: from its own row on this kernel, or from the peer kernel's
/// cached list. A source this kernel does not know declares nothing.
pub(super) fn declared_options_in(
    db: &Connection,
    owner: &str,
    source_id: &str,
) -> Result<Vec<NotificationSourceOption>, DaemonError> {
    let local: Option<String> = db
        .query_row(
            "SELECT payload_json FROM workflow_notification_exit_sources WHERE source_id=?1",
            [source_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(local) = local {
        return Ok(decode::<WorkflowNotificationSource>(&local)?.options);
    }
    let mut cached = db
        .prepare("SELECT payload_json FROM workflow_notification_source_cache WHERE owner_id=?1")
        .map_err(sql)?;
    let lists = cached
        .query_map([owner], |row| row.get::<_, String>(0))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for list in lists {
        let sources: Vec<crate::local::WorkflowNotificationSourceSummary> = decode(&list)?;
        if let Some(source) = sources.into_iter().find(|s| s.source_id == source_id) {
            return Ok(source.options);
        }
    }
    Ok(Vec::new())
}

fn outcome_word(outcome: NotificationTraceOutcome) -> Result<String, DaemonError> {
    Ok(encode(&outcome)?.trim_matches('"').to_owned())
}

pub(crate) fn title_of(env: &WorkflowNotificationEnvelope) -> String {
    title_line(env.output.message())
}

/// An App event's line reads the App's own words for the occurrence, or the event's name.
pub(crate) fn app_event_title(invocation: Option<&str>, event_name: &str) -> String {
    invocation
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
        .and_then(|invocation| invocation.get("prompt")?.as_str().map(title_line))
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| event_name.to_owned())
}

/// The first line of an occurrence's words, as a trace line keeps it.
pub(crate) fn title_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(TITLE_CHARS)
        .collect()
}

/// What a subscription keeps here beside its row, as the subscriptions list shows it.
#[derive(Debug, Default)]
pub(crate) struct SubscriptionSettings {
    pub(crate) options: Vec<NotificationOptionValue>,
    pub(crate) once: bool,
    pub(crate) paused: bool,
    pub(crate) fired: u64,
    pub(crate) last_fired_at_ms: Option<u64>,
    /// When a delivery came back to this subscription and was dropped.
    pub(crate) loop_stopped_at_ms: Option<u64>,
    /// The source no longer declares an option the subscription picked.
    pub(crate) options_changed: bool,
}

/// The source's latest occurrence this kernel holds, and why the subscription would not
/// deliver it, if it would not.
pub(crate) struct DryRun {
    pub(crate) title: String,
    pub(crate) skipped: Option<(NotificationTraceOutcome, Option<String>)>,
}

/// options_json, once, paused, fired, last_fired_at_ms, loop_stopped_at_ms
type SettingsColumns = (String, bool, bool, i64, Option<i64>, Option<i64>);

impl DurableKernelStateStore {
    pub(crate) fn notification_subscription_settings(
        &self,
        sub: &WorkflowNotificationSubscription,
    ) -> Result<SubscriptionSettings, DaemonError> {
        let db = self.lock_connection("notification subscription settings")?;
        let row: Option<SettingsColumns> = db
            .query_row(
                "SELECT options_json, once, paused, fired, last_fired_at_ms, loop_stopped_at_ms
                 FROM notification_subscription_settings WHERE subscription_id=?1",
                [&sub.subscription_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?;
        let Some((options_json, once, paused, fired, last_fired, loop_stopped)) = row else {
            return Ok(SubscriptionSettings::default());
        };
        let picked: Map<String, Value> = decode(&options_json)?;
        let options_changed = !picked.is_empty()
            && !still_declared(
                &declared_options_in(&db, &sub.owner_user_id, &sub.source_id)?,
                &picked,
            );
        Ok(SubscriptionSettings {
            options: picked
                .into_iter()
                .map(|(name, value)| NotificationOptionValue { name, value })
                .collect(),
            once,
            paused,
            fired: fired as u64,
            last_fired_at_ms: last_fired.map(|at| at as u64),
            loop_stopped_at_ms: loop_stopped.map(|at| at as u64),
            options_changed,
        })
    }

    /// The subscription's trace, the latest occurrence first.
    pub(crate) fn notification_trace(
        &self,
        subscription_id: &str,
    ) -> Result<Vec<NotificationTraceLine>, DaemonError> {
        let db = self.lock_connection("notification trace")?;
        let mut query = db
            .prepare(
                "SELECT occurrence_id, at_ms, outcome, title, detail, turn_agent_id, turn_prompt_id
                 FROM notification_trace WHERE subscription_id=?1 ORDER BY at_ms DESC, rowid DESC",
            )
            .map_err(sql)?;
        let rows = query
            .query_map([subscription_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (occurrence_id, at_ms, outcome, title, detail, agent_id, prompt_id) =
                row.map_err(sql)?;
            Ok(NotificationTraceLine {
                occurrence_id,
                at_ms: at_ms as u64,
                outcome: decode(&format!("\"{outcome}\""))?,
                title,
                detail,
                turn: agent_id
                    .zip(prompt_id)
                    .map(|(agent_id, prompt_id)| NotificationTraceTurn {
                        agent_id,
                        prompt_id,
                    }),
            })
        })
        .collect()
    }

    /// What the subscription would do with the latest occurrence of its source this kernel has
    /// received; none when it has received none. Nothing is written.
    pub(crate) fn notification_dry_run(
        &self,
        sub: &WorkflowNotificationSubscription,
    ) -> Result<Option<DryRun>, DaemonError> {
        let db = self.lock_connection("notification dry run")?;
        let latest: Option<String> = db
            .query_row(
                "SELECT payload_json FROM app_outbox WHERE source_kind='workflow_completion'
                 AND owner_id=?1 AND installation_id=?2 AND payload_json IS NOT NULL
                 ORDER BY accepted_at_ms DESC, rowid DESC LIMIT 1",
                params![sub.owner_user_id, sub.source_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        let Some(latest) = latest else {
            return Ok(None);
        };
        let env: WorkflowNotificationEnvelope = decode(&latest)?;
        Ok(Some(DryRun {
            title: title_of(&env),
            skipped: verdict(&db, sub, &env)?,
        }))
    }

    /// The same question for an App event's listener, against the latest occurrence the App
    /// emitted for it. An App event has no options, so only a pause holds it back.
    pub(crate) fn notification_app_dry_run(
        &self,
        owner: &str,
        installation_id: &str,
        automation_id: &str,
    ) -> Result<Option<DryRun>, DaemonError> {
        let db = self.lock_connection("notification dry run")?;
        let latest: Option<(Option<String>, String)> = db
            .query_row(
                "SELECT invocation_json, event_name FROM app_outbox WHERE owner_id=?1
                 AND installation_id=?2 AND automation_id=?3
                 ORDER BY accepted_at_ms DESC, occurrence_id DESC LIMIT 1",
                params![owner, installation_id, automation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql)?;
        let Some((invocation, event_name)) = latest else {
            return Ok(None);
        };
        Ok(Some(DryRun {
            title: app_event_title(invocation.as_deref(), &event_name),
            skipped: paused_in(&db, automation_id)?
                .then_some((NotificationTraceOutcome::Paused, None)),
        }))
    }
}
