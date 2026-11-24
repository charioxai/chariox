//! A watcher's durable record and the writes that move it. Everything a run changes (its events,
//! its cursor, its reading and its health) commits in the notification writer's one transaction,
//! so a crash or a restart neither repeats nor loses an event.
use super::*;
use crate::local::{
    NotificationSourceKind, WatcherCoalesce, WatcherDryRun, WatcherEvent, WatcherHealth,
    WatcherManifest, WatcherScript, WatcherState, WatcherSummary,
};
use crate::session::WorkflowOutputPayload;
use serde::{Deserialize, Serialize};

/// Events one source may fire in `RATE_WINDOW_MS` before it slows itself.
pub(crate) const RATE_LIMIT: usize = 30;
pub(crate) const RATE_WINDOW_MS: u64 = 10 * 60 * 1000;
/// A slowed source fires one held event this often.
pub(crate) const SLOWED_GAP_MS: u64 = 60 * 1000;
/// The script's memory, kept by the kernel.
pub(crate) const MAX_CURSOR_BYTES: usize = 64 * 1024;
/// Events a source holds while it waits for quiet or is slowed; the oldest go first.
const MAX_HELD: usize = 256;

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS notification_watchers (
      watcher_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, payload_json TEXT NOT NULL);",
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HeldEvent {
    pub occurrence_id: String,
    pub event: WatcherEvent,
}

/// The durable shape of a watcher. `revision` counts changes to its manifest and script; a run
/// commits only under the revision it started with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct WatcherRecord {
    pub watcher_id: String,
    pub owner_id: String,
    pub source_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub creator_agent_id: Option<String>,
    /// The owner took responsibility for turning on each revision of this source.
    #[serde(default)]
    pub owner_activation: bool,
    pub manifest: WatcherManifest,
    pub script: WatcherScript,
    pub state: WatcherState,
    pub health: WatcherHealth,
    pub revision: u64,
    /// The last dry run and the revision it ran; it counts only while that is the current one.
    pub dry_run: Option<PassedDryRun>,
    pub cursor: Option<serde_json::Value>,
    pub reading: Option<String>,
    pub last_check_ms: Option<u64>,
    pub last_ok_ms: Option<u64>,
    /// When it was last turned on: a gap counts from here or from the last good run.
    pub active_since_ms: Option<u64>,
    pub last_fired_ms: Option<u64>,
    pub fired_count: u64,
    /// When each recent event fired, for the rate limit.
    pub fired_window: Vec<u64>,
    pub held: Vec<HeldEvent>,
    pub held_release_ms: Option<u64>,
    /// The end of the last failed run's error output, for the agent that repairs it.
    pub last_failure_output: Option<String>,
    pub created_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PassedDryRun {
    pub revision: u64,
    pub result: WatcherDryRun,
    /// Some dry run of this revision reported a reading: a later preview that lost it does not
    /// lift the requirement, since the source may have become unreadable since.
    #[serde(default)]
    pub reading_demonstrated: bool,
}

impl WatcherRecord {
    pub(crate) fn passed_dry_run(&self) -> Option<&WatcherDryRun> {
        self.dry_run
            .as_ref()
            .filter(|dry_run| dry_run.revision == self.revision)
            .map(|dry_run| &dry_run.result)
    }

    /// A run of this revision must report a reading: one of its dry runs did.
    pub(crate) fn expects_reading(&self) -> bool {
        self.dry_run
            .as_ref()
            .filter(|dry_run| dry_run.revision == self.revision)
            .is_some_and(|dry_run| dry_run.reading_demonstrated || dry_run.result.reading.is_some())
    }

