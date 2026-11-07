//! Entry adapters and bounded background work. No worker runs under the session
//! or durable transition lock; only acceptance and committed queue release do.
use super::*;
use crate::durable_state::workflow_gate::{self as journal, Admission, Entry, Operation, Outcome};
use base64::engine::general_purpose::STANDARD as BASE64;
use chariox_app_runtime::workflow_gate as gate;
use std::os::unix::fs::DirBuilderExt;

struct WorkerSlot;
impl Drop for WorkerSlot {
    fn drop(&mut self) {
        GATE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}
static GATE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn reserve_worker() -> Result<WorkerSlot, gate::Failure> {
    GATE_WORKERS
        .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < 2).then_some(count + 1)
        })
        .map(|_| WorkerSlot)
        .map_err(|_| gate::Failure::GateRuntimeUnavailable)
}
fn evaluate(
    component: &[u8],
    evaluation: gate::Evaluation,
    validate_only: bool,
    slot: Option<WorkerSlot>,
) -> gate::Report {
    let run = (|| {
        let _slot = slot.map(Ok).unwrap_or_else(reserve_worker)?;
        let root = crate::config::default_state_dir().join("workflow-gates");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .map_err(|_| gate::Failure::GateRuntimeUnavailable)?;
        let scratch = root.join(format!("eval-{:032x}", rand::random::<u128>()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&scratch)
            .map_err(|_| gate::Failure::GateRuntimeUnavailable)?;
        let result = (|| {
            let runtime = chariox_app_runtime::worker_process::wasm::WasmRuntime::beside_kernel()
                .map_err(|_| gate::Failure::GateRuntimeUnavailable)?;
            let launch = gate::Launch::new(validate_only, component, evaluation);
            chariox_app_runtime::worker_process::gate::evaluate_blocking(
                runtime, &launch, component, &scratch,
            )
        })();
        let removed = std::fs::remove_dir(&scratch);
        if removed.is_err() {
            return Err(gate::Failure::GateWorkerFailed);
        }
        result
    })();
    run.unwrap_or_else(|failure| gate::Report {
        outcome: Err(failure),
        fuel_consumed: 0,
        execution_ms: 0,
        profile: gate::PROFILE.into(),
    })
}

impl KernelRuntimeState {
    pub(super) async fn workflow_gate_request(
        &self,
        request: crate::local::WorkflowGateRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_current_external_command()?;
        let session = self
            .owned
            .session_store
            .read()
            .get_session(&request.session_id)?;
        if !self
            .owned
            .acts_for_owner_in(session.owner_user_id(), caller, session.owner_user_id())
        {
            return Err(journal::error("workflow gate requires the Room owner"));
        }
        let workflow = self
            .owned
            .session_store
            .read()
            .resolve_workflow_ref(session.id(), &request.workflow_ref)?;
        let owner = session.owner_user_id().to_owned();
        match request.action {
            crate::local::WorkflowGateAction::Entries { after, limit } => {
                let (entries, next_cursor) = self.owned.durable_state_store.gate_entries(
                    &owner,
                    session.id(),
                    workflow.id(),
                    after.as_deref(),
                    limit,
                )?;
                Ok(LocalDaemonResponse::WorkflowGateEntries {
                    entries,
                    next_cursor,
                })
            }
            crate::local::WorkflowGateAction::State { binding_id } => {
                if binding_id.as_ref().is_some_and(|id| id.len() > 256) {
                    return Err(journal::error("gate binding identity exceeds limit"));
                }
                Ok(LocalDaemonResponse::WorkflowGateState {
                    snapshot: self.owned.durable_state_store.gate_state(
                        &owner,
                        session.id(),
                        workflow.id(),
                        binding_id.as_deref(),
                    )?,
                })
            }
            crate::local::WorkflowGateAction::ResetState { reason } => {
                self.owned.durable_state_store.gate(Operation::ResetState {
                    owner: owner.clone(),
                    session: session.id().into(),
                    workflow: workflow.id().into(),
                    actor: caller.into(),
                    reason,
                })?;
                self.owned.app_control.event_pump().wake();
                Ok(LocalDaemonResponse::WorkflowGateState {
                    snapshot: self.owned.durable_state_store.gate_state(
                        &owner,
                        session.id(),
                        workflow.id(),
                        None,
                    )?,
                })
            }
            crate::local::WorkflowGateAction::PutModule {
                wasm_base64,
                sha256,
            } => {
                if wasm_base64.len() > 2 * 1024 * 1024 * 4 / 3 + 4 {
                    return Err(journal::error("gate module exceeds 2 MiB"));
                }
                let component = BASE64
                    .decode(wasm_base64)
                    .map_err(|_| journal::error("gate module base64 invalid"))?;
                if gate::sha256(&component) != sha256 {
                    return Err(journal::error("gate module digest mismatch"));
                }
                let now = crate::session::unix_epoch_ms();
                let evaluation = gate::Evaluation {
                    input: gate::Input {
                        abi: gate::ABI.into(),
                        entry_id: "validate".into(),
                        endpoint_id: "validate".into(),
                        allowed_entry_nodes: vec!["validate".into()],
                        binding_id: "validate".into(),
                        phase: gate::Phase::Trigger,
                        accepted_at_ms: now,
                        evaluation_at_ms: now,
                        deadline_ms: now + 60_000,
                        trigger: Some(gate::Trigger {
                            kind: gate::TriggerKind::Manual,
                            occurrence_id: "validate".into(),
                            occurred_at_ms: None,
                            data: None,
                            data_schema: None,
                            prompt: None,
                            artifacts: vec![],
                        }),
                        batch: None,
                    },
                    configuration: gate::Configuration::new(
                        "validation".into(),
                        Default::default(),
                    )
                    .map_err(|e| journal::error(e.to_string()))?,
                    state: gate::State::default(),
                };
                let store = self.owned.durable_state_store.clone();
                let digest = sha256.clone();
                self.run_workflow_compiler_operation(move || {
                    evaluate(&component, evaluation, true, None)
                        .outcome
                        .map_err(|e| journal::error(e.to_string()))?;
                    store.gate(Operation::Put {
                        owner,
                        digest,
                        component,
                    })?;
                    Ok(())
                })
                .await?;
                Ok(LocalDaemonResponse::WorkflowGateModuleStored { sha256 })
            }
            crate::local::WorkflowGateAction::Test {
                module_sha256,
                evaluation,
            } => {
                let component = self
                    .owned
                    .durable_state_store
                    .gate_module(&owner, &module_sha256)?;
                let report = self
                    .run_workflow_compiler_operation(move || {
                        Ok(evaluate(&component, evaluation, false, None))
                    })
                    .await?;
                Ok(LocalDaemonResponse::WorkflowGateTested { report })
            }
            crate::local::WorkflowGateAction::Entry { entry_id } => {
                let entry = self
                    .owned
                    .durable_state_store
                    .gate_entry(&owner, &entry_id)?;
                if entry.session_id != session.id() || entry.workflow_id != workflow.id() {
                    return Err(journal::error("gate entry not found"));
                }
                Ok(LocalDaemonResponse::WorkflowEntryAccepted { entry })
            }
            crate::local::WorkflowGateAction::Cancel { entry_id, reason } => {
                let entry = self
                    .owned
                    .durable_state_store
                    .gate_entry(&owner, &entry_id)?;
                if entry.session_id != session.id()
                    || entry.workflow_id != workflow.id()
                    || reason.len() > 512
                {
                    return Err(journal::error(
                        "gate entry not found or cancellation reason too long",
                    ));
                }
                let Outcome::Entry(entry) =
                    self.owned.durable_state_store.gate(Operation::Cancel {
                        owner,
                        id: entry_id,
                        reason,
                    })?
                else {
                    return Err(journal::error("gate cancellation response invalid"));
                };
                Ok(LocalDaemonResponse::WorkflowEntryAccepted { entry })
            }
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn accept_gate_prompt(&self, admission: Admission) -> Result<Entry, DaemonError> {
        let _guard = self.begin_managed_activity_admission()?;
        let Outcome::Entry(entry) = self
            .durable_state_store
            .gate(Operation::Accept(Box::new(admission)))?
        else {
            return Err(journal::error("gate acceptance response invalid"));
        };
        self.app_control.event_pump().wake();
        Ok(entry)
    }
    /// At most eight evaluations/pass, one per workflow at a time. The existing
    /// App event pump owns the sole pass reservation and timer interest.
    pub(super) fn pump_workflow_gates(&self) -> std::collections::BTreeSet<String> {
        let mut sessions_to_dispatch = std::collections::BTreeSet::new();
        for _ in 0..8 {
            let Ok(_guard) = self.begin_managed_activity_admission() else {
                break;
            };
            let Ok(slot) = reserve_worker() else {
                break;
            };
            let now = crate::session::unix_epoch_ms();
            let Ok(Outcome::Claim(Some(claim))) =
                self.durable_state_store.gate(Operation::Claim { now })
            else {
                break;
            };
            let claim = *claim;
            if claim
                .stored
                .admission
                .notification
                .as_ref()
                .is_some_and(|sub| !self.notification_grant_live(sub))
            {
                let _ = self.durable_state_store.gate(Operation::Cancel {
                    owner: claim.stored.admission.owner.clone(),
                    id: claim.stored.entry.entry_id.clone(),
                    reason: "gate_notification_grant_retired".into(),
                });
                continue;
            }
            let report = evaluate(&claim.component, claim.evaluation, false, Some(slot));
            let complete = || {
                self.durable_state_store.gate(Operation::Complete {
                    id: claim.stored.entry.entry_id.clone(),
                    token: claim.token,
                    report,
                    now: crate::session::unix_epoch_ms(),
                })
            };
            if let Some(sub) = &claim.stored.admission.notification {
                let result = self.with_live_notification_grant(sub, |require_live| {
                    require_live()?;
                    complete()
                });
                if result.is_err() && !self.notification_grant_live(sub) {
                    let _ = self.durable_state_store.gate(Operation::Cancel {
                        owner: claim.stored.admission.owner.clone(),
                        id: claim.stored.entry.entry_id.clone(),
                        reason: "gate_notification_grant_retired".into(),
                    });
                }
            } else {
                let _ = complete();
            }
        }
        if let Ok(waiting) = self.durable_state_store.gate_waiting() {
            for stored in waiting {
                let _ = self
                    .durable_state_store
                    .with_workflow_runtime_transition_lock(|| {
                        let _guard = self.begin_managed_activity_admission()?;
                        let mut service = self.session_store.write();
                        let before = service.get_session(&stored.entry.session_id)?;
                        let queue = before
                            .workflow_prompt_queues()
                            .iter()
                            .find(|q| q.id() == stored.admission.template.queue_id())
                            .ok_or_else(|| journal::error("gate queue deleted"))?;
                        if !queue.enabled()
                            || before
                                .workflow_queued_prompts()
                                .iter()
                                .filter(|q| {
                                    q.status() == crate::session::WorkflowQueuedPromptStatus::Queued
                                })
                                .count()
                                >= 1024
                        {
                            return Ok(());
                        }
                        let mut after = before.clone();
                        after.enqueue_workflow_prompt(
                            stored
                                .admission
                                .template
                                .clone()
                                .with_gate_pass(stored.entry.entry_id.clone(), stored.routes),
                        );
                        let release = || {
                            self.durable_state_store.gate(Operation::Release {
                                id: stored.entry.entry_id.clone(),
                                before: Box::new(before),
                                after: Box::new(after.clone()),
                            })
                        };
                        if let Some(sub) = &stored.admission.notification {
                            self.with_live_notification_grant(sub, |require_live| {
                                require_live()?;
                                release()
                            })?;
                        } else {
                            release()?;
                        }

                        let entry = self
                            .durable_state_store
                            .gate_entry(&stored.admission.owner, &stored.entry.entry_id)?;
                        if entry.disposition == journal::Disposition::Queued {
                            service.restore_session(after);
                            self.begin_managed_activity_mutation().record();
                            sessions_to_dispatch.insert(stored.entry.session_id);
                        }
                        Ok::<_, DaemonError>(())
                    });
            }
        }
        // A released prompt is already durable and in the live session; publish the
        // projection like every other enqueue, or the queue view lags one release.
        for session_id in &sessions_to_dispatch {
            let _ = self.session_snapshot(session_id);
        }
        sessions_to_dispatch
    }
}
