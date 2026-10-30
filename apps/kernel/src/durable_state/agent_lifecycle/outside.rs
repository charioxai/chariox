//! MP-08 / MP-09 / MP-10 / MP-11: pull recipients in the shared durable inbox.
use super::*;
use crate::local::{EventSubscription, EventTarget, EventsBatch, OutsideEvent, OutsideEventValue};

mod followed;
pub(crate) use followed::Followed;
mod health;
mod retention;
mod validation;
pub(super) use retention::bound;
use validation::{limit as inbox_limit, now as outside_now_ms};

pub(crate) const PENDING_LIMIT: usize = 100_000;

// These refusal sentences are kernel constants, safe to return to an outside caller.
pub(crate) fn refused(message: &'static str) -> DaemonError {
    DaemonError::MachineRequest {
        code: "events_refused".into(),
        message: message.into(),
    }
}
pub(crate) fn is_public_code(code: &str) -> bool {
    code == "events_refused"
}

mod model;
pub(crate) use model::{
    CountedInbox, Mutation, OpenDecision, Part, Recipient, RunState, SettledTurn,
};
mod producers;
use producers::identity;
pub(super) use producers::{publish_deadline, publish_task};

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    inbox_limit()?;
    outside_now_ms()?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_outside_subscriptions(id TEXT PRIMARY KEY,payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS agent_outside_identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),kernel TEXT NOT NULL,machine TEXT NOT NULL);").map_err(sql)
}
pub(super) fn read(db: &Connection, id: &str) -> Result<Option<Part>, DaemonError> {
    db.query_row(
        "SELECT payload FROM agent_outside_subscriptions WHERE id=?1",
        [id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(sql)?
    .map(|s| decode(&s))
    .transpose()
}
pub(super) fn put(tx: &Transaction<'_>, part: &Part) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO agent_outside_subscriptions VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![part.id, encode(part)?]).map_err(sql)?;
    Ok(())
}
fn parts(db: &Connection) -> Result<Vec<Part>, DaemonError> {
    let mut q = db
        .prepare("SELECT payload FROM agent_outside_subscriptions ORDER BY id")
        .map_err(sql)?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?
        .into_iter()
        .map(|s| decode(&s))
        .collect();
    rows
}
fn require(db: &Connection, id: &str) -> Result<Part, DaemonError> {
    read(db, id)?
        .ok_or_else(|| refused("no event subscription; run chariox events subscribe first"))
}
fn matches(part: &Part, event: &OutsideEvent) -> bool {
    part.targets.iter().any(|target| {
        target.kernel == event.kernel
            && target.session_id == event.session_id
            && (target.agent_id.is_none()
                || target.agent_id == event.agent_id
                || matches!(
                    event.value,
                    OutsideEventValue::Session {
                        state: crate::local::EventSessionState::Ended
                    }
                ))
    })
}
fn append(
    tx: &Transaction<'_>,
    part: &mut Part,
    occurrence: &str,
    event: &OutsideEvent,
) -> Result<(), DaemonError> {
    let recipient = Recipient::Outside {
        subscription_id: part.id.clone(),
    };
    let (room, agent) = recipient.address();
    let inserted = tx.execute("INSERT OR IGNORE INTO agent_inbox(room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,?2,?3,?4,?5)", params![room, agent, event.kernel, occurrence, encode(event)?]).map_err(sql)?;
    if inserted > 0 {
        part.latest = tx
            .last_insert_rowid()
            .try_into()
            .map_err(|_| error("invalid inbox sequence"))?;
    }
    Ok(())
}
fn end(tx: &Transaction<'_>, part: &mut Part, reason: &str) -> Result<(), DaemonError> {
    if part.ended.is_some() {
        return Ok(());
    }
    let (kernel, machine) = identity(tx)?.unwrap_or_else(|| ("local".into(), String::new()));
    let event = OutsideEvent {
        cursor: 0,
        kernel,
        machine,
        session_id: String::new(),
        agent_id: None,
        value: OutsideEventValue::SubscriptionEnded {
            reason: reason.into(),
        },
        summary: reason.into(),
        at_ms: outside_now_ms()?,
        refs: Default::default(),
    };
    append(tx, part, "subscription_ended", &event)?;
    bound(tx, part, inbox_limit()?)?;
    part.ended = Some(reason.into());
    part.ended_at_ms = Some(event.at_ms);
    if part.home {
        for target in &part.targets {
            if target.kernel != event.kernel && !part.cleanup_targets.contains(target) {
                part.cleanup_targets.push(target.clone());
            }
        }
    }
    part.targets.clear();
    part.run_states.clear();
    part.settled_prompts.clear();
    part.kernel_health.clear();
    Ok(())
}

pub(super) fn apply(tx: &Transaction<'_>, mutation: Mutation) -> Result<Outcome, DaemonError> {
    match mutation {
        Mutation::KernelHealth {
            id,
            kernel,
            online,
            reason,
        } => {
            health::record(tx, &id, &kernel, online, reason)?;
        }
        Mutation::RunStates { states } => {
            let Some((kernel, machine)) = identity(tx)? else {
                return Ok(Outcome::Saved);
            };
            // One decode and at most one write per part, however many states changed.
            let mut parts: Vec<_> = parts(tx)?
                .into_iter()
                .filter(|part| part.ended.is_none())
                .map(|part| (part, false))
                .collect();
            for state in states {
                let at_ms = outside_now_ms()?;
                let event = agent_event(&kernel, &machine, &state, state.state, at_ms);
                let key = run_state_key(&event)?;
                for (part, changed) in &mut parts {
                    if !matches(part, &event) {
                        continue;
                    }
                    let mut previous = part
                        .run_states
                        .get(&key)
                        .copied()
                        .unwrap_or(crate::local::EventRunState::Idle);
                    let unheard = state
                        .turn
                        .as_ref()
                        .filter(|turn| part.settled_prompts.get(&key) != Some(&turn.prompt_id));
                    if state.state == crate::local::EventRunState::Working {
                        // A turn that ended into a queued one leaves the agent working. It
                        // is heard on its own, ahead of the next turn's start.
                        if let Some(turn) = unheard {
                            previous = if turn.outcome.is_error() {
                                crate::local::EventRunState::Error
                            } else {
                                crate::local::EventRunState::Idle
                            };
                            let ended = agent_event(&kernel, &machine, &state, previous, at_ms);
                            append_run_state(tx, part, &key, previous, Some(turn), &ended)?;
                            *changed = true;
                        }
                        if previous != state.state {
                            append_run_state(tx, part, &key, state.state, None, &event)?;
                            *changed = true;
                        }
                    } else if previous != state.state || unheard.is_some() {
                        append_run_state(
                            tx,
                            part,
                            &key,
                            state.state,
                            state.turn.as_ref(),
                            &event,
                        )?;
                        *changed = true;
                    }
                }
            }
            for (mut part, _) in parts.into_iter().filter(|(_, changed)| *changed) {
                bound(tx, &mut part, inbox_limit()?)?;
                put(tx, &part)?;
            }
        }
        #[cfg(test)]
        Mutation::Bound { id, limit } => {
            let mut part = require(tx, &id)?;
            bound(tx, &mut part, limit)?;
            put(tx, &part)?;
        }
        Mutation::CleanupAck { id, targets } => {
            let mut part = require(tx, &id)?;
            part.cleanup_targets
                .retain(|target| !targets.contains(target));
            put(tx, &part)?;
        }
        Mutation::Identity { kernel, machine } => {
            tx.execute("INSERT INTO agent_outside_identity VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET kernel=excluded.kernel,machine=excluded.machine", params![kernel,machine]).map_err(sql)?;
        }
        Mutation::Subscribe {
            id,
            home,
            targets,
            baseline,
            open_decisions,
        } => {
            let existing = read(tx, &id)?;
            let mut changed = existing.is_none();
            let mut part = existing.unwrap_or_else(|| Part::new(id, home));
            if part.ended.is_some() {
                return Err(refused("subscription ended; request a new access grant"));
            }
            let mut added = Vec::new();
            for target in targets {
                let cleanup_before = part.cleanup_targets.len();
                part.cleanup_targets.retain(|removed| removed != &target);
                changed |= cleanup_before != part.cleanup_targets.len();
                if !part.targets.contains(&target) {
                    changed = true;
                    added.push(target.clone());
                    part.targets.push(target);
                }
            }
            replay_open_decisions(tx, &mut part, &added, open_decisions)?;
            if part.targets.len() > 1024 {
                return Err(refused(
                    "event target limit is 1024; follow whole sessions instead",
                ));
            }
            if !baseline.is_empty() {
                let (kernel, _) =
                    identity(tx)?.ok_or_else(|| error("outside source identity is missing"))?;
                for state in baseline {
                    let key =
                        serde_json::to_string(&(&kernel, &state.session_id, Some(&state.agent_id)))
                            .map_err(|_| error("event run-state key could not be encoded"))?;
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        part.run_states.entry(key.clone())
                    {
                        entry.insert(state.state);
                        if let Some(turn) = &state.turn {
                            part.settled_prompts.insert(key, turn.prompt_id.clone());
                        }
                        changed = true;
                    }
                }
            }
            if changed {
                put(tx, &part)?;
            }
        }
        Mutation::Unsubscribe {
            id,
            targets,
            all,
            reason,
        } => {
            if let Some(mut part) = read(tx, &id)? {
                if all {
                    end(tx, &mut part, &reason)?;
                } else {
                    if part.home {
                        let local = identity(tx)?.map(|(kernel, _)| kernel);
                        for target in &part.targets {
                            if local.as_deref() != Some(target.kernel.as_str())
                                && targets.contains(target)
                                && !part.cleanup_targets.contains(target)
                            {
                                part.cleanup_targets.push(target.clone());
                            }
                        }
                    }
                    part.targets.retain(|t| !targets.contains(t));
                }
                prune_target_state(&mut part)?;
                put(tx, &part)?;
            }
        }
        Mutation::Publish { occurrence, event } => publish(tx, &occurrence, &event)?,
        Mutation::Import { id, kernel, events } => {
            let mut part = require(tx, &id)?;
            if part.ended.is_some() {
                return Ok(Outcome::Saved);
            }
            if events
                .windows(2)
                .any(|pair| pair[0].cursor >= pair[1].cursor)
                || events.iter().any(|event| event.kernel != kernel)
            {
                return Err(error(
                    "target event batch is unordered or names a different kernel",
                ));
            }
            for event in events {
                let previous = part.remote_cursors.get(&kernel).copied().unwrap_or(0);
                if event.cursor <= previous {
                    continue;
                }
                if matches!(event.value, OutsideEventValue::SubscriptionEnded { .. }) {
                    return Err(error(
                        "target event part ended unexpectedly; inspect the subscription",
                    ));
                }
                append(tx, &mut part, &format!("remote-{}", event.cursor), &event)?;
                part.remote_cursors.insert(kernel.clone(), event.cursor);
                remove_ended_target(tx, &mut part, &event)?;
            }
            bound(tx, &mut part, inbox_limit()?)?;
            put(tx, &part)?;
        }
        Mutation::Ack { id, cursor } => {
            let mut part = require(tx, &id)?;
            if cursor > part.latest {
                return Err(refused(
                    "ack cursor is ahead of the inbox; list events first",
                ));
            }
            part.ack = part.ack.max(cursor);
            part.floor = part.floor.max(part.ack);
            tx.execute(
                "DELETE FROM agent_inbox WHERE room_id='' AND agent_id=?1 AND sequence<=?2",
                params![id, sql_integer(part.ack)?],
            )
            .map_err(sql)?;
            put(tx, &part)?;
        }
        Mutation::Restart { keep } => {
            let local = identity(tx)?.map(|(kernel, _)| kernel);
            for mut part in parts(tx)? {
                if part.home && !keep.contains(&part.id) {
                    end(tx, &mut part, "kernel_restarted")?;
                    // Older homes queued local targets even though no remote part
                    // exists for them. Keep only work a remote pump can finish.
                    part.cleanup_targets
                        .retain(|target| local.as_deref() != Some(target.kernel.as_str()));
                    put(tx, &part)?;
                }
            }
        }
    }
    Ok(Outcome::Saved)
}
/// Gives a new follower the `pending` event of each decision that opened before
/// it followed. Targets it already had saw them live.
fn replay_open_decisions(
    tx: &Transaction<'_>,
    part: &mut Part,
    added: &[EventTarget],
    decisions: Vec<OpenDecision>,
) -> Result<(), DaemonError> {
    let Some((kernel, machine)) = identity(tx)? else {
        return Ok(());
    };
    for decision in decisions {
        let in_scope = added.iter().any(|target| {
            target.kernel == kernel
                && target.session_id == decision.session_id
                && (target.agent_id.is_none() || target.agent_id == decision.agent_id)
        });
        if !in_scope {
            continue;
        }
        let event = project(OutsideEvent {
            cursor: 0,
            kernel: kernel.clone(),
            machine: machine.clone(),
            session_id: decision.session_id,
            agent_id: decision.agent_id,
            value: decision.value,
            summary: decision.summary,
            at_ms: outside_now_ms()?,
            refs: crate::local::EventRefs {
                interaction_id: Some(decision.interaction_id.clone()),
                ..Default::default()
            },
        })?;
        append(
            tx,
            part,
            &format!("open-decision-{}", decision.interaction_id),
            &event,
        )?;
    }
    bound(tx, part, inbox_limit()?)
}
fn remove_ended_target(
    tx: &Transaction<'_>,
    part: &mut Part,
    event: &OutsideEvent,
) -> Result<(), DaemonError> {
    if matches!(
        event.value,
        OutsideEventValue::Session {
            state: crate::local::EventSessionState::Ended
        }
    ) {
        if part.home {
            let local = identity(tx)?.map(|(kernel, _)| kernel);
            for target in &part.targets {
                if local.as_deref() != Some(target.kernel.as_str())
                    && target.kernel == event.kernel
                    && target.session_id == event.session_id
                    && !part.cleanup_targets.contains(target)
                {
                    part.cleanup_targets.push(target.clone());
                }
            }
        }
        part.targets
            .retain(|t| t.kernel != event.kernel || t.session_id != event.session_id);
    }
    if matches!(
        event.value,
        OutsideEventValue::Session {
            state: crate::local::EventSessionState::AgentRemoved
        }
    ) {
        part.run_states.remove(&run_state_key(event)?);
        // The event just recorded tells the holder; keeping a target that can
        // never match again would only make a later subscribe refuse it.
        part.targets.retain(|t| {
            t.agent_id.is_none()
                || t.kernel != event.kernel
                || t.session_id != event.session_id
                || t.agent_id != event.agent_id
        });
    }
    prune_target_state(part)
}
/// The agent event for `run_state`, carrying the settled turn unless the agent is working.
fn agent_event(
    kernel: &str,
    machine: &str,
    state: &RunState,
    run_state: crate::local::EventRunState,
    at_ms: u64,
) -> OutsideEvent {
    let turn = state
        .turn
        .as_ref()
        .filter(|_| run_state != crate::local::EventRunState::Working);
    OutsideEvent {
        cursor: 0,
        kernel: kernel.into(),
        machine: machine.into(),
        session_id: state.session_id.clone(),
        agent_id: Some(state.agent_id.clone()),
        value: OutsideEventValue::Agent {
            run_state: Some(run_state),
            task_state: None,
            reason: None,
            prompt_id: turn.map(|turn| turn.prompt_id.clone()),
            turn_id: turn.map(|turn| turn.turn_id.clone()),
            outcome: turn.map(|turn| turn.outcome.clone()),
        },
        summary: match (run_state, turn) {
            (crate::local::EventRunState::Working, _) => "working".into(),
            (crate::local::EventRunState::Idle, _) => "idle".into(),
            (crate::local::EventRunState::Error, None) => "error".into(),
            (crate::local::EventRunState::Error, Some(turn)) => {
                format!("error: {}", turn.outcome.summary())
            }
        },
        at_ms,
        refs: Default::default(),
    }
}
/// Hands `event` to the holder and remembers the state, and the turn it told of.
fn append_run_state(
    tx: &Transaction<'_>,
    part: &mut Part,
    key: &str,
    run_state: crate::local::EventRunState,
    turn: Option<&SettledTurn>,
    event: &OutsideEvent,
) -> Result<(), DaemonError> {
    part.run_states.insert(key.into(), run_state);
    if let Some(turn) = turn {
        part.settled_prompts
            .insert(key.into(), turn.prompt_id.clone());
    }
    append(
        tx,
        part,
        &format!("activity-{:032x}", rand::random::<u128>()),
        event,
    )
}
fn run_state_key(event: &OutsideEvent) -> Result<String, DaemonError> {
    serde_json::to_string(&(&event.kernel, &event.session_id, &event.agent_id))
        .map_err(|_| error("event run-state key could not be encoded"))
}
fn prune_target_state(part: &mut Part) -> Result<(), DaemonError> {
    part.kernel_health
        .retain(|kernel, _| part.targets.iter().any(|target| &target.kernel == kernel));
    let mut kept = std::collections::BTreeMap::new();
    for (key, state) in std::mem::take(&mut part.run_states) {
        let (kernel, session, agent): (String, String, Option<String>) =
            serde_json::from_str(&key).map_err(|_| error("event run-state key is invalid"))?;
        if part.targets.iter().any(|target| {
            target.kernel == kernel
                && target.session_id == session
                && (target.agent_id.is_none() || target.agent_id == agent)
        }) {
            kept.insert(key, state);
        }
    }
    part.run_states = kept;
    part.settled_prompts
        .retain(|key, _| part.run_states.contains_key(key));
    Ok(())
}
/// What an outside caller may see of an event: secrets redacted.
fn project(event: OutsideEvent) -> Result<OutsideEvent, DaemonError> {
    let mut projected =
        serde_json::to_value(event).map_err(|_| error("outside event encoding failed"))?;
    crate::secret_redaction::redact_json_secrets(&mut projected);
    serde_json::from_value(projected).map_err(|_| error("outside event projection failed"))
}
pub(super) fn publish(
    tx: &Transaction<'_>,
    occurrence: &str,
    event: &OutsideEvent,
) -> Result<(), DaemonError> {
    let event = project(event.clone())?;
    for mut part in parts(tx)? {
        if part.ended.is_none() && matches(&part, &event) {
            append(tx, &mut part, occurrence, &event)?;
            remove_ended_target(tx, &mut part, &event)?;
            bound(tx, &mut part, inbox_limit()?)?;
            put(tx, &part)?;
        }
    }
    Ok(())
}

pub(crate) struct TerminalCompletion {
    pub session: String,
    pub agent: Option<String>,
    pub terminal: String,
    pub command: Option<u64>,
    pub offset: u64,
    pub exit_code: Option<u32>,
}

impl DurableKernelStateStore {
    pub(crate) fn outside_mutate(&self, mutation: Mutation) -> Result<(), DaemonError> {
        self.agent_lifecycle(Operation::Outside(mutation))?;
        Ok(())
    }
    /// Run states are committed without a waiter, in queue order: once this empty
    /// publication commits, every publication queued before it has.
    #[cfg(test)]
    pub(crate) fn outside_settled(&self) -> Result<Outcome, DaemonError> {
        self.agent_lifecycle(Operation::Outside(Mutation::RunStates { states: vec![] }))
    }
    pub(crate) fn outside_part(&self, id: &str) -> Result<Option<Part>, DaemonError> {
        read(&*self.lock_connection("outside.part")?, id)
    }
    pub(crate) fn outside_parts(&self) -> Result<Vec<Part>, DaemonError> {
        parts(&*self.lock_connection("outside.parts")?)
    }
    pub(crate) fn outside_list(
        &self,
        id: &str,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Result<EventsBatch, u64>, DaemonError> {
        let db = self.lock_connection("outside.inbox")?;
        let snapshot = db.unchecked_transaction().map_err(sql)?;
        let part = require(&snapshot, id)?;
        let after = since.unwrap_or(part.ack);
        if after < part.floor {
            return Ok(Err(part.floor + 1));
        }
        if after > part.latest {
            return Err(refused(
                "cursor is ahead of the inbox; use the cursor returned by list",
            ));
        }
        let mut q = snapshot.prepare("SELECT sequence,payload FROM agent_inbox WHERE room_id='' AND agent_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(sql)?;
        let rows = q
            .query_map(
                params![id, sql_integer(after)?, limit.clamp(1, 1000) as i64],
                |r| Ok((cursor_from_row(r)?, r.get::<_, String>(1)?)),
            )
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let mut events = Vec::new();
        let mut bytes = 0;
        for (cursor, payload) in rows {
            let mut event: OutsideEvent = decode(&payload)?;
            event.cursor = cursor;
            let length = serde_json::to_vec(&event)
                .map_err(|_| error("outside event encoding failed"))?
                .len()
                + 1;
            if length > 2 * 1024 * 1024 {
                return Err(error(
                    "outside event exceeds the transport budget; inspect the source kernel",
                ));
            }
            if bytes + length > 2 * 1024 * 1024 {
                break;
            }
            bytes += length;
            events.push(event);
        }
        let next_cursor = events.last().map_or(after, |e| e.cursor);
        Ok(Ok(EventsBatch {
            events,
            next_cursor,
            oldest_cursor: part.floor + 1,
            timed_out: false,
        }))
    }
}

fn cursor_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<u64> {
    let value: i64 = row.get(0)?;
    value
        .try_into()
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}