    pub(crate) fn summary(&self) -> WatcherSummary {
        WatcherSummary {
            watcher_id: self.watcher_id.clone(),
            source_id: self.source_id.clone(),
            kernel_id: self.kernel_id.clone(),
            session_id: self.session_id.clone(),
            creator_agent_id: self.creator_agent_id.clone(),
            manifest: self.manifest.clone(),
            state: self.state,
            health: self.health.clone(),
            reading: self.reading.clone(),
            last_check_ms: self.last_check_ms,
            last_fired_ms: self.last_fired_ms,
            fired_count: self.fired_count,
            dry_run: self.passed_dry_run().cloned(),
        }
    }

    /// The notification source subscribers follow; it lives as long as the watcher.
    pub(crate) fn source(&self, enabled: bool) -> WorkflowNotificationSource {
        WorkflowNotificationSource {
            kind: NotificationSourceKind::Watcher,
            source_id: self.source_id.clone(),
            owner_user_id: self.owner_id.clone(),
            kernel_id: self.kernel_id.clone(),
            session_id: self.session_id.clone(),
            workflow_id: self.watcher_id.clone(),
            exit_node_id: "event".into(),
            enabled,
            available: true,
            name: self.manifest.name.clone(),
            exit_ordinal: 0,
            display_name: self.manifest.name.clone(),
            // A watcher's manifest declares no options a subscriber could pick.
            options: Vec::new(),
        }
    }
}

/// What a run of the script found. A poll run commits once when it exits with 0; a stream
/// commits at each cursor line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunCommit {
    pub watcher_id: String,
    pub revision: u64,
    pub run_id: String,
    pub reading: Option<String>,
    pub cursor: Option<serde_json::Value>,
    pub events: Vec<WatcherEvent>,
    pub at: u64,
}

#[derive(Debug)]
pub(crate) enum WatcherOperation {
    Create(Box<WatcherRecord>),
    Change {
        owner: String,
        watcher_id: String,
        manifest: WatcherManifest,
        script: WatcherScript,
    },
    DryRunPassed {
        owner: String,
        watcher_id: String,
        revision: u64,
        result: WatcherDryRun,
    },
    SetState {
        owner: String,
        watcher_id: String,
        state: WatcherState,
        by_owner: bool,
        at: u64,
    },
    /// The name the person gave it at Turn on: it changes what it is called, not what it does.
    Rename {
        owner: String,
        watcher_id: String,
        name: String,
    },
    Remove {
        owner: String,
        watcher_id: String,
    },
    Commit(RunCommit),
    Failed {
        watcher_id: String,
        revision: u64,
        reason: String,
        output: Option<String>,
        at: u64,
    },
    /// No good run for three periods.
    Gap {
        watcher_id: String,
        at: u64,
    },
    /// Fires what the source held once its quiet or its slowed gap is over.
    Release {
        watcher_id: String,
        at: u64,
    },
}

/// The record after the write, and the health it entered if it changed to a state its
/// responsible hears about.
#[derive(Debug)]
pub(crate) struct WatcherWritten {
    pub record: WatcherRecord,
    pub entered: Option<WatcherHealth>,
}

