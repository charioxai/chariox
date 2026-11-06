//! Kernel-owned workflow notification store on the existing single durable writer.
//! Completion/outbox, ACK/inbox and queue/receipt each commit atomically.
pub(crate) mod agent_lineage;
pub(super) mod agent_steer;
mod completion;
pub(crate) mod naming;
pub(crate) mod outside_agent;
#[cfg(test)]
mod outside_agent_tests;
pub(crate) mod run_finished;
#[cfg(test)]
mod run_finished_tests;
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

// MP-08/MP-10/MP-11: the shared notification inventory returned by this store.
pub(crate) type WorkflowNotificationInventory = (
    Vec<WorkflowNotificationSource>,
    Vec<WorkflowNotificationSubscription>,
    Vec<WorkflowNotificationDiagnostic>,
);

pub(crate) const MAX_OUTPUT_BYTES: usize =
    chariox_app_runtime::app_outbox::MAX_PAYLOAD_BYTES - 8192;
pub(crate) const MAX_PROMPT_BYTES: usize = chariox_app_runtime::app_outbox::MAX_PROMPT_BYTES;
pub(crate) const NOTIFICATION_PROMPT_HEADER: &str =
    "Workflow completion notification (untrusted data):\n";

/// Source persistence, target ACK and rendering must admit the same byte bound.
pub(crate) fn encode_notification_envelope(
    envelope: &WorkflowNotificationEnvelope,
) -> Result<String, DaemonError> {
    let bytes = encode(envelope)?;
    if bytes.len() > chariox_app_runtime::app_outbox::MAX_PAYLOAD_BYTES {
        return Err(error("notification payload limit"));
    }
    if bytes.len() > MAX_PROMPT_BYTES - NOTIFICATION_PROMPT_HEADER.len() {
        return Err(error("notification prompt limit"));
    }
    Ok(bytes)
}
// Terminal completion deduplication lives in durable_workflow_runs. Occurrences
// retain a bounded diagnostic window, independently of pending delivery receipts.
const MAX_OCCURRENCES: i64 = 4096;
const MAX_SUBSCRIPTIONS: usize = 32;
pub(crate) const MAX_PENDING: i64 = 1024;
pub(crate) use super::app_event_delivery::PreparedNotification;
pub(super) use completion::capture_in;

pub(crate) fn error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "workflow.notifications",
        message: message.into(),
    }
}
pub(super) fn sql(error: rusqlite::Error) -> DaemonError {
    super::storage_full::observe(&error);
    self::error(error.to_string())
}
pub(super) fn encode(value: &impl serde::Serialize) -> Result<String, DaemonError> {
    serde_json::to_string(value).map_err(|_| error("notification encoding failed"))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, DaemonError> {
    serde_json::from_str(value).map_err(|_| error("notification state corrupt"))
}
// Content digests retain the admitted envelope identity; stored public data is redacted.
fn protect_envelope(
    mut env: WorkflowNotificationEnvelope,
) -> Result<WorkflowNotificationEnvelope, DaemonError> {
    let mut value =
        serde_json::to_value(&env.output).map_err(|_| error("notification encoding failed"))?;
    crate::secret_redaction::redact_json_secrets(&mut value);
    env.output =
        serde_json::from_value(value).map_err(|_| error("notification encoding failed"))?;
    Ok(env)
}
/// The identity a subscription's target contributes to, and is checked against, an
/// occurrence's ancestry: its workflow, or the agent it prompts.
pub(crate) fn ancestry_identity(sub: &WorkflowNotificationSubscription) -> String {
    match sub.target_kind {
        crate::local::WorkflowNotificationTargetKind::WorkflowEndpoint => {
            workflow_identity(&sub.target_kernel_id, &sub.session_id, &sub.workflow_id)
        }
        crate::local::WorkflowNotificationTargetKind::Agent => {
            format!("agent:{}", sub.endpoint_id)
        }
        crate::local::WorkflowNotificationTargetKind::OutsideAgent => {
            format!("grant:{}", sub.endpoint_id)
        }
    }
}
pub(crate) fn workflow_identity(kernel: &str, session: &str, workflow: &str) -> String {
    // JSON tuple avoids delimiter ambiguity; identity stays stable across registration toggles.
    serde_json::json!([kernel, session, workflow]).to_string()
}

pub(super) fn initialize(db: &mut Connection) -> Result<(), DaemonError> {
    // Registration and diagnostics are workflow metadata. Delivery state uses
    // the existing App automation/receipt store, not a second inbox/outbox.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_notification_exit_sources (
      source_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, kernel_id TEXT NOT NULL,
      session_id TEXT NOT NULL, workflow_id TEXT NOT NULL, exit_node_id TEXT NOT NULL,
      enabled INTEGER NOT NULL, payload_json TEXT NOT NULL,
      UNIQUE(kernel_id,session_id,workflow_id,exit_node_id,owner_id));
    CREATE TABLE IF NOT EXISTS workflow_notification_occurrences (
      source_id TEXT NOT NULL, occurrence_id TEXT NOT NULL, diagnostic TEXT,
      PRIMARY KEY(source_id,occurrence_id));
    CREATE INDEX IF NOT EXISTS workflow_notification_diagnostics
      ON workflow_notification_occurrences((diagnostic IS NOT NULL)) WHERE diagnostic IS NOT NULL;
    CREATE TABLE IF NOT EXISTS workflow_notification_source_cache (
      owner_id TEXT NOT NULL,kernel_id TEXT NOT NULL,payload_json TEXT NOT NULL,
      PRIMARY KEY(owner_id,kernel_id));
    CREATE TABLE IF NOT EXISTS workflow_notification_grants (
      owner_id TEXT NOT NULL, subscription_id TEXT NOT NULL, grant_id TEXT NOT NULL,
      PRIMARY KEY(owner_id,subscription_id));",
    )
    .map_err(sql)?;
    agent_lineage::initialize(db).map_err(sql)?;
    prune_occurrences(db).map_err(sql)?;
    Ok(())
}

