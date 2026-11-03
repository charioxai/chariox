use super::*;
use crate::session::{
    WorkflowDefinition, WorkflowEndpointDefinition, WorkflowQueuedPromptInput,
    WorkflowQueuedPromptSource,
};
use serde_json::json;
struct Fixture {
    db: Connection,
    admission: Admission,
    now: u64,
}
impl Fixture {
    fn new() -> Self {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE durable_workflow_hot_entities(owner_id TEXT,session_id TEXT,entity_kind TEXT,entity_id TEXT,payload_json TEXT);").unwrap();
        initialize(&db).unwrap();
        db.execute("INSERT INTO durable_workflow_hot_entities VALUES('kernel','session','queue','default','{\"workflow_id\":\"workflow\"}')",[]).unwrap();
        let definition = gate::Definition {
            abi: gate::ABI.into(),
            module_sha256: gate::sha256(b"test-component"),
            config: Default::default(),
            config_sha256: gate::sha256(b"{}"),
        };
        let mut workflow = WorkflowDefinition::new("workflow", None);
        workflow.set_gate(Some(definition.clone()));
        workflow.add_endpoint(WorkflowEndpointDefinition::new("endpoint", None, "triage"));
        db.execute("INSERT INTO durable_workflow_hot_entities VALUES('kernel','session','workflow','workflow',?1)",[encode(&workflow).unwrap()]).unwrap();
        db.execute(
            "INSERT INTO workflow_gate_modules VALUES(?1,?2,?3,0)",
            params![
                crate::session::DEFAULT_LOCAL_USER_ID,
                definition.module_sha256,
                b"test-component"
            ],
        )
        .unwrap();
        let now = crate::session::unix_epoch_ms();
        let template = WorkflowQueuedPrompt::new(WorkflowQueuedPromptInput {
            id: "queued".into(),
            queue_id: "default".into(),
            workflow_id: "workflow".into(),
            endpoint_id: "endpoint".into(),
            prompt: Some("original advisory prompt".into()),
            publication_invocation: None,
            source: WorkflowQueuedPromptSource::Manual,
            schedule_id: None,
        });
        let admission = Admission {
            kernel_id: "kernel".into(),
            session_id: "session".into(),
            owner: crate::session::DEFAULT_LOCAL_USER_ID.into(),
            workflow_id: "workflow".into(),
            workflow_revision: workflow.revision(),
            definition,
            binding_id: "binding".into(),
            dedupe_id: "occurrence".into(),
            caller_sha256: gate::sha256(b"authenticated local caller"),
            deadline_ms: now + 120_000,
            trigger: gate::Trigger {
                kind: gate::TriggerKind::Manual,
                occurrence_id: "occurrence".into(),
                occurred_at_ms: Some(now),
                data: Some(json!({"severity":"high"}).as_object().unwrap().clone()),
                data_schema: None,
                prompt: Some("original advisory prompt".into()),
                artifacts: vec![],
            },
            allowed_nodes: vec!["triage".into()],
            template,
            source_receipt: None,
            notification: None,
        };
        Self { db, admission, now }
    }
    fn accept(&mut self) -> Entry {
        let tx = self.db.transaction().unwrap();
        let result = admission::accept_in(&tx, self.admission.clone()).unwrap();
        tx.commit().unwrap();
        self.now = self.now.max(result.accepted_at_ms);
        result
    }
    fn claim(&mut self, now: u64) -> Option<Claim> {
        let tx = self.db.transaction().unwrap();
        let result = decisions::claim(&tx, now).unwrap();
        tx.commit().unwrap();
        result
    }
    fn complete(&mut self, c: Claim, decision: gate::Decision) -> Entry {
        let report = gate::Report {
            outcome: Ok(gate::Output {
                decision,
                state: gate::State {
                    workflow: json!({"count":c.stored.claimed_revision+1})
                        .as_object()
                        .unwrap()
                        .clone(),
                    binding: Default::default(),
                },
                reason: "policy".into(),
                logs: vec![],
            }),
            fuel_consumed: 5,
            execution_ms: 1,
            profile: gate::PROFILE.into(),
        };
        let tx = self.db.transaction().unwrap();
        let result =
            decisions::complete(&tx, &c.stored.entry.entry_id, &c.token, report, self.now).unwrap();
        tx.commit().unwrap();
        result
    }
}
#[test]
fn exact_replay_cannot_repeat_state_and_changed_content_conflicts() {
    let mut f = Fixture::new();
    let first = f.accept();
    let c = f.claim(f.now + 1).unwrap();
    assert!(f.claim(f.now + 1).is_none());
    let entry = f.complete(c, gate::Decision::Drop);
    assert_eq!(entry.disposition, Disposition::Dropped);
    assert_eq!(f.accept().entry_id, first.entry_id);
    assert!(f.claim(f.now + 2).is_none());
    let revisions: i64 =
        f.db.query_row(
            "SELECT revision FROM workflow_gate_state WHERE binding_id=''",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revisions, 1);
    f.admission.trigger.prompt = Some("changed".into());
    let tx = f.db.transaction().unwrap();
    assert!(admission::accept_in(&tx, f.admission).is_err());
}
#[test]
fn deferral_uses_current_owned_state_and_retains_original_configuration() {
    let mut f = Fixture::new();
    let first = f.accept();
    let c = f.claim(f.now + 1).unwrap();
    let due = f.now + 1000;
    assert_eq!(
        f.complete(c, gate::Decision::DeferUntil { at_ms: due })
            .next_due_at_ms,
        Some(due)
    );
    assert!(f.claim(due - 1).is_none());
    f.admission.dedupe_id = "second".into();
    f.admission.trigger.occurrence_id = "second".into();
    f.accept();
    let c = f.claim(f.now + 2).unwrap();
    f.complete(c, gate::Decision::Drop);
    let resumed = f.claim(due).unwrap();
    assert_eq!(resumed.stored.entry.entry_id, first.entry_id);
    assert_eq!(resumed.evaluation.state.workflow["count"], 2);
    assert_eq!(
        resumed.evaluation.configuration.config,
        f.admission.definition.config
    );
}
#[test]
fn unknown_target_never_commits_a_state_change_or_dispatch() {
    let mut f = Fixture::new();
    f.accept();
    let c = f.claim(f.now + 1).unwrap();
    let entry = f.complete(
        c,
        gate::Decision::Run {
            targets: vec![gate::Route {
                node_id: "foreign".into(),
                payload: json!({}),
                prompt: None,
            }],
        },
    );
    assert_eq!(entry.disposition, Disposition::Failed);
    assert_eq!(entry.failure.as_deref(), Some("gate_invalid_target"));
    let count: i64 =
        f.db.query_row("SELECT count(*) FROM workflow_gate_state", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn restart_terminalizes_claim_instead_of_executing_it_twice() {
    let mut f = Fixture::new();
    let entry = f.accept();
    f.claim(f.now + 1).unwrap();
    initialize(&f.db).unwrap();
    assert!(f.claim(f.now + 2).is_none());
    let stored = load(&f.db, &entry.entry_id).unwrap().unwrap();
    assert_eq!(stored.entry.failure.as_deref(), Some("gate_interrupted"));
    let count: i64 =
        f.db.query_row("SELECT count(*) FROM workflow_gate_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}
#[test]
fn one_batch_flush_retains_members_and_never_reopens_the_group() {
    let mut f = Fixture::new();
    let first = f.accept();
    let c = f.claim(f.now + 1).unwrap();
    let due = f.now + 1000;
    f.complete(
        c,
        gate::Decision::Batch {
            key: "digest".into(),
            flush_at_ms: due,
            payload: json!({"id":1}),
        },
    );
    f.admission.dedupe_id = "second".into();
    f.admission.trigger.occurrence_id = "second".into();
    f.accept();
    let c = f.claim(f.now + 2).unwrap();
    f.complete(
        c,
        gate::Decision::Batch {
            key: "digest".into(),
            flush_at_ms: due,
            payload: json!({"id":2}),
        },
    );
    assert!(f.claim(due - 1).is_none());
    let flush = f.claim(due).unwrap();
    assert_eq!(
        flush.evaluation.input.batch.as_ref().unwrap().items.len(),
        2
    );
    f.complete(flush, gate::Decision::Drop);
    assert_eq!(
        load(&f.db, &first.entry_id)
            .unwrap()
            .unwrap()
            .entry
            .disposition,
        Disposition::Dropped
    );
    assert!(f.claim(due + 1).is_none());
}

#[test]
fn queue_waiters_expire_without_another_guest_evaluation() {
    let mut f = Fixture::new();
    let entry = f.accept();
    let claim = f.claim(f.now).unwrap();
    let waiting = f.complete(
        claim,
        gate::Decision::Run {
            targets: vec![gate::Route {
                node_id: "triage".into(),
                payload: json!({}),
                prompt: Some("filtered".into()),
            }],
        },
    );
    assert_eq!(waiting.disposition, Disposition::WaitingForQueue);
    assert!(f.claim(entry.deadline_ms).is_none());
    let stored = load(&f.db, &entry.entry_id).unwrap().unwrap();
    assert_eq!(stored.entry.disposition, Disposition::Expired);
    assert_eq!(stored.entry.evaluation_count, 1);
}

#[test]
fn explicit_state_reset_cancels_pending_entries_and_records_actor_reason() {
    let mut f = Fixture::new();
    f.db.execute_batch("CREATE TABLE durable_state_events(event_id TEXT,kind TEXT,subject_id TEXT,timestamp_ms INTEGER,payload_json TEXT);").unwrap();
    let entry = f.accept();
    let claim = f.claim(f.now).unwrap();
    f.complete(
        claim,
        gate::Decision::DeferUntil {
            at_ms: f.now + 1000,
        },
    );
    let tx = f.db.transaction().unwrap();
    state::reset(
        &tx,
        &f.admission.owner,
        &f.admission.session_id,
        &f.admission.workflow_id,
        "configuration intentionally changed",
        "owner",
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        load(&f.db, &entry.entry_id)
            .unwrap()
            .unwrap()
            .entry
            .disposition,
        Disposition::Cancelled
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM workflow_gate_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let audit: String =
        f.db.query_row(
            "SELECT payload_json FROM durable_state_events WHERE kind='workflow.gate.state_reset'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(audit.contains("configuration intentionally changed"));
    assert!(audit.contains("owner"));
}

#[test]
fn terminal_replay_compacts_without_losing_decisions_or_dedupe_and_pending_survives() {
    let mut f = Fixture::new();
    let original = f.accept();
    let c = f.claim(f.now).unwrap();
    f.complete(c, gate::Decision::Drop);
    f.admission.dedupe_id = "still-pending".into();
    f.admission.trigger.occurrence_id = "still-pending".into();
    let pending = f.accept();
    let tx = f.db.transaction().unwrap();
    retention::sweep(&tx, f.now + 8 * 86_400_000).unwrap();
    tx.commit().unwrap();
    let summary = entry(&f.db, &original.entry_id).unwrap().unwrap();
    assert!(summary.replay_compacted_at_ms.is_some());
    assert_eq!(summary.disposition, Disposition::Dropped);
    let json: String =
        f.db.query_row(
            "SELECT body_json FROM workflow_gate_decisions WHERE entry_id=?1",
            [&original.entry_id],
            |r| r.get(0),
        )
        .unwrap();
    let record: DecisionRecord = decode(&json).unwrap();
    assert!(record.evaluation.is_none());
    assert!(record.report.is_none());
    assert_eq!(record.compacted.unwrap().decision, "drop");
    assert_eq!(
        load(&f.db, &pending.entry_id)
            .unwrap()
            .unwrap()
            .entry
            .disposition,
        Disposition::Ready
    );
    f.admission.dedupe_id = "occurrence".into();
    f.admission.trigger.occurrence_id = "occurrence".into();
    assert_eq!(f.accept().entry_id, original.entry_id);
    let tx = f.db.transaction().unwrap();
    retention::sweep(&tx, f.now + 31 * 86_400_000).unwrap();
    tx.commit().unwrap();
    assert!(entry(&f.db, &original.entry_id).unwrap().is_none());
    assert!(entry(&f.db, &pending.entry_id).unwrap().is_some());
}

#[test]
fn admission_charges_batch_payload_copies_to_workflow_body_budget() {
    let mut f = Fixture::new();
    let original = f.accept();
    f.db.execute(
        "INSERT INTO workflow_gate_members VALUES('retained-group',?1,?2)",
        params![original.entry_id, "x".repeat(16 * 1024 * 1024)],
    )
    .unwrap();
    f.admission.dedupe_id = "next".into();
    f.admission.trigger.occurrence_id = "next".into();
    let tx = f.db.transaction().unwrap();
    assert!(admission::accept_in(&tx, f.admission.clone()).is_err());
}

#[test]
fn a_closed_batch_key_can_start_its_next_fixed_window() {
    let mut f = Fixture::new();
    f.accept();
    let c = f.claim(f.now).unwrap();
    let due = f.now + 1000;
    assert_eq!(
        f.complete(
            c,
            gate::Decision::Batch {
                key: "daily".into(),
                flush_at_ms: due,
                payload: json!({"first":true})
            }
        )
        .disposition,
        Disposition::Batched
    );
    let c = f.claim(due).unwrap();
    f.complete(c, gate::Decision::Drop);
    f.admission.dedupe_id = "next-window".into();
    f.admission.trigger.occurrence_id = "next-window".into();
    f.accept();
    let c = f.claim(due + 1).unwrap();
    assert_eq!(
        f.complete(
            c,
            gate::Decision::Batch {
                key: "daily".into(),
                flush_at_ms: due + 1000,
                payload: json!({"second":true})
            }
        )
        .disposition,
        Disposition::Batched
    );
    let c = f.claim(due + 1000).unwrap();
    assert_eq!(c.evaluation.input.batch.as_ref().unwrap().items.len(), 1);
}

#[test]
fn passed_notification_preserves_authority_deadline_and_ancestry() {
    let f = Fixture::new();
    let deadline = f.now + 60_000;
    let q=WorkflowQueuedPrompt::new(WorkflowQueuedPromptInput{
        id:"notification".into(),queue_id:"default".into(),workflow_id:"workflow".into(),endpoint_id:"endpoint".into(),prompt:Some("original secret-free source text".into()),source:WorkflowQueuedPromptSource::Event,schedule_id:None,
        publication_invocation:Some(crate::session::WorkflowPublicationInvocationEnvelope{
            http_method:None,publication_id:"publication".into(),hook_id:Some("subscription".into()),invocation_id:"receipt".into(),transport:"workflow_notification".into(),endpoint_id:"endpoint".into(),queue_ref:Some("default".into()),input:json!({"deadline_ms":deadline,"ancestry":["source"],"payload":{"original":true}}),artifacts:vec![],mode:None,caller:json!({"owner_id":"local","delivery_mode":"inject"}),
        }),
    }).with_gate_pass("gate-entry".into(),vec![gate::Route{node_id:"triage".into(),payload:json!({"passed":true}),prompt:None}]);
    assert_eq!(q.notification_deadline_ms(), Some(deadline));
    assert!(!q.notification_expired_at(f.now));
    assert!(q.notification_injection_pending());
    let input = &q.publication_invocation().unwrap().input;
    assert_eq!(input["ancestry"], json!(["source"]));
    assert!(input.get("payload").is_none());
}

#[test]
fn batch_groups_do_not_merge_authenticated_callers() {
    let mut f = Fixture::new();
    let due = f.now + 1000;
    for index in 0..2 {
        f.admission.caller_sha256 = gate::sha256(&[index]);
        f.admission.dedupe_id = format!("caller-{index}");
        f.admission.trigger.occurrence_id = f.admission.dedupe_id.clone();
        f.accept();
        let c = f.claim(f.now).unwrap();
        assert_eq!(
            f.complete(
                c,
                gate::Decision::Batch {
                    key: "shared-key".into(),
                    flush_at_ms: due,
                    payload: json!({"index":index})
                }
            )
            .disposition,
            Disposition::Batched
        );
    }
    let groups: i64 =
        f.db.query_row(
            "SELECT count(*) FROM workflow_gate_batches WHERE closed=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(groups, 2);
}

#[test]
fn batch_forces_queue_and_carries_member_ancestry_union_and_earliest_expiry() {
    let mut f = Fixture::new();
    let due = f.now + 1000;
    let earliest = f.now + 20_000;
    for (index, deadline) in [(0, earliest + 1000), (1, earliest)] {
        let mut template = serde_json::to_value(&f.admission.template).unwrap();
        template["publication_invocation"] = json!({"publication_id":"publication","hook_id":"subscription","invocation_id":format!("receipt-{index}"),"transport":"workflow_notification","endpoint_id":"endpoint","queue_ref":"default","input":{"deadline_ms":deadline,"ancestry":[format!("ancestor-{index}")]},"artifacts":[],"mode":null,"caller":{"owner_id":"local","delivery_mode":"inject"}});
        f.admission.template = serde_json::from_value(template).unwrap();
        f.admission.deadline_ms = deadline;
        f.admission.dedupe_id = format!("notification-{index}");
        f.admission.trigger.occurrence_id = f.admission.dedupe_id.clone();
        f.accept();
        let c = f.claim(f.now).unwrap();
        assert_eq!(
            f.complete(
                c,
                gate::Decision::Batch {
                    key: "digest".into(),
                    flush_at_ms: due,
                    payload: json!({"index":index})
                }
            )
            .disposition,
            Disposition::Batched
        );
    }
    let c = f.claim(due).unwrap();
    let template = &c.stored.admission.template;
    assert!(!template.notification_injection_pending());
    assert_eq!(template.notification_deadline_ms(), Some(earliest));
    assert_eq!(c.stored.entry.deadline_ms, earliest);
    assert_eq!(
        template.publication_invocation().unwrap().input["ancestry"],
        json!(["ancestor-0", "ancestor-1"])
    );
}

#[test]
fn cancelling_a_batch_member_cancels_its_pending_aggregate_and_members() {
    let mut f = Fixture::new();
    let due = f.now + 1000;
    let first = f.accept();
    let c = f.claim(f.now).unwrap();
    f.complete(
        c,
        gate::Decision::Batch {
            key: "shared".into(),
            flush_at_ms: due,
            payload: json!({}),
        },
    );
    f.admission.dedupe_id = "second".into();
    f.admission.trigger.occurrence_id = "second".into();
    let second = f.accept();
    let c = f.claim(f.now).unwrap();
    f.complete(
        c,
        gate::Decision::Batch {
            key: "shared".into(),
            flush_at_ms: due,
            payload: json!({}),
        },
    );
    let (send, receive) = mpsc::channel();
    assert!(!execute(
        &mut f.db,
        Request {
            operation: Operation::Cancel {
                owner: crate::session::DEFAULT_LOCAL_USER_ID.into(),
                id: first.entry_id.clone(),
                reason: "creator cancelled".into()
            },
            response: send
        }
    ));
    let Outcome::Entry(cancelled) = receive.recv().unwrap().unwrap() else {
        panic!("entry result")
    };
    assert_eq!(cancelled.entry_id, first.entry_id);
    assert_eq!(cancelled.disposition, Disposition::Cancelled);
    assert_eq!(
        entry(&f.db, &second.entry_id).unwrap().unwrap().disposition,
        Disposition::Cancelled
    );
    assert!(f.claim(due).is_none());
}
