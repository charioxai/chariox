//! Kernel-owned workflow notification store on the existing single durable writer.
//! Completion/outbox, ACK/inbox and queue/receipt each commit atomically.
mod completion;
mod queue;
#[cfg(test)]
pub(crate) mod tests;

use super::notification_target::WorkflowNotificationTarget;
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{
    error::DaemonError,
    local::{
        WorkflowNotificationAck, WorkflowNotificationDiagnostic, WorkflowNotificationEnvelope,
        WorkflowNotificationSource, WorkflowNotificationSubscription,
    },
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::sync::mpsc;

pub(crate) const MAX_OUTPUT_BYTES: usize = chariox_event_protocol::MAX_EVENT_PROMPT_BYTES - 1024;
pub(crate) const MAX_PROMPT_BYTES: usize = chariox_event_protocol::MAX_EVENT_PROMPT_BYTES;
const MAX_SUBSCRIPTIONS: usize = 32;
const MAX_PENDING: i64 = 1024;
pub(super) use completion::capture_in;
pub(crate) use queue::PreparedNotification;

pub(crate) fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "workflow.notifications",
        message: message.into(),
    }
}
fn sql(error: rusqlite::Error) -> DaemonError {
    super::storage_full::observe(&error);
    self::error(error.to_string())
}
fn encode(value: &impl serde::Serialize) -> Result<String, DaemonError> {
    serde_json::to_string(value).map_err(|_| error("notification encoding failed"))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, DaemonError> {
    serde_json::from_str(value).map_err(|_| error("notification state corrupt"))
}
pub(crate) fn workflow_identity(kernel: &str, session: &str, workflow: &str) -> String {
    // JSON tuple avoids delimiter ambiguity; identity stays stable across registration toggles.
    serde_json::json!([kernel, session, workflow]).to_string()
}

pub(super) fn initialize(db: &mut Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS workflow_notification_sources (
      source_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, kernel_id TEXT NOT NULL,
      session_id TEXT NOT NULL, workflow_id TEXT NOT NULL, enabled INTEGER NOT NULL,
      payload_json TEXT NOT NULL, UNIQUE(kernel_id,session_id,workflow_id,owner_id));
    CREATE TABLE IF NOT EXISTS workflow_notification_subscriptions (
      subscription_id TEXT PRIMARY KEY, source_id TEXT NOT NULL, owner_id TEXT NOT NULL,
      target_identity TEXT NOT NULL, payload_json TEXT NOT NULL,
      UNIQUE(source_id,target_identity));
    CREATE TABLE IF NOT EXISTS workflow_notification_occurrences (
      source_id TEXT NOT NULL, occurrence_id TEXT NOT NULL, diagnostic TEXT,
      PRIMARY KEY(source_id,occurrence_id));
    CREATE TABLE IF NOT EXISTS workflow_notification_outbox (
      subscription_id TEXT NOT NULL, source_id TEXT NOT NULL, occurrence_id TEXT NOT NULL,
      deadline_ms INTEGER NOT NULL, state TEXT NOT NULL, envelope_json TEXT, retry_at_ms INTEGER NOT NULL DEFAULT 0,
      PRIMARY KEY(subscription_id,source_id,occurrence_id));
    CREATE INDEX IF NOT EXISTS idx_workflow_notification_outbox_pending
      ON workflow_notification_outbox(state,deadline_ms);
    CREATE TABLE IF NOT EXISTS workflow_notification_inbox (
      subscription_id TEXT NOT NULL, source_id TEXT NOT NULL, occurrence_id TEXT NOT NULL,
      deadline_ms INTEGER NOT NULL, state TEXT NOT NULL, envelope_json TEXT, ancestry_json TEXT NOT NULL,
      session_id TEXT NOT NULL, queued_prompt_id TEXT, retry_at_ms INTEGER NOT NULL DEFAULT 0,
      PRIMARY KEY(subscription_id,source_id,occurrence_id));
    CREATE INDEX IF NOT EXISTS idx_workflow_notification_inbox_queue
      ON workflow_notification_inbox(session_id,queued_prompt_id);") .map_err(sql)
}

pub(crate) struct SourceAdmission {
    pub source: WorkflowNotificationSource,
    pub workflow_json: String,
}
pub(crate) enum NotificationOperation {
    Register(SourceAdmission),
    Attach {
        subscription: WorkflowNotificationSubscription,
        target: WorkflowNotificationTarget,
    },
    Accept {
        subscription: WorkflowNotificationSubscription,
        envelope: WorkflowNotificationEnvelope,
    },
    Queue(Box<PreparedNotification>),
    Sweep {
        now: u64,
    },
    Retry {
        subscription_id: String,
        source_id: String,
        occurrence_id: String,
        accepted: bool,
        at: u64,
    },
}
#[derive(Debug)]
pub(crate) enum NotificationOutcome {
    Source(WorkflowNotificationSource),
    Subscription(WorkflowNotificationSubscription),
    Ack(WorkflowNotificationAck),
    Queued,
    Swept,
}
pub(super) struct NotificationRequest {
    operation: NotificationOperation,
    response: mpsc::Sender<Result<NotificationOutcome, DaemonError>>,
}
impl std::fmt::Debug for NotificationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationRequest")
            .finish_non_exhaustive()
    }
}
impl DurableKernelStateStore {
    pub(crate) fn notify(
        &self,
        operation: NotificationOperation,
    ) -> Result<NotificationOutcome, DaemonError> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::WorkflowNotification(Box::new(
                NotificationRequest {
                    operation,
                    response,
                },
            )))?;
        receiver
            .recv()
            .map_err(|_| error("notification writer unavailable"))?
    }
    pub(crate) fn notification_inventory(
        &self,
        owner: &str,
    ) -> Result<
        (
            Vec<WorkflowNotificationSource>,
            Vec<WorkflowNotificationSubscription>,
            Vec<WorkflowNotificationDiagnostic>,
        ),
        DaemonError,
    > {
        let db = self.lock_connection("workflow.notifications.inventory")?;
        let mut query = db.prepare("SELECT payload_json FROM workflow_notification_sources WHERE owner_id=?1 ORDER BY source_id LIMIT 1024").map_err(sql)?;
        let sources = query
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .map(|r| decode(&r.map_err(sql)?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut query = db.prepare("SELECT payload_json FROM workflow_notification_subscriptions WHERE owner_id=?1 ORDER BY subscription_id LIMIT 1024").map_err(sql)?;
        let subscriptions = query
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .map(|r| decode(&r.map_err(sql)?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut query = db.prepare("SELECT o.source_id,o.occurrence_id,o.diagnostic FROM workflow_notification_occurrences o JOIN workflow_notification_sources s ON s.source_id=o.source_id WHERE s.owner_id=?1 AND o.diagnostic IS NOT NULL ORDER BY o.rowid DESC LIMIT 64").map_err(sql)?;
        let diagnostics = query
            .query_map([owner], |r| {
                Ok(WorkflowNotificationDiagnostic {
                    source_id: r.get(0)?,
                    occurrence_id: r.get(1)?,
                    code: r.get(2)?,
                })
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        Ok((sources, subscriptions, diagnostics))
    }
    pub(crate) fn notification_has_pending(&self) -> Result<bool, DaemonError> {
        let db = self.lock_connection("workflow.notifications.backlog")?;
        db.query_row("SELECT EXISTS(SELECT 1 FROM workflow_notification_outbox WHERE state='pending') OR EXISTS(SELECT 1 FROM workflow_notification_inbox WHERE state='accepted')",[],|r|r.get(0)).map_err(sql)
    }
    pub(crate) fn notification_candidates(
        &self,
        accepted: bool,
        now: u64,
        limit: usize,
    ) -> Result<
        Vec<(
            WorkflowNotificationSubscription,
            WorkflowNotificationEnvelope,
        )>,
        DaemonError,
    > {
        let db = self.lock_connection("workflow.notifications.pending")?;
        let table = if accepted {
            "workflow_notification_inbox"
        } else {
            "workflow_notification_outbox"
        };
        let state = if accepted { "accepted" } else { "pending" };
        let mut q = db.prepare(&format!("SELECT s.payload_json,d.envelope_json FROM {table} d JOIN workflow_notification_subscriptions s ON s.subscription_id=d.subscription_id WHERE d.state=?1 AND d.deadline_ms>?2 AND d.retry_at_ms<=?2 ORDER BY d.retry_at_ms,d.rowid LIMIT ?3")).map_err(sql)?;
        let rows = q
            .query_map(params![state, now as i64, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(sql)?;
        rows.map(|row| {
            let (s, e) = row.map_err(sql)?;
            Ok((decode(&s)?, decode(&e)?))
        })
        .collect()
    }
}

/// Returns true only on an uncertain COMMIT: fence the shared writer before any
/// tentative session can become visible or another stale transition can commit.
pub(super) fn execute(db: &mut Connection, request: NotificationRequest) -> bool {
    let mut uncertain = false;
    let result = (|| {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let result = apply(&tx, request.operation)?;
        if let Err(e) = tx.commit() {
            uncertain = !super::storage_full::observe(&e);
            return Err(error(if uncertain {
                "notification commit unknown; restart required".into()
            } else {
                e.to_string()
            }));
        }
        Ok(result)
    })();
    let _ = request.response.send(result);
    uncertain
}
fn apply(
    tx: &Transaction<'_>,
    op: NotificationOperation,
) -> Result<NotificationOutcome, DaemonError> {
    match op {
        NotificationOperation::Register(admission) => {
            let mut source = admission.source;
            require_source_workflow(tx, &source, Some(&admission.workflow_json))?;
            // The durable workflow identity owns its original source ID and owner.
            let existing: Option<(String,String)> = tx.query_row("SELECT source_id,owner_id FROM workflow_notification_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND owner_id=?4",params![source.kernel_id,source.session_id,source.workflow_id,source.owner_user_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
            if let Some((id, owner)) = existing {
                if owner != source.owner_user_id {
                    return Err(error("source not available"));
                }
                source.source_id = id;
            } else {
                let count: i64 = tx
                    .query_row(
                        "SELECT count(*) FROM workflow_notification_sources WHERE owner_id=?1",
                        [&source.owner_user_id],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                if count >= 1024 {
                    return Err(error("workflow notification source limit"));
                }
            }
            tx.execute("INSERT INTO workflow_notification_sources VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(source_id) DO UPDATE SET enabled=excluded.enabled,payload_json=excluded.payload_json",params![source.source_id,source.owner_user_id,source.kernel_id,source.session_id,source.workflow_id,source.enabled,encode(&source)?]).map_err(sql)?;
            Ok(NotificationOutcome::Source(source))
        }
        NotificationOperation::Attach {
            mut subscription,
            target,
        } => {
            if !(1..=30).contains(&subscription.ttl_days) {
                return Err(error("TTL must be 1–30 days"));
            }
            let source = load_source(tx, &subscription.source_id)?;
            if source.owner_user_id != subscription.owner_user_id || !source.enabled {
                return Err(error("source not available"));
            }
            require_source_workflow(tx, &source, None)?;
            target
                .require_current(tx, &subscription.owner_user_id)
                .map_err(|e| error(e.to_string()))?;
            let from =
                workflow_identity(&source.kernel_id, &source.session_id, &source.workflow_id);
            let to = workflow_identity(
                &subscription.target_kernel_id,
                &subscription.session_id,
                &subscription.workflow_id,
            );
            // Recursive reachability is UX; runtime ancestry below is authoritative.
            let cycle: bool = tx.query_row("WITH RECURSIVE reachable(identity) AS (VALUES (?1) UNION SELECT s.target_identity FROM reachable r JOIN workflow_notification_sources src ON json_array(src.kernel_id,src.session_id,src.workflow_id)=r.identity JOIN workflow_notification_subscriptions s ON s.source_id=src.source_id WHERE src.enabled=1 AND s.owner_id=?3) SELECT EXISTS(SELECT 1 FROM reachable WHERE identity=?2)", params![to,from,subscription.owner_user_id], |r|r.get(0)).map_err(sql)?;
            if cycle {
                return Err(error("workflow notification cycle"));
            }
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM workflow_notification_subscriptions WHERE source_id=?1",
                    [&source.source_id],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            let existing: Option<String> = tx.query_row("SELECT payload_json FROM workflow_notification_subscriptions WHERE source_id=?1 AND target_identity=?2",params![source.source_id,to],|r|r.get(0)).optional().map_err(sql)?;
            if let Some(existing) = existing {
                let current: WorkflowNotificationSubscription = decode(&existing)?;
                if current.publication_id != subscription.publication_id
                    || current.queue_id != subscription.queue_id
                {
                    return Err(error("workflow already attached; target differs"));
                }
                subscription.subscription_id = current.subscription_id;
            } else if tx
                .query_row(
                    "SELECT count(*) FROM workflow_notification_subscriptions WHERE owner_id=?1",
                    [&subscription.owner_user_id],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(sql)?
                >= 1024
            {
                return Err(error("workflow notification subscription limit"));
            } else if count >= MAX_SUBSCRIPTIONS as i64 {
                return Err(error("notification fanout limit"));
            }
            tx.execute("INSERT INTO workflow_notification_subscriptions VALUES (?1,?2,?3,?4,?5) ON CONFLICT(subscription_id) DO UPDATE SET payload_json=excluded.payload_json",params![subscription.subscription_id,subscription.source_id,subscription.owner_user_id,to,encode(&subscription)?]).map_err(sql)?;
            Ok(NotificationOutcome::Subscription(subscription))
        }
        NotificationOperation::Accept {
            subscription,
            envelope,
        } => accept(tx, &subscription, &envelope),
        NotificationOperation::Queue(prepared) => {
            queue::commit_in(tx, &prepared)?;
            Ok(NotificationOutcome::Queued)
        }
        NotificationOperation::Retry {
            subscription_id,
            source_id,
            occurrence_id,
            accepted,
            at,
        } => {
            let table = if accepted {
                "workflow_notification_inbox"
            } else {
                "workflow_notification_outbox"
            };
            tx.execute(&format!("UPDATE {table} SET retry_at_ms=?4 WHERE subscription_id=?1 AND source_id=?2 AND occurrence_id=?3"),params![subscription_id,source_id,occurrence_id,at as i64]).map_err(sql)?;
            Ok(NotificationOutcome::Swept)
        }
        NotificationOperation::Sweep { now } => {
            // Dedupe/lineage records survive payload expiry. No active delete/transfer action.
            tx.execute("UPDATE workflow_notification_outbox SET state=CASE WHEN state='pending' THEN 'expired' ELSE state END,envelope_json=NULL WHERE deadline_ms<=?1 AND envelope_json IS NOT NULL",[now as i64]).map_err(sql)?;
            tx.execute("UPDATE workflow_notification_inbox SET state='expired' WHERE state='accepted' AND deadline_ms<=?1",[now as i64]).map_err(sql)?;
            tx.execute(
                "UPDATE workflow_notification_inbox SET envelope_json=NULL WHERE deadline_ms<=?1",
                [now as i64],
            )
            .map_err(sql)?;
            Ok(NotificationOutcome::Swept)
        }
    }
}
fn load_source(
    tx: &Transaction<'_>,
    source_id: &str,
) -> Result<WorkflowNotificationSource, DaemonError> {
    let value: Option<String> = tx
        .query_row(
            "SELECT payload_json FROM workflow_notification_sources WHERE source_id=?1",
            [source_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    decode(
        value
            .as_deref()
            .ok_or_else(|| error("source not available"))?,
    )
}
fn require_source_workflow(
    tx: &Transaction<'_>,
    source: &WorkflowNotificationSource,
    expected: Option<&str>,
) -> Result<(), DaemonError> {
    let current: Option<String> = tx.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='workflow' AND entity_id=?3",params![source.kernel_id,source.session_id,source.workflow_id],|r|r.get(0)).optional().map_err(sql)?;
    if current.is_none() || expected.is_some_and(|expected| current.as_deref() != Some(expected)) {
        return Err(error("source not available"));
    }
    Ok(())
}
fn accept(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
) -> Result<NotificationOutcome, DaemonError> {
    // This round admits only exact persisted, kernel-derived local outbox bytes.
    // A future relay adapter must supply authenticated same-owner peer authority.
    let persisted: Option<String> = tx.query_row("SELECT envelope_json FROM workflow_notification_outbox WHERE subscription_id=?1 AND source_id=?2 AND occurrence_id=?3",params![sub.subscription_id,env.source_id,env.occurrence_id],|r|r.get(0)).optional().map_err(sql)?.flatten();
    if persisted.as_deref() != Some(encode(env)?.as_str()) {
        return Err(error("notification occurrence conflict"));
    }
    let current: String = tx
        .query_row(
            "SELECT payload_json FROM workflow_notification_subscriptions WHERE subscription_id=?1",
            [&sub.subscription_id],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let current: WorkflowNotificationSubscription = decode(&current)?;
    if current.source_id != env.source_id || current.owner_user_id != sub.owner_user_id {
        return Err(error("notification owner mismatch"));
    }
    let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_notification_inbox WHERE subscription_id=?1 AND source_id=?2 AND occurrence_id=?3)",params![sub.subscription_id,env.source_id,env.occurrence_id],|r|r.get(0)).map_err(sql)?;
    let now = crate::session::unix_epoch_ms();
    if env.deadline_ms <= now {
        return Ok(NotificationOutcome::Ack(WorkflowNotificationAck::Expired));
    }
    if !duplicate {
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM workflow_notification_inbox WHERE state='accepted'",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if count >= MAX_PENDING {
            return Err(error("notification inbox full"));
        }
        tx.execute("INSERT INTO workflow_notification_inbox VALUES (?1,?2,?3,?4,'accepted',?5,?6,?7,NULL,0)",params![sub.subscription_id,env.source_id,env.occurrence_id,env.deadline_ms as i64,encode(env)?,encode(&env.ancestry)?,sub.session_id]).map_err(sql)?;
    }
    tx.execute("UPDATE workflow_notification_outbox SET state='accepted' WHERE subscription_id=?1 AND source_id=?2 AND occurrence_id=?3",params![sub.subscription_id,env.source_id,env.occurrence_id]).map_err(sql)?;
    Ok(NotificationOutcome::Ack(if duplicate {
        WorkflowNotificationAck::Duplicate
    } else {
        WorkflowNotificationAck::Accepted
    }))
}