fn prune_occurrences(db: &Connection) -> rusqlite::Result<()> {
    db.execute("DELETE FROM workflow_notification_occurrences WHERE rowid < (SELECT rowid FROM workflow_notification_occurrences ORDER BY rowid DESC LIMIT 1 OFFSET ?1)", [MAX_OCCURRENCES - 1])?;
    Ok(())
}

pub(crate) struct SourceAdmission {
    pub source: WorkflowNotificationSource,
    pub workflow_json: String,
}
pub(crate) enum NotificationOperation {
    Register(SourceAdmission),
    EmitOwnerAction {
        source_id: String,
        occurrence_id: String,
        output: crate::session::WorkflowOutputPayload,
        at: u64,
    },
    Attach {
        subscription: WorkflowNotificationSubscription,
        target: WorkflowNotificationTarget,
    },
    Accept {
        subscription: WorkflowNotificationSubscription,
        envelope: WorkflowNotificationEnvelope,
    },
    Queue(Box<PreparedNotification>),
    ProtectRetained {
        subscription: WorkflowNotificationSubscription,
        occurrence_id: String,
    },
    /// Attaches a subscription whose target needs no workflow resolution: an agent, or an
    /// entry point of another kernel.
    RemoteAttach {
        subscription: WorkflowNotificationSubscription,
    },
    /// The agent's prompt for an accepted occurrence is queued: the receipt is settled.
    AgentQueued {
        subscription_id: String,
        source_id: String,
        occurrence_id: String,
        prompt_id: String,
        session_id: String,
    },
    /// The receipt names the operation of the prompt about to be admitted for it.
    ReserveAgentLineage {
        receipt_id: String,
        operation_id: String,
    },
    BindAgentSteer {
        session: String,
        notification_prompt: String,
        active_prompt: String,
    },
    /// A local or remote attach made under an ephemeral external access grant.
    Granted {
        grant: String,
        attach: Box<NotificationOperation>,
    },
    /// A grant follows several sources in one commit: a limit that refuses one refuses them all.
    FollowSources {
        grant: String,
        subscriptions: Vec<WorkflowNotificationSubscription>,
    },
    /// The accepted occurrence becomes one event in the subscribed grant's own stream, and
    /// its receipt is settled, in one commit.
    AppendToGrantInbox {
        subscription: WorkflowNotificationSubscription,
        envelope: WorkflowNotificationEnvelope,
    },
    /// Disable subscriptions bound to this grant, or to any grant on restart, except the
    /// outside agent subscriptions of the grants named in `keep_outside_agent_of`.
    RetireGrants {
        grant: Option<String>,
        keep_outside_agent_of: Vec<String>,
    },
    Detach {
        subscription_id: String,
        owner: String,
        kernel: String,
    },
    Acknowledge {
        subscription_id: String,
        occurrence_id: String,
    },
    Cache {
        owner: String,
        kernel: String,
        sources: Vec<crate::local::WorkflowNotificationSourceSummary>,
    },
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
    LineageReserved,
    SteerBound,
    Protected(Option<WorkflowNotificationEnvelope>),
    Retired(Vec<WorkflowNotificationSubscription>),
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
        crate::durable_state::await_writer_response(receiver)
            .map_err(|_| error("notification writer unavailable"))?
    }
    #[allow(
        clippy::type_complexity,
        reason = "Keep the explicit notification_inventory state or return type at the existing boundary"
    )]
    pub(crate) fn notification_inventory(
        &self,
        owner: &str,
    ) -> Result<WorkflowNotificationInventory, DaemonError> {
        let db = self.lock_connection("workflow.notifications.inventory")?;
        let mut query = db.prepare("SELECT payload_json FROM workflow_notification_exit_sources WHERE owner_id=?1 ORDER BY source_id LIMIT 1024").map_err(sql)?;
        let sources = query
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .map(|r| decode(&r.map_err(sql)?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut query = db.prepare("SELECT notification_json FROM app_automations WHERE owner_id=?1 AND source_kind='workflow_completion' AND status='active' ORDER BY automation_id LIMIT 1024").map_err(sql)?;
        let subscriptions = query
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .map(|r| decode(&r.map_err(sql)?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut query = db.prepare("SELECT DISTINCT o.source_id,o.occurrence_id,o.diagnostic FROM workflow_notification_occurrences o LEFT JOIN workflow_notification_exit_sources s ON s.source_id=o.source_id LEFT JOIN app_automations a ON a.installation_id=o.source_id AND a.source_kind='workflow_completion' WHERE (s.owner_id=?1 OR a.owner_id=?1) AND o.diagnostic IS NOT NULL ORDER BY o.rowid DESC LIMIT 64").map_err(sql)?;
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
        db.query_row("SELECT EXISTS(SELECT 1 FROM app_outbox WHERE source_kind='workflow_completion' AND state IN ('accepted','retryable'))",[],|r|r.get(0)).map_err(sql)
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
        let rows = {
            let db = self.lock_connection("workflow.notifications.pending")?;
            let state = if accepted { "accepted" } else { "retryable" };
            let mut q = db.prepare("SELECT s.notification_json,d.payload_json FROM app_outbox d JOIN app_automations s ON s.owner_id=d.owner_id AND s.installation_id=d.installation_id AND s.automation_id=d.automation_id WHERE d.source_kind='workflow_completion' AND d.state=?1 AND d.expires_at_ms>?2 AND d.next_attempt_at_ms<=?2 AND s.status='active' ORDER BY d.next_attempt_at_ms,d.sequence LIMIT ?3").map_err(sql)?;
            let rows = q
                .query_map(params![state, now as i64, limit as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(sql)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(sql)?
        };
        let mut candidates = Vec::new();
        for (subscription, bytes) in rows {
            let subscription: WorkflowNotificationSubscription = decode(&subscription)?;
            let safe = protect_envelope(decode(&bytes)?)?;
            if encode(&safe)? == bytes {
                candidates.push((subscription, safe));
            } else if let NotificationOutcome::Protected(Some(safe)) =
                self.notify(NotificationOperation::ProtectRetained {
                    subscription: subscription.clone(),
                    occurrence_id: safe.occurrence_id,
                })?
            {
                candidates.push((subscription, safe));
            }
        }
        Ok(candidates)
    }
    pub(crate) fn notification_grant(
        &self,
        owner: &str,
        subscription_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let db = self.lock_connection("workflow.notifications.grant")?;
        db.query_row("SELECT grant_id FROM workflow_notification_grants WHERE owner_id=?1 AND subscription_id=?2",params![owner,subscription_id],|r|r.get(0)).optional().map_err(sql)
    }
    pub(crate) fn notification_cached_sources(
        &self,
        owner: &str,
    ) -> Result<Vec<crate::local::WorkflowNotificationSourceSummary>, DaemonError> {
        let db = self.lock_connection("notification sources")?;
        let mut q=db.prepare("SELECT payload_json FROM workflow_notification_source_cache WHERE owner_id=?1 LIMIT 128").map_err(sql)?;
        let rows = q
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(sql)?;
        let mut result = Vec::new();
        for row in rows {
            result.extend(decode::<
                Vec<crate::local::WorkflowNotificationSourceSummary>,
            >(&row.map_err(sql)?)?);
        }
        Ok(result)
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
        NotificationOperation::EmitOwnerAction {
            source_id,
            occurrence_id,
            output,
            at,
        } => {
            let source = load_source(tx, &source_id)?;
            if source.kind != crate::local::NotificationSourceKind::OwnerAction {
                return Err(error("not an owner action source"));
            }
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO workflow_notification_occurrences VALUES (?1,?2,NULL)",
                    params![source_id, occurrence_id],
                )
                .map_err(sql)?;
            if inserted != 0 {
                completion::publish_in(tx, &source, &occurrence_id, output, Vec::new(), at)?;
                prune_occurrences(tx).map_err(sql)?;
            }
            Ok(NotificationOutcome::Source(source))
        }
        NotificationOperation::Register(admission) => {
            let mut source = admission.source;
            require_source_workflow(tx, &source, Some(&admission.workflow_json))?;
            // The durable workflow identity owns its original source ID and owner.
            let existing: Option<(String,String)> = tx.query_row("SELECT source_id,owner_id FROM workflow_notification_exit_sources WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND exit_node_id=?4 AND owner_id=?5",params![source.kernel_id,source.session_id,source.workflow_id,source.exit_node_id,source.owner_user_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
            if let Some((id, owner)) = existing {
                if owner != source.owner_user_id {
                    return Err(error("source not available"));
                }
                source.source_id = id;
            } else {
                let count: i64 = tx
                    .query_row(
                        "SELECT count(*) FROM workflow_notification_exit_sources WHERE owner_id=?1",
                        [&source.owner_user_id],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                if count >= 1024 {
                    return Err(error("workflow notification source limit"));
                }
            }
            if source.kind == crate::local::NotificationSourceKind::OwnerAction {
                // Blocked-task alerts go to the Room's current owner only.
                tx.execute("UPDATE workflow_notification_exit_sources SET enabled=0,payload_json=json_set(payload_json,'$.enabled',json('false')) WHERE kernel_id=?1 AND session_id=?2 AND workflow_id=?3 AND exit_node_id=?4 AND owner_id<>?5",params![source.kernel_id,source.session_id,source.workflow_id,source.exit_node_id,source.owner_user_id]).map_err(sql)?;
            }
            tx.execute("INSERT INTO workflow_notification_exit_sources VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(source_id) DO UPDATE SET enabled=excluded.enabled,payload_json=excluded.payload_json",params![source.source_id,source.owner_user_id,source.kernel_id,source.session_id,source.workflow_id,source.exit_node_id,source.enabled,encode(&source)?]).map_err(sql)?;
            Ok(NotificationOutcome::Source(source))
        }
        NotificationOperation::Attach {
            mut subscription,
            target,
        } => {
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
            let cycle: bool = tx.query_row("WITH RECURSIVE reachable(identity) AS (VALUES (?1) UNION SELECT json_array(json_extract(s.notification_json,'$.target_kernel_id'),s.session_id,json_extract(s.notification_json,'$.workflow_id')) FROM reachable r JOIN workflow_notification_exit_sources src ON json_array(src.kernel_id,src.session_id,src.workflow_id)=r.identity JOIN app_automations s ON s.installation_id=src.source_id WHERE src.enabled=1 AND s.owner_id=?3 AND s.source_kind='workflow_completion' AND s.status='active') SELECT EXISTS(SELECT 1 FROM reachable WHERE identity=?2)", params![to,from,subscription.owner_user_id], |r|r.get(0)).map_err(sql)?;
            if cycle {
                return Err(error("workflow notification cycle"));
            }
            save_subscription(tx, &mut subscription)?;
            Ok(NotificationOutcome::Subscription(subscription))
        }
        NotificationOperation::Accept {
            subscription,
            envelope,
        } => accept(tx, &subscription, &envelope),
        NotificationOperation::Queue(prepared) => {
            super::app_event_delivery::workflow_completion::commit_in(tx, &prepared)?;
            Ok(NotificationOutcome::Queued)
        }
        NotificationOperation::RemoteAttach { mut subscription } => {
            validate_subscription(&subscription)?;
            if let Ok(source) = load_source(tx, &subscription.source_id) {
                if source.owner_user_id != subscription.owner_user_id
                    || !source.enabled
                    || source.kernel_id != subscription.source_kernel_id
                {
                    return Err(error("source not available"));
                }
                require_source_workflow(tx, &source, None)?;
            }
            save_subscription(tx, &mut subscription)?;
            Ok(NotificationOutcome::Subscription(subscription))
        }
        NotificationOperation::AgentQueued {
            subscription_id,
            source_id,
            occurrence_id,
            prompt_id,
            session_id,
        } => {
            tx.execute("UPDATE app_outbox SET state='queued',queued_prompt_id=?4,queued_session_id=?5,invocation_json=json_set(coalesce(invocation_json,'{}'),'$.ancestry',json_extract(payload_json,'$.ancestry')) WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3 AND state='accepted'",params![subscription_id,source_id,occurrence_id,prompt_id,session_id]).map_err(sql)?;
            Ok(NotificationOutcome::Queued)
        }
        NotificationOperation::ReserveAgentLineage {
            receipt_id,
            operation_id,
        } => {
            agent_lineage::reserve_in(tx, &receipt_id, &operation_id).map_err(sql)?;
            Ok(NotificationOutcome::LineageReserved)
        }
        NotificationOperation::BindAgentSteer {
            session,
            notification_prompt,
            active_prompt,
        } => {
            agent_steer::bind_in(tx, &session, &notification_prompt, &active_prompt)
                .map_err(sql)?;
            Ok(NotificationOutcome::SteerBound)
        }
        NotificationOperation::AppendToGrantInbox {
            subscription,
            envelope,
        } => outside_agent::append_in(tx, &subscription, &envelope),
        NotificationOperation::Granted { grant, attach } => {
            let outcome = apply(tx, *attach)?;
            if let NotificationOutcome::Subscription(sub) = &outcome {
                outside_agent::require_follow_room(tx, &grant, sub)?;
                tx.execute(
                    "INSERT INTO workflow_notification_grants VALUES (?1,?2,?3)",
                    params![sub.owner_user_id, sub.subscription_id, grant],
                )
                .map_err(sql)?;
            }
            Ok(outcome)
        }
        NotificationOperation::FollowSources {
            grant,
            subscriptions,
        } => {
            for subscription in subscriptions {
                apply(
                    tx,
                    NotificationOperation::Granted {
                        grant: grant.clone(),
                        attach: Box::new(NotificationOperation::RemoteAttach { subscription }),
                    },
                )?;
            }
            Ok(NotificationOutcome::Swept)
        }
        NotificationOperation::RetireGrants {
            grant,
            keep_outside_agent_of,
        } => {
            let keep = encode(&keep_outside_agent_of)?;
            let retired = {
                let mut q = tx.prepare("SELECT a.notification_json FROM app_automations a JOIN workflow_notification_grants g ON g.owner_id=a.owner_id AND g.subscription_id=a.automation_id WHERE a.source_kind='workflow_completion' AND a.status='active' AND (?1 IS NULL OR g.grant_id=?1) AND NOT (json_extract(a.notification_json,'$.target_kind')='outside_agent' AND g.grant_id IN (SELECT value FROM json_each(?2)))").map_err(sql)?;
                let rows = q
                    .query_map(params![grant, keep], |r| r.get::<_, String>(0))
                    .map_err(sql)?;
                rows.map(|r| decode(&r.map_err(sql)?))
                    .collect::<Result<Vec<_>, _>>()?
            };
            // The binding stays: a stale candidate of a retired grant never reads as owner-attached.
            tx.execute("UPDATE app_automations SET status='disabled' WHERE source_kind='workflow_completion' AND EXISTS(SELECT 1 FROM workflow_notification_grants g WHERE g.owner_id=app_automations.owner_id AND g.subscription_id=app_automations.automation_id AND (?1 IS NULL OR g.grant_id=?1)) AND NOT (json_extract(app_automations.notification_json,'$.target_kind')='outside_agent' AND EXISTS(SELECT 1 FROM workflow_notification_grants g WHERE g.owner_id=app_automations.owner_id AND g.subscription_id=app_automations.automation_id AND g.grant_id IN (SELECT value FROM json_each(?2))))",params![grant, keep]).map_err(sql)?;
            Ok(NotificationOutcome::Retired(retired))
        }
        NotificationOperation::Detach {
            subscription_id,
            owner,
            kernel,
        } => {
            tx.execute("UPDATE app_automations SET status='disabled' WHERE source_kind='workflow_completion' AND automation_id=?1 AND owner_id=?2 AND json_extract(notification_json,'$.target_kernel_id')=?3",params![subscription_id,owner,kernel]).map_err(sql)?;
            Ok(NotificationOutcome::Swept)
        }
        NotificationOperation::Acknowledge {
            subscription_id,
            occurrence_id,
        } => {
            tx.execute("UPDATE app_outbox SET state='delivered' WHERE source_kind='workflow_completion' AND automation_id=?1 AND occurrence_id=?2 AND state='retryable'",params![subscription_id,occurrence_id]).map_err(sql)?;
            Ok(NotificationOutcome::Swept)
        }
        NotificationOperation::Cache {
            owner,
            kernel,
            sources,
        } => {
            tx.execute("INSERT INTO workflow_notification_source_cache VALUES (?1,?2,?3) ON CONFLICT(owner_id,kernel_id) DO UPDATE SET payload_json=excluded.payload_json",params![owner,kernel,encode(&sources)?]).map_err(sql)?;
            Ok(NotificationOutcome::Swept)
        }
        NotificationOperation::ProtectRetained {
            subscription,
            occurrence_id,
        } => {
            // The writer derives public bytes from its current receipt, preserving the
            // original digest. A concurrent queue/expiry must never be resurrected.
            let retained: Option<String> = tx.query_row("SELECT payload_json FROM app_outbox WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND automation_id=?3 AND occurrence_id=?4 AND state IN ('accepted','retryable') AND expires_at_ms>?5",params![subscription.owner_user_id,subscription.source_id,subscription.subscription_id,occurrence_id,crate::session::unix_epoch_ms() as i64],|r|r.get(0)).optional().map_err(sql)?;
            let Some(retained) = retained else {
                return Ok(NotificationOutcome::Protected(None));
            };
            let safe = protect_envelope(decode(&retained)?)?;
            tx.execute("UPDATE app_outbox SET payload_json=?5 WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND automation_id=?3 AND occurrence_id=?4",params![subscription.owner_user_id,subscription.source_id,subscription.subscription_id,occurrence_id,encode(&safe)?]).map_err(sql)?;
            Ok(NotificationOutcome::Protected(Some(safe)))
        }
        NotificationOperation::Retry {
            subscription_id,
            source_id,
            occurrence_id,
            accepted,
            at,
        } => {
            let state = if accepted { "accepted" } else { "retryable" };
            tx.execute("UPDATE app_outbox SET next_attempt_at_ms=?4 WHERE source_kind='workflow_completion' AND automation_id=?1 AND installation_id=?2 AND occurrence_id=?3 AND state=?5",params![subscription_id,source_id,occurrence_id,at as i64,state]).map_err(sql)?;
            Ok(NotificationOutcome::Swept)
        }
        NotificationOperation::Sweep { now } => {
            prune_occurrences(tx).map_err(sql)?;
            // Routing goes on if expiry fails; the unread rows are retried by the next sweep.
            if let Err(error) =
                crate::durable_state::agent_lifecycle::outside::expire_notifications(tx, now)
            {
                crate::logging::warn_with_fields(
                    "workflow.notifications",
                    "unread notification expiry failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
            tx.execute("UPDATE app_outbox SET state=CASE WHEN state IN ('accepted','retryable') THEN 'expired' ELSE state END,payload_json=NULL WHERE source_kind='workflow_completion' AND expires_at_ms<=?1",[now as i64]).map_err(sql)?;
            tx.execute("DELETE FROM app_outbox WHERE sequence IN (SELECT sequence FROM app_outbox WHERE source_kind='workflow_completion' AND expires_at_ms<=?1 AND state NOT IN ('accepted','retryable') AND NOT EXISTS(SELECT 1 FROM durable_workflow_runs r WHERE r.status NOT IN ('Completed','Failed','Stopped') AND json_extract(r.payload_json,'$.queue_item_id')=app_outbox.queued_prompt_id) ORDER BY sequence LIMIT 256)",[now.saturating_sub(30*86_400_000) as i64]).map_err(sql)?;
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
            "SELECT payload_json FROM workflow_notification_exit_sources WHERE source_id=?1",
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
    if source.kind == crate::local::NotificationSourceKind::OwnerAction {
        return Ok(());
    }
    // A run source follows the workflow's runs, whatever its definition is now.
    let expected =
        expected.filter(|_| source.kind != crate::local::NotificationSourceKind::WorkflowRun);
    let current: Option<String> = tx.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='workflow' AND entity_id=?3",params![source.kernel_id,source.session_id,source.workflow_id],|r|r.get(0)).optional().map_err(sql)?;
    if current.is_none() || expected.is_some_and(|expected| current.as_deref() != Some(expected)) {
        return Err(error("source not available"));
    }
    Ok(())
}
fn validate_subscription(sub: &WorkflowNotificationSubscription) -> Result<(), DaemonError> {
    use crate::local::WorkflowNotificationTargetKind::{Agent, OutsideAgent, WorkflowEndpoint};
    let target_ids = match sub.target_kind {
        WorkflowEndpoint => vec![&sub.workflow_id],
        Agent => {
            if sub.agent_id.as_deref() != Some(sub.endpoint_id.as_str())
                || sub.trigger_id.as_deref() != Some(sub.publication_id.as_str())
            {
                return Err(error("notification identifier invalid"));
            }
            vec![&sub.endpoint_id]
        }
        // The grant's id is both the publication and the endpoint; it names no agent or trigger.
        OutsideAgent => {
            if sub.endpoint_id != sub.publication_id
                || sub.agent_id.is_some()
                || sub.trigger_id.is_some()
                || !sub.workflow_id.is_empty()
            {
                return Err(error("notification identifier invalid"));
            }
            vec![&sub.endpoint_id]
        }
    };
    for id in [
        &sub.subscription_id,
        &sub.source_id,
        &sub.owner_user_id,
        &sub.created_by_user_id,
        &sub.source_kernel_id,
        &sub.target_kernel_id,
        &sub.session_id,
        &sub.publication_id,
        &sub.endpoint_id,
        &sub.queue_id,
    ]
    .into_iter()
    .chain(target_ids)
    {
        if id.is_empty()
            || id.len() > 128
            || id.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(error("notification identifier invalid"));
        }
    }
    Ok(())
}
fn save_subscription(
    tx: &Transaction<'_>,
    sub: &mut WorkflowNotificationSubscription,
) -> Result<(), DaemonError> {
    validate_subscription(sub)?;
    let existing:Option<(String,String)>=tx.query_row("SELECT status,notification_json FROM app_automations WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND json_extract(notification_json,'$.target_kernel_id')=?3 AND session_id=?4 AND publication_id=?5",params![sub.owner_user_id,sub.source_id,sub.target_kernel_id,sub.session_id,sub.publication_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
    let replaces_active = existing
        .as_ref()
        .is_some_and(|(status, _)| status == "active");
    if let Some((_, ref existing)) = existing {
        sub.subscription_id = decode::<WorkflowNotificationSubscription>(existing)?.subscription_id;
    }
    let count:i64=tx.query_row("SELECT count(*) FROM app_automations WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND status='active'",params![sub.owner_user_id,sub.source_id],|r|r.get(0)).map_err(sql)?;
    let total:i64=tx.query_row("SELECT count(*) FROM app_automations WHERE owner_id=?1 AND source_kind='workflow_completion' AND status='active'",[&sub.owner_user_id],|r|r.get(0)).map_err(sql)?;
    if total >= 1024 && !replaces_active {
        return Err(error("workflow notification subscription limit"));
    }
    if count >= MAX_SUBSCRIPTIONS as i64 && !replaces_active {
        return Err(error("notification fanout limit"));
    }
    tx.execute("INSERT INTO app_automations(owner_id,installation_id,automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status,source_kind,notification_json) VALUES(?1,?2,?3,1,'workflow_completion',1,'kernel',?4,?5,?6,?7,'active','workflow_completion',?8) ON CONFLICT(owner_id,installation_id,automation_id) DO UPDATE SET notification_json=excluded.notification_json,status='active'",params![sub.owner_user_id,sub.source_id,sub.subscription_id,sub.session_id,sub.publication_id,sub.endpoint_id,sub.queue_id,encode(sub)?]).map_err(sql)?;
    // The latest attach owns the row: an owner attach unbinds, a granted one rebinds.
    tx.execute(
        "DELETE FROM workflow_notification_grants WHERE owner_id=?1 AND subscription_id=?2",
        params![sub.owner_user_id, sub.subscription_id],
    )
    .map_err(sql)?;
    Ok(())
}
pub(super) fn insert_receipt(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
    state: &str,
    now: u64,
) -> Result<(), DaemonError> {
    let bytes = encode_notification_envelope(&protect_envelope(env.clone())?)?;
    let (count,retained):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(CAST(payload_json AS BLOB))),0) FROM app_outbox WHERE source_kind='workflow_completion' AND state IN ('accepted','retryable') AND owner_id=?1 AND installation_id=?2 AND automation_id=?3",params![sub.owner_user_id,env.source_id,sub.subscription_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
    if count >= MAX_PENDING
        || retained + bytes.len() as i64
            > chariox_app_runtime::app_outbox::MAX_RETAINED_PAYLOAD_BYTES as i64
    {
        return Err(error("notification outbox full"));
    }
    insert_receipt_row(tx, sub, env, state, now)
}
// Round-1 migration preserves already accepted bytes under their original TTL.
// New admission always calls the bounded wrapper above.
pub(super) fn insert_receipt_row(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
    state: &str,
    now: u64,
) -> Result<(), DaemonError> {
    let bytes = encode(env)?;
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(bytes.as_bytes()));
    let bytes = encode(&protect_envelope(env.clone())?)?;
    let receipt_id = receipt_id(sub, env)?;
    tx.execute("INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,occurred_at_ms,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,source_kind) VALUES (?1,?2,?3,?4,1,?5,?6,'workflow_completion','kernel',?7,1,0,?8,?6,?9,?10,1,0,0,'workflow_completion')",params![sub.owner_user_id,env.source_id,receipt_id,sub.subscription_id,env.occurrence_id,now as i64,digest,bytes,env.deadline_ms as i64,state]).map_err(sql)?;
    Ok(())
}
/// Admission and queue commit both require the exact subscription to be active.
pub(crate) fn require_attached(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    source_id: &str,
) -> Result<(), DaemonError> {
    let current:Option<String>=tx.query_row("SELECT notification_json FROM app_automations WHERE source_kind='workflow_completion' AND automation_id=?1 AND owner_id=?2 AND installation_id=?3 AND status='active'",params![sub.subscription_id,sub.owner_user_id,source_id],|r|r.get(0)).optional().map_err(sql)?;
    let current: WorkflowNotificationSubscription = decode(
        current
            .as_deref()
            .ok_or_else(|| error("notification not attached"))?,
    )?;
    if current != *sub {
        return Err(error("notification subscription changed"));
    }
    Ok(())
}
fn accept(
    tx: &Transaction<'_>,
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
) -> Result<NotificationOutcome, DaemonError> {
    require_attached(tx, sub, &env.source_id)?;
    let bytes = encode_notification_envelope(env)?;
    let now = crate::session::unix_epoch_ms();
    if env.deadline_ms <= now {
        return Ok(NotificationOutcome::Ack(WorkflowNotificationAck::Expired));
    }
    // Occurrence deadlines are immutable: the fixed retention plus a clock-skew allowance.
    if env.deadline_ms > now.saturating_add(crate::local::NOTIFICATION_RETENTION_MS + 300_000)
        || env.ancestry.len() > 256
        || env.ancestry.iter().any(|id| id.len() > 512)
    {
        return Err(error("notification envelope invalid"));
    }
    let identity = ancestry_identity(sub);
    if env.ancestry.contains(&identity) {
        tx.execute("INSERT INTO workflow_notification_occurrences VALUES (?1,?2,'workflow_notification_loop_dropped') ON CONFLICT(source_id,occurrence_id) DO UPDATE SET diagnostic=excluded.diagnostic",params![env.source_id,env.occurrence_id]).map_err(sql)?;
        tx.execute("UPDATE app_outbox SET state='failed' WHERE source_kind='workflow_completion' AND automation_id=?1 AND occurrence_id=?2",params![sub.subscription_id,env.occurrence_id]).map_err(sql)?;
        return Ok(NotificationOutcome::Ack(
            WorkflowNotificationAck::LoopDropped,
        ));
    }
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(bytes.as_bytes()));
    let existing:Option<(String,String,Option<String>)>=tx.query_row("SELECT state,content_digest,payload_json FROM app_outbox WHERE source_kind='workflow_completion' AND owner_id=?1 AND installation_id=?2 AND automation_id=?3 AND occurrence_id=?4",params![sub.owner_user_id,env.source_id,sub.subscription_id,env.occurrence_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let duplicate = if let Some((state, stored, public_bytes)) = existing {
        // Admit the exact retained public representation, or the original replay
        // proof. Never normalize an incoming mutated raw replay before comparison.
        if stored != digest && public_bytes.as_deref() != Some(bytes.as_str()) {
            return Err(error("notification occurrence conflict"));
        }
        if state == "retryable" {
            tx.execute("UPDATE app_outbox SET state='accepted',next_attempt_at_ms=0 WHERE source_kind='workflow_completion' AND automation_id=?1 AND occurrence_id=?2",params![sub.subscription_id,env.occurrence_id]).map_err(sql)?;
            false
        } else {
            true
        }
    } else {
        insert_receipt(tx, sub, env, "accepted", now)?;
        false
    };
    Ok(NotificationOutcome::Ack(if duplicate {
        WorkflowNotificationAck::Duplicate
    } else {
        WorkflowNotificationAck::Accepted
    }))
}

pub(crate) fn receipt_id(
    sub: &WorkflowNotificationSubscription,
    env: &WorkflowNotificationEnvelope,
) -> Result<String, DaemonError> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "wf_receipt_{:x}",
        Sha256::digest(
            encode(&(
                sub.subscription_id.as_str(),
                env.source_id.as_str(),
                env.occurrence_id.as_str()
            ))?
            .as_bytes()
        )
    ))
}

impl DurableKernelStateStore {
    pub(crate) fn agent_notification_ancestry(
        &self,
        session: &str,
        prompt: &str,
    ) -> Result<Option<Vec<String>>, DaemonError> {
        let db = self.lock_connection("workflow.notifications.ancestry")?;
        let mut query = db.prepare(
            "SELECT CASE source_kind WHEN 'workflow_completion' THEN json_extract(invocation_json,'$.ancestry') ELSE '[]' END FROM app_outbox WHERE source_kind IN ('workflow_completion','app_event') AND queued_session_id=?1 AND (queued_prompt_id=?2 OR json_extract(invocation_json,'$.steered_into_prompt_id')=?2)",
        ).map_err(sql)?;
        let rows = query
            .query_map(params![session, prompt], |row| {
                row.get::<_, Option<String>>(0)
            })
            .map_err(sql)?;
        let mut found = false;
        let mut ancestry = Vec::new();
        for row in rows {
            if let Some(value) = row.map_err(sql)? {
                found = true;
                for identity in decode::<Vec<String>>(&value)? {
                    if !ancestry.contains(&identity) {
                        ancestry.push(identity);
                    }
                }
            }
        }
        Ok(found.then_some(ancestry))
    }

    /// The prompt already admitted for a receipt that is still waiting for settlement.
    pub(crate) fn admitted_agent_notification_prompt(
        &self,
        receipt_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let db = self.lock_connection("workflow.notifications.admitted")?;
        db.query_row(
            "SELECT queued_prompt_id FROM app_outbox WHERE receipt_id=?1
               AND source_kind IN ('workflow_completion','app_event')
               AND state IN ('accepted','retryable')",
            [receipt_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(sql)
    }

    pub(crate) fn reserve_agent_notification_lineage(
        &self,
        receipt_id: &str,
        operation_id: &str,
    ) -> Result<(), DaemonError> {
        match self.notify(NotificationOperation::ReserveAgentLineage {
            receipt_id: receipt_id.into(),
            operation_id: operation_id.into(),
        })? {
            NotificationOutcome::LineageReserved => Ok(()),
            _ => Err(error("unexpected notification lineage result")),
        }
    }

    pub(crate) fn bind_agent_notification_steer(
        &self,
        session: &str,
        notification_prompt: &str,
        active_prompt: &str,
    ) -> Result<(), DaemonError> {
        match self.notify(NotificationOperation::BindAgentSteer {
            session: session.into(),
            notification_prompt: notification_prompt.into(),
            active_prompt: active_prompt.into(),
        })? {
            NotificationOutcome::SteerBound => Ok(()),
            _ => Err(error("unexpected notification steer result")),
        }
    }
}