fn load(tx: &Transaction<'_>, watcher_id: &str) -> Result<WatcherRecord, DaemonError> {
    let json: Option<String> = tx
        .query_row(
            "SELECT payload_json FROM notification_watchers WHERE watcher_id=?1",
            [watcher_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    decode(&json.ok_or_else(|| error("watcher not found"))?)
}

fn load_owned(
    tx: &Transaction<'_>,
    owner: &str,
    watcher_id: &str,
) -> Result<WatcherRecord, DaemonError> {
    let record = load(tx, watcher_id)?;
    if record.owner_id != owner {
        return Err(error("watcher not found"));
    }
    Ok(record)
}

fn save(tx: &Transaction<'_>, record: &WatcherRecord) -> Result<(), DaemonError> {
    tx.execute(
        "INSERT INTO notification_watchers VALUES (?1,?2,?3) ON CONFLICT(watcher_id) DO UPDATE SET payload_json=excluded.payload_json",
        params![record.watcher_id, record.owner_id, encode(record)?],
    )
    .map_err(sql)?;
    Ok(())
}

pub(super) fn apply(
    tx: &Transaction<'_>,
    operation: WatcherOperation,
) -> Result<NotificationOutcome, DaemonError> {
    let written = |record: WatcherRecord, entered: Option<WatcherHealth>| {
        save(tx, &record)?;
        Ok(NotificationOutcome::Watcher(Box::new(WatcherWritten {
            record,
            entered,
        })))
    };
    match operation {
        WatcherOperation::Create(record) => {
            super::apply(
                tx,
                NotificationOperation::Register(SourceAdmission {
                    source: record.source(true),
                    workflow_json: String::new(),
                }),
            )?;
            written(*record, None)
        }
        WatcherOperation::Change {
            owner,
            watcher_id,
            manifest,
            script,
        } => {
            let mut record = load_owned(tx, &owner, &watcher_id)?;
            record.manifest = manifest;
            record.script = script;
            record.revision += 1;
            record.state = WatcherState::Draft;
            if !record.owner_activation {
                record.health = WatcherHealth::Ok;
            }
            record.held.clear();
            record.held_release_ms = None;
            register(tx, &record, true)?;
            written(record, None)
        }
        WatcherOperation::DryRunPassed {
            owner,
            watcher_id,
            revision,
            result,
        } => {
            let mut record = load_owned(tx, &owner, &watcher_id)?;
            if record.revision != revision {
                return Err(error(
                    "the watcher changed during its dry run; run it again",
                ));
            }
            let reading_demonstrated = result.reading.is_some() || record.expects_reading();
            record.dry_run = Some(PassedDryRun {
                revision,
                result,
                reading_demonstrated,
            });
            written(record, None)
        }
        WatcherOperation::SetState {
            owner,
            watcher_id,
            state,
            by_owner,
            at,
        } => {
            let mut record = load_owned(tx, &owner, &watcher_id)?;
            match (record.state, state) {
                (WatcherState::Draft, WatcherState::Active) => {
                    if record.owner_activation && !by_owner {
                        return Err(error(
                            "only the owner can turn this watcher on after reviewing its dry run",
                        ));
                    }
                    if record.passed_dry_run().is_none() {
                        return Err(error(
                            "run a dry run of this version of the watcher before turning it on",
                        ));
                    }
                    record.owner_activation = by_owner;
                    record.health = WatcherHealth::Ok;
                    record.last_check_ms = None;
                    record.last_ok_ms = None;
                    record.active_since_ms = Some(at);
                }
                (WatcherState::Draft, WatcherState::Paused) => {
                    return Err(error("the watcher is not on yet; activate it first"))
                }
                (WatcherState::Paused, WatcherState::Active) => {
                    record.health = WatcherHealth::Ok;
                    record.active_since_ms = Some(at);
                }
                _ => {}
            }
            record.state = state;
            written(record, None)
        }
        WatcherOperation::Rename {
            owner,
            watcher_id,
            name,
        } => {
            let mut record = load_owned(tx, &owner, &watcher_id)?;
            record.manifest.name = name;
            register(tx, &record, true)?;
            written(record, None)
        }
        WatcherOperation::Remove { owner, watcher_id } => {
            let record = load_owned(tx, &owner, &watcher_id)?;
            register(tx, &record, false)?;
            tx.execute(
                "DELETE FROM notification_watchers WHERE watcher_id=?1",
                [&watcher_id],
            )
            .map_err(sql)?;
            Ok(NotificationOutcome::Watcher(Box::new(WatcherWritten {
                record,
                entered: None,
            })))
        }
        WatcherOperation::Commit(commit) => {
            let mut record = load(tx, &commit.watcher_id)?;
            if record.state != WatcherState::Active || record.revision != commit.revision {
                return Err(error("watcher run is stale"));
            }
            record.last_check_ms = Some(commit.at);
            record.last_ok_ms = Some(commit.at);
            record.last_failure_output = None;
            if let Some(reading) = commit.reading {
                record.reading = Some(reading);
            }
            if let Some(cursor) = commit.cursor {
                record.cursor = Some(cursor);
            }
            if matches!(
                record.health,
                WatcherHealth::Failing { .. } | WatcherHealth::Gap { .. }
            ) {
                record.health = WatcherHealth::Ok;
            }
            let events = coalesced(record.manifest.coalesce, commit.events);
            let held = events
                .into_iter()
                .enumerate()
                .map(|(n, event)| HeldEvent {
                    occurrence_id: format!("{}:{n}", commit.run_id),
                    event,
                })
                .collect::<Vec<_>>();
            let entered = if record.manifest.debounce_s > 0 && !held.is_empty() {
                hold(&mut record, held);
                record.held_release_ms = Some(commit.at + record.manifest.debounce_s * 1000);
                None
            } else if matches!(record.health, WatcherHealth::Noisy { .. }) {
                hold(&mut record, held);
                None
            } else {
                fire(tx, &mut record, held, commit.at)?
            };
            written(record, entered)
        }
        WatcherOperation::Failed {
            watcher_id,
            revision,
            reason,
            output,
            at,
        } => {
            let mut record = load(tx, &watcher_id)?;
            if record.state != WatcherState::Active || record.revision != revision {
                return Err(error("watcher run is stale"));
            }
            record.last_check_ms = Some(at);
            record.last_failure_output = output;
            let (since_ms, entered) = match record.health {
                WatcherHealth::Failing { since_ms, .. } => (since_ms, false),
                _ => (at, true),
            };
            record.health = WatcherHealth::Failing { since_ms, reason };
            let entered = entered.then(|| record.health.clone());
            written(record, entered)
        }
        WatcherOperation::Gap { watcher_id, at } => {
            let mut record = load(tx, &watcher_id)?;
            if record.state != WatcherState::Active || record.health != WatcherHealth::Ok {
                return written(record, None);
            }
            record.health = WatcherHealth::Gap { since_ms: at };
            let entered = Some(record.health.clone());
            written(record, entered)
        }
        WatcherOperation::Release { watcher_id, at } => {
            let mut record = load(tx, &watcher_id)?;
            if record.state != WatcherState::Active
                || record.held_release_ms.is_none_or(|due| due > at)
            {
                return written(record, None);
            }
            record.held_release_ms = None;
            let held = std::mem::take(&mut record.held);
            let entered = if let WatcherHealth::Noisy { .. } = record.health {
                let mut held = held.into_iter();
                let first = held.next();
                record.held = held.collect();
                prune_window(&mut record, at);
                if let Some(first) = first {
                    publish(tx, &mut record, first, at)?;
                }
                if record.held.is_empty() && record.fired_window.len() < RATE_LIMIT {
                    record.health = WatcherHealth::Ok;
                } else {
                    record.held_release_ms = Some(at + SLOWED_GAP_MS);
                }
                None
            } else {
                fire(tx, &mut record, held, at)?
            };
            written(record, entered)
        }
    }
}

fn register(
    tx: &Transaction<'_>,
    record: &WatcherRecord,
    enabled: bool,
) -> Result<(), DaemonError> {
    super::apply(
        tx,
        NotificationOperation::Register(SourceAdmission {
            source: record.source(enabled),
            workflow_json: String::new(),
        }),
    )?;
    Ok(())
}

/// Latest per key keeps each key's newest event, in the order the newest ones came.
fn coalesced(coalesce: WatcherCoalesce, events: Vec<WatcherEvent>) -> Vec<WatcherEvent> {
    match coalesce {
        WatcherCoalesce::Every => events,
        WatcherCoalesce::LatestPerKey => {
            let mut kept: Vec<WatcherEvent> = Vec::new();
            for event in events {
                kept.retain(|held| !same_key(held, &event));
                kept.push(event);
            }
            kept
        }
    }
}

fn same_key(a: &WatcherEvent, b: &WatcherEvent) -> bool {
    a.event == b.event && a.key.is_some() && a.key == b.key
}

/// A slowed source keeps one event per key; unkeyed events are kept per event name.
fn slowed_key(event: &WatcherEvent) -> (&str, Option<&str>) {
    (event.event.as_str(), event.key.as_deref())
}

fn hold(record: &mut WatcherRecord, events: Vec<HeldEvent>) {
    let noisy = matches!(record.health, WatcherHealth::Noisy { .. });
    let latest = record.manifest.coalesce == WatcherCoalesce::LatestPerKey;
    for event in events {
        if noisy {
            record
                .held
                .retain(|held| slowed_key(&held.event) != slowed_key(&event.event));
        } else if latest {
            record
                .held
                .retain(|held| !same_key(&held.event, &event.event));
        }
        record.held.push(event);
    }
    let over = record.held.len().saturating_sub(MAX_HELD);
    record.held.drain(..over);
}

fn prune_window(record: &mut WatcherRecord, at: u64) {
    record
        .fired_window
        .retain(|fired| at.saturating_sub(*fired) < RATE_WINDOW_MS);
}

/// Fires events up to the rate limit; the rest are held and the source slows itself.
fn fire(
    tx: &Transaction<'_>,
    record: &mut WatcherRecord,
    events: Vec<HeldEvent>,
    at: u64,
) -> Result<Option<WatcherHealth>, DaemonError> {
    prune_window(record, at);
    let mut events = events.into_iter();
    for event in events.by_ref() {
        if record.fired_window.len() >= RATE_LIMIT {
            record.health = WatcherHealth::Noisy { since_ms: at };
            hold(record, std::iter::once(event).chain(events).collect());
            record.held_release_ms = Some(at + SLOWED_GAP_MS);
            return Ok(Some(record.health.clone()));
        }
        publish(tx, record, event, at)?;
    }
    Ok(None)
}

fn publish(
    tx: &Transaction<'_>,
    record: &mut WatcherRecord,
    held: HeldEvent,
    at: u64,
) -> Result<(), DaemonError> {
    let source = load_source(tx, &record.source_id)?;
    let inserted = tx
        .execute(
            "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
            params![source.source_id, held.occurrence_id],
        )
        .map_err(sql)?;
    if inserted == 0 {
        return Ok(());
    }
    completion::publish_in(
        tx,
        &source,
        &held.occurrence_id,
        WorkflowOutputPayload::new(held.event.text, Vec::new()),
        Vec::new(),
        at,
    )?;
    prune_occurrences(tx).map_err(sql)?;
    record.fired_window.push(at);
    record.fired_count += 1;
    record.last_fired_ms = Some(at);
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn watcher(&self, watcher_id: &str) -> Result<Option<WatcherRecord>, DaemonError> {
        let db = self.lock_connection("notification.watchers.read")?;
        let json: Option<String> = db
            .query_row(
                "SELECT payload_json FROM notification_watchers WHERE watcher_id=?1",
                [watcher_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?;
        json.map(|json| decode(&json)).transpose()
    }

    /// The owner's watchers, or every watcher of this kernel.
    pub(crate) fn watchers(&self, owner: Option<&str>) -> Result<Vec<WatcherRecord>, DaemonError> {
        let db = self.lock_connection("notification.watchers.list")?;
        let mut query = db
            .prepare("SELECT payload_json FROM notification_watchers WHERE ?1 IS NULL OR owner_id=?1 ORDER BY watcher_id")
            .map_err(sql)?;
        let rows = query
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(sql)?;
        rows.map(|row| decode(&row.map_err(sql)?)).collect()
    }

    pub(crate) fn watcher_write(
        &self,
        operation: WatcherOperation,
    ) -> Result<WatcherWritten, DaemonError> {
        match self.notify(NotificationOperation::Watcher(operation))? {
            NotificationOutcome::Watcher(written) => Ok(*written),
            _ => Err(error("unexpected watcher write result")),
        }
    }
}
