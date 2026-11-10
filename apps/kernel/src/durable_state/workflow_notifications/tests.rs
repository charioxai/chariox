//! MP-08 / MP-10: focused real single-writer SQLite drills; no provider/network.
use super::*;
use crate::agent::{AgentInstance, GridPosition};
use crate::session::{
    CreateSessionRequest, SessionService, WorkflowNodeRun, WorkflowNodeRunStatus,
    WorkflowOutputPayload, WorkflowRun, WorkflowRunStatus,
};
pub(crate) struct Fixture {
    pub root: std::path::PathBuf,
    pub store: DurableKernelStateStore,
    pub sessions: SessionService,
    pub session: String,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        Self::for_kernel("daemon-test")
    }
    pub(crate) fn for_kernel(kernel: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("chariox-wfnotify-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("wfnotify-fixture-owner"),
            "/root/work/agent-wfnotify",
        )
        .unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.daemon_id = kernel.into();
        let mut sessions = SessionService::new(&config);
        let mut session = sessions
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .unwrap();
        session.set_agents(vec![AgentInstance::new(
            "agent",
            "agent-ref",
            session.id(),
            None,
            "dev-stub",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        )]);
        let id = session.id().to_owned();
        sessions.restore_session(session);
        Self {
            root,
            store,
            sessions,
            session: id,
        }
    }
    pub(crate) fn workflow(&mut self, name: &str) -> (String, String, String) {
        let w = self
            .sessions
            .create_workflow(&self.session, Some(name.into()))
            .unwrap();
        let mut session = self.sessions.get_session(&self.session).unwrap();
        session.workflow_mut(w.id()).unwrap().set_max_concurrent(1);
        self.sessions.restore_session(session);
        let n = self
            .sessions
            .add_workflow_node(&self.session, w.id(), "agent")
            .unwrap();
        let mut session = self.sessions.get_session(&self.session).unwrap();
        session
            .workflow_mut(w.id())
            .unwrap()
            .node_mut(n.id())
            .unwrap()
            .set_can_complete_workflow_run(true);
        self.sessions.restore_session(session);
        let e = self
            .sessions
            .create_workflow_endpoint(&self.session, w.id(), n.id(), Some("main".into()))
            .unwrap();
        let p = self
            .sessions
            .create_workflow_publication(
                &self.session,
                w.id(),
                e.id(),
                Some("default".into()),
                Some(format!("{name}-events")),
                Some("event_based".into()),
                None,
                vec![],
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                "local".into(),
            )
            .unwrap();
        self.persist();
        (w.id().into(), e.id().into(), p.id().into())
    }
    pub(crate) fn persist(&self) {
        self.store
            .persist_workflow_runtime_transition(
                &self.sessions.get_session(&self.session).unwrap(),
                "wfnotify fixture",
            )
            .unwrap();
    }
    /// The workflow's one exit node.
    pub(crate) fn exit_of(&self, w: &str) -> String {
        self.sessions
            .resolve_workflow_ref(&self.session, w)
            .unwrap()
            .nodes()
            .iter()
            .find(|node| node.can_complete_workflow_run())
            .unwrap()
            .id()
            .to_owned()
    }
    pub(crate) fn source(&self, w: &str) -> WorkflowNotificationSource {
        self.source_of_exit(w, &self.exit_of(w))
    }
    /// The owner turns the exit source of `w` on or off.
    pub(crate) fn set_source_enabled(&self, w: &str, enabled: bool) {
        let mut source = self.source(w);
        source.enabled = enabled;
        let workflow = self
            .sessions
            .resolve_workflow_ref(&self.session, w)
            .unwrap();
        self.store
            .notify(NotificationOperation::Register(SourceAdmission {
                source,
                workflow_json: encode(&workflow).unwrap(),
            }))
            .unwrap();
    }
    pub(crate) fn source_of_exit(&self, w: &str, exit: &str) -> WorkflowNotificationSource {
        let s = self.sessions.get_session(&self.session).unwrap();
        let workflow = self
            .sessions
            .resolve_workflow_ref(&self.session, w)
            .unwrap();
        let source = WorkflowNotificationSource {
            kind: crate::local::NotificationSourceKind::WorkflowExit,
            source_id: format!("source-{w}-{exit}"),
            owner_user_id: "local".into(),
            kernel_id: s.host_daemon_id().into(),
            session_id: self.session.clone(),
            workflow_id: w.into(),
            exit_node_id: exit.into(),
            enabled: true,
            available: true,
            name: String::new(),
            exit_ordinal: 0,
            display_name: String::new(),
        };
        match self
            .store
            .notify(NotificationOperation::Register(SourceAdmission {
                source,
                workflow_json: encode(&workflow).unwrap(),
            }))
            .unwrap()
        {
            NotificationOutcome::Source(s) => s,
            _ => panic!(),
        }
    }
    pub(crate) fn attach(
        &self,
        source: &WorkflowNotificationSource,
        w: &str,
        p: &str,
    ) -> WorkflowNotificationSubscription {
        let target =
            WorkflowNotificationTarget::resolve(&self.sessions, "local", &self.session, p, None)
                .unwrap();
        let t = target.target();
        let subscription = WorkflowNotificationSubscription {
            subscription_id: format!("sub-{}-{w}", source.source_id),
            source_id: source.source_id.clone(),
            owner_user_id: "local".into(),
            created_by_user_id: "local".into(),
            target_kernel_id: source.kernel_id.clone(),
            target_kind: crate::local::WorkflowNotificationTargetKind::WorkflowEndpoint,
            session_id: self.session.clone(),
            workflow_id: w.into(),
            publication_id: p.into(),
            endpoint_id: t.endpoint_id.clone(),
            queue_id: t.queue_id.clone(),
            source_available: true,
            source_kernel_id: source.kernel_id.clone(),
            agent_id: None,
            trigger_id: None,
        };
        match self
            .store
            .notify(NotificationOperation::Attach {
                subscription,
                target,
            })
            .unwrap()
        {
            NotificationOutcome::Subscription(s) => s,
            _ => panic!(),
        }
    }
    pub(crate) fn complete(
        &mut self,
        w: &str,
        id: &str,
        queue: Option<&str>,
        status: WorkflowRunStatus,
        message: &str,
    ) {
        let exit = self.exit_of(w);
        self.complete_through(w, &exit, id, queue, status, message);
    }
    /// A run that ends through the given exit node.
    pub(crate) fn complete_through(
        &mut self,
        w: &str,
        exit: &str,
        id: &str,
        queue: Option<&str>,
        status: WorkflowRunStatus,
        message: &str,
    ) {
        let node_run = format!("node-run-{id}");
        let mut r = WorkflowRun::new(
            id,
            w,
            "endpoint",
            "node",
            None,
            None,
            vec![WorkflowNodeRun::new(
                &node_run,
                exit.to_owned(),
                "agent",
                0,
                WorkflowNodeRunStatus::Completed,
            )],
            vec![],
        );
        r.set_invocation_context(0, None, queue.map(str::to_owned), 0, None);
        r.set_final_output(
            Some(WorkflowOutputPayload::new(message, vec![])),
            Some(true),
            None,
            Some(node_run),
        );
        r.set_status(status);
        let mut session = self.sessions.get_session(&self.session).unwrap();
        session.create_workflow_run(r);
        self.sessions.restore_session(session);
        self.persist();
    }
    pub(crate) fn candidates(
        &self,
        accepted: bool,
    ) -> Vec<(
        WorkflowNotificationSubscription,
        WorkflowNotificationEnvelope,
    )> {
        self.store
            .notification_candidates(accepted, crate::session::unix_epoch_ms(), 8)
            .unwrap()
    }
    pub(crate) fn queue(
        &mut self,
        sub: WorkflowNotificationSubscription,
        env: WorkflowNotificationEnvelope,
    ) -> String {
        let p = PreparedNotification::prepare(&mut self.sessions, sub, env).unwrap();
        let after = p.after.clone();
        let id = after
            .workflow_queued_prompts()
            .back()
            .unwrap()
            .id()
            .to_owned();
        self.store
            .notify(NotificationOperation::Queue(Box::new(p)))
            .unwrap();
        self.sessions.restore_session(after);
        id
    }
}
pub(crate) fn cleanup(mut f: Fixture) {
    let root = f.root.clone();
    f.sessions = SessionService::new(&crate::config::DaemonConfig::for_tests());
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn completion_persists_before_router_restart_deduplicates_and_failed_runs_emit_nothing() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    let source = f.source(&a);
    f.attach(&source, &b, &bp);
    f.complete(
        &a,
        "success",
        None,
        WorkflowRunStatus::Completed,
        "final output",
    );
    f.complete(
        &a,
        "failed",
        None,
        WorkflowRunStatus::Failed,
        "failure output",
    );
    // Simulate completion committed and process stopped before any route/ACK.
    let root = f.root.clone();
    drop(f.store);
    f.store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
    assert_eq!(f.candidates(false).len(), 1);
    f.persist();
    assert_eq!(f.candidates(false).len(), 1);
    let (sub, env) = f.candidates(false).remove(0);
    assert_eq!(env.output.message(), "final output");
    assert_eq!(env.display_name, "a");
    assert!(matches!(
        f.store
            .notify(NotificationOperation::Accept {
                subscription: sub.clone(),
                envelope: env.clone()
            })
            .unwrap(),
        NotificationOutcome::Ack(WorkflowNotificationAck::Accepted)
    ));
    assert!(matches!(
        f.store
            .notify(NotificationOperation::Accept {
                subscription: sub.clone(),
                envelope: env.clone()
            })
            .unwrap(),
        NotificationOutcome::Ack(WorkflowNotificationAck::Duplicate)
    ));
    let queued = f.queue(sub.clone(), env.clone());
    let room = f.sessions.get_session(&f.session).unwrap();
    assert!(
        f.store
            .workflow_invocation_is_owned(
                room.host_daemon_id(),
                room.id(),
                &queued,
                &sub.owner_user_id
            )
            .unwrap(),
        "MP-11: notification admission retains the subscription owner's immutable provenance"
    );

    assert_eq!(
        f.sessions
            .get_session(&f.session)
            .unwrap()
            .workflow_queued_prompts()
            .len(),
        1
    );
    assert!(PreparedNotification::prepare(&mut f.sessions, sub.clone(), env.clone()).is_ok());
    // Retry cannot durably admit a second queue item even if preparation succeeds.
    let retry = PreparedNotification::prepare(&mut f.sessions, sub, env).unwrap();
    assert!(f
        .store
        .notify(NotificationOperation::Queue(Box::new(retry)))
        .is_err());
    assert!(!queued.is_empty());
    cleanup(f);
}
#[test]
fn runtime_ancestry_drops_two_and_three_workflow_cycles_visibly() {
    for middle in [false, true] {
        let mut f = Fixture::new();
        let (a, _, ap) = f.workflow("a");
        let (b, _, bp) = f.workflow("b");
        let (c, _, cp) = f.workflow("c");
        let sa = f.source(&a);
        let sb = f.source(&b);
        let sc = f.source(&c);
        f.attach(&sa, &b, &bp);
        if middle {
            f.attach(&sb, &c, &cp);
        }
        // A->B->A attach is already refused. Runtime cycles can still arise across
        // kernels or a raced graph; inject only test subscription rows for that seam.
        let target = if middle { &c } else { &b };
        let src = if middle { &sc } else { &sb };
        let t = WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &ap, None)
            .unwrap();
        let sub = WorkflowNotificationSubscription {
            subscription_id: "cycle-tail".into(),
            source_id: src.source_id.clone(),
            owner_user_id: "local".into(),
            created_by_user_id: "local".into(),
            target_kernel_id: src.kernel_id.clone(),
            target_kind: crate::local::WorkflowNotificationTargetKind::WorkflowEndpoint,
            session_id: f.session.clone(),
            workflow_id: a.clone(),
            publication_id: ap.clone(),
            endpoint_id: t.target().endpoint_id.clone(),
            queue_id: t.target().queue_id.clone(),
            source_available: true,
            source_kernel_id: src.kernel_id.clone(),
            agent_id: None,
            trigger_id: None,
        };
        let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
        db.execute(
            "INSERT INTO app_automations(owner_id,installation_id,automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status,source_kind,notification_json) VALUES (?3,?2,?1,1,'workflow_completion',1,'kernel',?6,?7,?8,?9,'active','workflow_completion',?5)",
            params![
                sub.subscription_id,
                sub.source_id,
                sub.owner_user_id,
                workflow_identity(&src.kernel_id, &f.session, &a),
                encode(&sub).unwrap(),sub.session_id,sub.publication_id,sub.endpoint_id,sub.queue_id
            ],
        )
        .unwrap();
        drop(db);
        f.complete(&a, "a-root", None, WorkflowRunStatus::Completed, "A");
        let sequence = if middle {
            vec![b.clone(), c.clone(), a.clone()]
        } else {
            vec![b.clone(), a.clone()]
        };
        for (i, w) in sequence.iter().enumerate() {
            let (s, e) = f.candidates(false).remove(0);
            let ack = f
                .store
                .notify(NotificationOperation::Accept {
                    subscription: s.clone(),
                    envelope: e.clone(),
                })
                .unwrap();
            if w == &a {
                assert!(matches!(
                    ack,
                    NotificationOutcome::Ack(WorkflowNotificationAck::LoopDropped)
                ));
                break;
            }
            let deadline = e.deadline_ms;
            let q = f.queue(s, e);
            f.store
                .notify(NotificationOperation::Sweep { now: deadline + 1 })
                .unwrap();
            f.complete(
                w,
                &format!("run-{i}"),
                Some(&q),
                WorkflowRunStatus::Completed,
                "next",
            );
        }
        assert!(f.candidates(false).is_empty(), "{target}");
        let (_, _, diagnostics) = f.store.notification_inventory("local").unwrap();
        assert!(diagnostics
            .iter()
            .any(|d| d.code == "workflow_notification_loop_dropped"));
        cleanup(f);
    }
}

#[test]
fn accepted_inbox_survives_paused_and_busy_targets_until_original_deadline() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    f.complete(&a, "ready", None, WorkflowRunStatus::Completed, "output");
    let (_, env) = f.candidates(false).remove(0);
    f.store
        .notify(NotificationOperation::Accept {
            subscription: sub.clone(),
            envelope: env.clone(),
        })
        .unwrap();
    // Pause through the existing SessionService queue operation.
    f.sessions
        .update_workflow_prompt_queue(&f.session, &b, &sub.queue_id, None, None, Some(false))
        .unwrap();
    f.persist();
    assert!(PreparedNotification::prepare(&mut f.sessions, sub.clone(), env.clone()).is_err());
    assert_eq!(f.candidates(true).len(), 1);
    let root = f.root.clone();
    drop(f.store);
    f.store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
    assert_eq!(f.candidates(true).len(), 1);
    f.sessions
        .update_workflow_prompt_queue(&f.session, &b, &sub.queue_id, None, None, Some(true))
        .unwrap();
    f.persist();
    f.complete(&b, "busy", None, WorkflowRunStatus::Running, "not final");
    assert!(PreparedNotification::prepare(&mut f.sessions, sub.clone(), env.clone()).is_err());
    assert_eq!(f.candidates(true).len(), 1);
    f.store
        .notify(NotificationOperation::Sweep {
            now: env.deadline_ms,
        })
        .unwrap();
    assert!(f
        .store
        .notification_candidates(true, env.deadline_ms, 8)
        .unwrap()
        .is_empty());
    let db = f.store.lock_connection("assert expiry").unwrap();
    let (state, body): (String, Option<String>) = db
        .query_row(
            "SELECT state,payload_json FROM app_outbox WHERE source_kind='workflow_completion'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "expired");
    assert!(body.is_none());
    drop(db);
    cleanup(f);
}

#[test]
fn wrong_owner_static_cycles_and_occurrence_content_conflicts_are_refused() {
    let mut f = Fixture::new();
    let (a, _, ap) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    let (c, _, cp) = f.workflow("c");
    let sa = f.source(&a);
    let sb = f.source(&b);
    let sc = f.source(&c);
    let ab = f.attach(&sa, &b, &bp);
    f.attach(&sb, &c, &cp);
    let target = || {
        WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &ap, None).unwrap()
    };
    let mut ca = ab.clone();
    ca.subscription_id = "c-a".into();
    ca.source_id = sc.source_id.clone();
    ca.workflow_id = a.clone();
    ca.publication_id = ap.clone();
    ca.endpoint_id = target().target().endpoint_id.clone();
    ca.queue_id = target().target().queue_id.clone();
    assert!(f
        .store
        .notify(NotificationOperation::Attach {
            subscription: ca,
            target: target()
        })
        .unwrap_err()
        .to_string()
        .contains("cycle"));
    let mut ba = ab.clone();
    ba.source_id = sb.source_id.clone();
    ba.workflow_id = a.clone();
    ba.publication_id = ap.clone();
    ba.endpoint_id = target().target().endpoint_id.clone();
    ba.queue_id = target().target().queue_id.clone();
    assert!(f
        .store
        .notify(NotificationOperation::Attach {
            subscription: ba,
            target: target()
        })
        .is_err());
    let mut foreign = ab.clone();
    foreign.owner_user_id = "another-user".into();
    let t =
        WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &bp, None).unwrap();
    assert!(f
        .store
        .notify(NotificationOperation::Attach {
            subscription: foreign,
            target: t
        })
        .is_err());
    assert!(f
        .store
        .notification_inventory("another-user")
        .unwrap()
        .0
        .is_empty());
    f.complete(&a, "one", None, WorkflowRunStatus::Completed, "original");
    let (sub, mut env) = f.candidates(false).remove(0);
    env.output = WorkflowOutputPayload::new("forged", vec![]);
    assert!(f
        .store
        .notify(NotificationOperation::Accept {
            subscription: sub,
            envelope: env
        })
        .is_err());
    cleanup(f);
}
#[test]
fn source_registration_has_no_historical_replay_and_output_limit_has_visible_diagnostic() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    f.complete(
        &a,
        "before-register",
        None,
        WorkflowRunStatus::Completed,
        "historical",
    );
    let source = f.source(&a);
    f.attach(&source, &b, &bp);
    f.persist();
    assert!(f.candidates(false).is_empty());
    f.complete(
        &a,
        "oversized",
        None,
        WorkflowRunStatus::Completed,
        &"x".repeat(MAX_OUTPUT_BYTES),
    );
    assert!(f.candidates(false).is_empty());
    assert!(f
        .store
        .notification_inventory("local")
        .unwrap()
        .2
        .iter()
        .any(|d| d.code == "workflow_notification_output_limit"));
    cleanup(f);
}

#[test]
fn expired_ordinary_notification_queue_never_dispatches_after_recovery() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    let source = f.source(&a);
    f.attach(&source, &b, &bp);
    f.complete(&a, "one", None, WorkflowRunStatus::Completed, "output");
    let (s, e) = f.candidates(false).remove(0);
    let deadline = e.deadline_ms;
    f.store
        .notify(NotificationOperation::Accept {
            subscription: s.clone(),
            envelope: e.clone(),
        })
        .unwrap();
    f.queue(s, e);
    let mut session = f.sessions.get_session(&f.session).unwrap();
    assert!(session
        .workflow_queued_prompts()
        .front()
        .unwrap()
        .notification_expired_at(deadline));
    assert!(session.expire_workflow_notification_prompts(deadline));
    assert!(session.pop_next_workflow_queued_prompt().is_none());
    f.sessions.restore_session(session);
    f.persist();
    let root = f.root.clone();
    drop(f.store);
    f.store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
    let hot = f.store.load_workflow_hot_states("daemon-test").unwrap();
    assert!(hot.iter().any(|(_, state)| state
        .workflow_queued_prompts
        .iter()
        .any(|q| q.status() == crate::session::WorkflowQueuedPromptStatus::Cancelled)));
    cleanup(f);
}

#[test]
fn ownership_transfer_does_not_capture_output_for_former_owner_or_touch_pending() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    let old = f.source(&a);
    f.attach(&old, &b, &bp);
    f.complete(
        &a,
        "old-owner",
        None,
        WorkflowRunStatus::Completed,
        "old output",
    );
    let mut session = f.sessions.get_session(&f.session).unwrap();
    session.set_owner_user_id("another-user");
    f.sessions.restore_session(session);
    f.persist();
    f.complete(
        &a,
        "new-owner-unregistered",
        None,
        WorkflowRunStatus::Completed,
        "private new output",
    );
    assert_eq!(
        f.candidates(false).len(),
        1,
        "former owner must capture no new output"
    );
    let mut source = old.clone();
    source.source_id = "new-owner-source".into();
    source.owner_user_id = "another-user".into();
    let workflow = f.sessions.resolve_workflow_ref(&f.session, &a).unwrap();
    let NotificationOutcome::Source(new) = f
        .store
        .notify(NotificationOperation::Register(SourceAdmission {
            source,
            workflow_json: encode(&workflow).unwrap(),
        }))
        .unwrap()
    else {
        panic!()
    };
    assert_ne!(new.source_id, old.source_id);
    f.complete(
        &a,
        "new-owner-registered",
        None,
        WorkflowRunStatus::Completed,
        "new output",
    );
    assert_eq!(
        f.candidates(false).len(),
        1,
        "old pending is untouched; no historical or new replay to old subscription"
    );
    assert_eq!(
        f.store
            .notification_inventory("another-user")
            .unwrap()
            .0
            .len(),
        1
    );
    cleanup(f);
}

#[test]
fn a_run_that_fails_before_any_exit_emits_nothing() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("reviewer");
    let (b, _, bp) = f.workflow("subscriber");
    let source = f.source(&a);
    f.attach(&source, &b, &bp);
    f.complete(
        &a,
        "failed-review",
        None,
        WorkflowRunStatus::Failed,
        "must not be exported",
    );
    assert!(f.candidates(false).is_empty());
    assert!(f
        .store
        .notification_inventory("local")
        .unwrap()
        .2
        .is_empty());
    f.complete(
        &a,
        "reviewed",
        None,
        WorkflowRunStatus::Completed,
        "approved",
    );
    assert_eq!(f.candidates(false).len(), 1);
    cleanup(f);
}

// MP-08 / MP-10: the output can consume the rendered prompt's envelope reserve.
#[test]
fn notification_prompt_header_is_reserved_before_source_persistence_and_target_ack() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("bounded-source");
    let (b, _, bp) = f.workflow("bounded-target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    let header = NOTIFICATION_PROMPT_HEADER;
    let mut env = WorkflowNotificationEnvelope {
        source_id: source.source_id.clone(),
        display_name: "bounded-source".into(),
        occurrence_id: "boundary".into(),
        output: WorkflowOutputPayload::new("", vec![]),
        ancestry: vec![workflow_identity(&source.kernel_id, &f.session, &a)],
        deadline_ms: crate::session::unix_epoch_ms() + 7 * 86_400_000,
    };
    // The output is bounded below the prompt, so the reserve is consumed by lineage entries
    // (each adds its length plus the quotes and the comma).
    let mut extra = MAX_PROMPT_BYTES - header.len() - encode(&env).unwrap().len();
    while extra >= 406 {
        env.ancestry.push("x".repeat(400));
        extra -= 403;
    }
    env.ancestry.push("x".repeat(extra - 3));
    assert_eq!(encode(&env).unwrap().len() + header.len(), MAX_PROMPT_BYTES);
    let mut oversized = env.clone();
    oversized.ancestry.last_mut().unwrap().push('x');

    // A different target's first admission must apply the same bound before ACK.
    let refusal = f.store.notify(NotificationOperation::Accept {
        subscription: sub.clone(),
        envelope: oversized,
    });
    let accepted_after_refusal = f.candidates(true);
    // Boundary control: an exactly fitting rendered prompt is accepted and queued.
    let accepted = f
        .store
        .notify(NotificationOperation::Accept {
            subscription: sub.clone(),
            envelope: env.clone(),
        })
        .unwrap();
    f.queue(sub, env);
    let prompt_len = f
        .sessions
        .get_session(&f.session)
        .unwrap()
        .workflow_queued_prompts()
        .back()
        .unwrap()
        .prompt()
        .unwrap()
        .len();
    cleanup(f);
    assert!(
        refusal.is_err(),
        "MP-08: oversized prompt must not be ACKed"
    );
    assert!(accepted_after_refusal.is_empty());
    assert!(matches!(
        accepted,
        NotificationOutcome::Ack(WorkflowNotificationAck::Accepted)
    ));
    assert_eq!(prompt_len, MAX_PROMPT_BYTES);
}

// MP-08 / MP-10 / MP-11: reactivation adds an active subscriber at both ceilings.
#[test]
fn disabled_binding_reactivation_respects_source_fanout_limit() {
    reactivation_limit_drill(false);
}
#[test]
fn disabled_binding_reactivation_respects_owner_subscription_limit() {
    reactivation_limit_drill(true);
}
fn reactivation_limit_drill(owner_limit: bool) {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("limited-source");
    let (b, _, bp) = f.workflow("limited-target");
    let source = f.source(&a);
    let original = f.attach(&source, &b, &bp);
    f.store
        .notify(NotificationOperation::Detach {
            subscription_id: original.subscription_id.clone(),
            owner: original.owner_user_id.clone(),
            kernel: original.target_kernel_id.clone(),
        })
        .unwrap();
    let limit = if owner_limit { 1024 } else { MAX_SUBSCRIPTIONS };
    let mut last = original.clone();
    for n in 0..limit {
        let mut sub = original.clone();
        sub.subscription_id = format!("filler-{n}");
        sub.target_kernel_id = format!("target-{n}");
        if owner_limit {
            sub.source_id = format!("remote-source-{}", n / MAX_SUBSCRIPTIONS);
            sub.source_kernel_id = "remote-source-kernel".into();
        }
        let NotificationOutcome::Subscription(saved) = f
            .store
            .notify(NotificationOperation::RemoteAttach { subscription: sub })
            .unwrap()
        else {
            panic!("subscription")
        };
        last = saved;
    }
    // An already-active replacement at capacity still works and preserves its ID.
    last.queue_id = "replacement-queue".into();
    let replacement = f
        .store
        .notify(NotificationOperation::RemoteAttach {
            subscription: last.clone(),
        })
        .unwrap();
    let target =
        WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &bp, None).unwrap();
    let refusal = f.store.notify(NotificationOperation::Attach {
        subscription: original.clone(),
        target,
    });
    let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
    let status: String = db
        .query_row(
            "SELECT status FROM app_automations WHERE automation_id=?1",
            [&original.subscription_id],
            |r| r.get(0),
        )
        .unwrap();
    let count: u32 = db.query_row("SELECT count(*) FROM app_automations WHERE source_kind='workflow_completion' AND owner_id='local' AND status='active'", [], |r| r.get(0)).unwrap();
    drop(db);
    let inventory_len = f.store.notification_inventory("local").unwrap().1.len();
    // Free one slot: the detached row may reactivate using its original identity.
    f.store
        .notify(NotificationOperation::Detach {
            subscription_id: last.subscription_id.clone(),
            owner: last.owner_user_id.clone(),
            kernel: last.target_kernel_id.clone(),
        })
        .unwrap();
    let target =
        WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &bp, None).unwrap();
    let reactivated = f
        .store
        .notify(NotificationOperation::Attach {
            subscription: original.clone(),
            target,
        })
        .unwrap();
    cleanup(f);
    assert!(refusal.is_err(), "MP-08 / MP-11: disabled binding must consume a slot on reactivation (owner_limit={owner_limit})");
    assert!(refusal.unwrap_err().to_string().contains(if owner_limit {
        "workflow notification subscription limit"
    } else {
        "notification fanout limit"
    }));
    assert_eq!(status, "disabled");
    assert_eq!(count as usize, limit);
    assert_eq!(inventory_len, count as usize);
    assert!(matches!(replacement, NotificationOutcome::Subscription(s) if s == last));
    assert!(
        matches!(reactivated, NotificationOutcome::Subscription(s) if s.subscription_id == original.subscription_id)
    );
}

// MP-08 / MP-10 / MP-11: persisted UTF-8 text is charged in bytes on every admission.
#[test]
fn retained_multibyte_notification_payload_limit_is_measured_in_bytes() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("byte-source");
    let (b, _, bp) = f.workflow("byte-target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    f.complete(&a, "seed", None, WorkflowRunStatus::Completed, "fixture");
    let (_, mut env) = f.candidates(false).pop().unwrap();
    env.output = WorkflowOutputPayload::new("🦀".repeat(7000) + &"界".repeat(10000), vec![]);
    env.occurrence_id = "byte-0000".into();
    let size = encode_notification_envelope(&env).unwrap().len();
    let max = chariox_app_runtime::app_outbox::MAX_RETAINED_PAYLOAD_BYTES;
    let capacity = max / size;
    assert!(capacity < MAX_PENDING as usize);
    let mut db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
    db.execute(
        "DELETE FROM app_outbox WHERE source_kind='workflow_completion'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    for n in 0..capacity {
        env.occurrence_id = format!("byte-{n:04}");
        assert_eq!(encode_notification_envelope(&env).unwrap().len(), size);
        insert_receipt(
            &tx,
            &sub,
            &env,
            "retryable",
            crate::session::unix_epoch_ms(),
        )
        .unwrap();
    }
    env.occurrence_id = format!("byte-{capacity:04}");
    let refusal = insert_receipt(
        &tx,
        &sub,
        &env,
        "retryable",
        crate::session::unix_epoch_ms(),
    );
    let (bytes, chars): (i64, i64) = tx.query_row(
        "SELECT sum(length(CAST(payload_json AS BLOB))),sum(length(payload_json)) FROM app_outbox",
        [], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    tx.rollback().unwrap();
    drop(db);
    cleanup(f);
    assert!(
        refusal.is_err(),
        "MP-08: next UTF-8 envelope must exceed the retained byte ceiling"
    );
    assert!(refusal
        .unwrap_err()
        .to_string()
        .contains("notification outbox full"));
    assert_eq!(bytes as usize, capacity * size);
    assert!(bytes as usize <= max && bytes as usize + size > max);
    assert!(
        chars < bytes / 2,
        "fixture distinguishes characters from bytes"
    );
}
// MP-08/MP-10/MP-11 F8: one subscription cannot consume another's allowance.
#[test]
fn security_f8_notification_budget_is_per_subscription() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("source");
    let (b, _, bp) = f.workflow("first");
    let (c, _, cp) = f.workflow("second");
    let source = f.source(&a);
    let first = f.attach(&source, &b, &bp);
    let second = f.attach(&source, &c, &cp);
    f.complete(&a, "seed", None, WorkflowRunStatus::Completed, "fixture");
    let (_, mut env) = f.candidates(false).pop().unwrap();
    let mut db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
    db.execute("DELETE FROM app_outbox", []).unwrap();
    let tx = db.transaction().unwrap();
    for n in 0..MAX_PENDING {
        env.occurrence_id = format!("first-{n}");
        insert_receipt(&tx, &first, &env, "retryable", 1).unwrap();
    }
    env.occurrence_id = "second-0".into();
    let independent = insert_receipt(&tx, &second, &env, "retryable", 1);
    assert!(
        independent.is_ok(),
        "MP-11 F8: another subscription retains its own admission budget"
    );
    assert!(insert_receipt(&tx, &first, &env, "retryable", 1).is_err());
    tx.rollback().unwrap();
    drop(db);
    cleanup(f);
}
#[test]
fn security_f8_notification_receipts_redact_output() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("source");
    let (b, _, bp) = f.workflow("target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    let synthetic = format!("ghp_{}", "a".repeat(36));
    f.complete(&a, "seed", None, WorkflowRunStatus::Completed, &synthetic);
    let (_, mut env) = f.candidates(false).pop().unwrap();
    assert_eq!(
        env.output.message(),
        crate::secret_redaction::redact_secrets(&synthetic),
        "MP-11 F8: source outbox must protect public output before forwarding"
    );
    env.occurrence_id = "inbound".into();
    env.output = WorkflowOutputPayload::new(&synthetic, vec![]);
    f.store
        .notify(NotificationOperation::Accept {
            subscription: sub.clone(),
            envelope: env.clone(),
        })
        .unwrap();
    let (_, stored) = f.candidates(true).pop().unwrap();
    assert_ne!(stored.output, env.output);
    let replay = f
        .store
        .notify(NotificationOperation::Accept {
            subscription: sub.clone(),
            envelope: env.clone(),
        })
        .unwrap();
    assert!(matches!(
        replay,
        NotificationOutcome::Ack(WorkflowNotificationAck::Duplicate)
    ));
    env.output = WorkflowOutputPayload::new(format!("ghp_{}", "b".repeat(36)), vec![]);
    assert!(f
        .store
        .notify(NotificationOperation::Accept {
            subscription: sub,
            envelope: env
        })
        .is_err());
    cleanup(f);
}

#[test]
fn security_f8_legacy_candidates_redact_before_local_or_peer_delivery() {
    for state in ["retryable", "accepted"] {
        let mut f = Fixture::new();
        let (a, _, _) = f.workflow("source");
        let (b, _, bp) = f.workflow("target");
        let source = f.source(&a);
        f.attach(&source, &b, &bp);
        f.complete(&a, "legacy", None, WorkflowRunStatus::Completed, "fixture");
        let (_, mut env) = f.candidates(false).pop().unwrap();
        let synthetic = format!("ghp_{}", "a".repeat(36));
        env.output = WorkflowOutputPayload::new(&synthetic, vec![]);
        use sha2::{Digest, Sha256};
        let legacy_bytes = encode(&env).unwrap();
        let digest = format!("{:x}", Sha256::digest(legacy_bytes.as_bytes()));
        // MP-11 / MP-08 / MP-10: retained legacy bytes must deliver without replacing replay proof.
        {
            let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
            db.execute("UPDATE app_outbox SET payload_json=?1,state=?2,content_digest=?3 WHERE source_kind='workflow_completion'", rusqlite::params![legacy_bytes,state,digest]).unwrap();
        }
        let (sub, safe) = f.candidates(state == "accepted").pop().unwrap();
        assert_ne!(safe.output, env.output);
        if state == "retryable" {
            assert!(matches!(
                f.store
                    .notify(NotificationOperation::Accept {
                        subscription: sub.clone(),
                        envelope: safe.clone()
                    })
                    .unwrap(),
                NotificationOutcome::Ack(WorkflowNotificationAck::Accepted)
            ));
        }
        f.queue(sub.clone(), safe.clone());
        {
            let db = f.store.lock_connection("MP-11 F8 legacy digest").unwrap();
            let retained: String = db
                .query_row(
                    "SELECT content_digest FROM app_outbox WHERE source_kind='workflow_completion'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(retained, digest);
        }
        for replay in [env.clone(), safe] {
            assert!(matches!(
                f.store
                    .notify(NotificationOperation::Accept {
                        subscription: sub.clone(),
                        envelope: replay
                    })
                    .unwrap(),
                NotificationOutcome::Ack(WorkflowNotificationAck::Duplicate)
            ));
        }
        env.output = WorkflowOutputPayload::new(format!("ghp_{}", "c".repeat(36)), vec![]);
        assert!(
            f.store
                .notify(NotificationOperation::Accept {
                    subscription: sub,
                    envelope: env
                })
                .is_err(),
            "MP-11: redaction must not hide a mutated raw replay"
        );
        cleanup(f);
    }
}

#[test]
fn mp08_mp11_notification_occurrences_are_bounded_without_replaying_completions() {
    let mut f = Fixture::new();
    let (w, _, _) = f.workflow("bounded");
    f.source(&w);
    // An upgraded old database has accumulated successful and diagnostic rows.
    {
        let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
        for n in 0..5000 {
            db.execute(
                "INSERT INTO workflow_notification_occurrences VALUES (?1,?2,?3)",
                params![
                    format!("source-{w}"),
                    format!("old-{n}"),
                    if n % 2 == 0 {
                        Some("old_diagnostic")
                    } else {
                        None
                    }
                ],
            )
            .unwrap();
        }
    }
    f.complete(&w, "latest", None, WorkflowRunStatus::Completed, "done");
    let count = || {
        let db = f.store.lock_connection("test").unwrap();
        db.query_row(
            "SELECT COUNT(*) FROM workflow_notification_occurrences",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert!(count() <= 4096);
    let before = count();
    f.persist();
    assert_eq!(count(), before);
}

// MP-08 / MP-10 (VS-S1-1, VS-S1-2): each exit is its own source, named by ordinal once more than
// one exit notifies, and a run notifies only through the exit it ended at.
#[test]
fn two_exits_are_independent_sources_named_by_ordinal() {
    let mut f = Fixture::new();
    let (pipeline, _, _) = f.workflow("pipeline");
    let (consumer, _, consumer_publication) = f.workflow("consumer");
    let mut session = f.sessions.get_session(&f.session).unwrap();
    let mut agents = session.agents().to_vec();
    agents.push(AgentInstance::new(
        "agent-2",
        "agent-ref-2",
        session.id(),
        None,
        "dev-stub",
        None,
        None,
        None,
        GridPosition::new(0, 1, 1, 1),
    ));
    session.set_agents(agents);
    f.sessions.restore_session(session);
    let second = f
        .sessions
        .add_workflow_node(&f.session, &pipeline, "agent-2")
        .unwrap();
    let mut session = f.sessions.get_session(&f.session).unwrap();
    session
        .workflow_mut(&pipeline)
        .unwrap()
        .node_mut(second.id())
        .unwrap()
        .set_can_complete_workflow_run(true);
    f.sessions.restore_session(session);
    f.persist();
    let first_exit = f.exit_of(&pipeline);
    let second_exit = second.id().to_owned();

    let first = f.source_of_exit(&pipeline, &first_exit);
    let only_first = f
        .store
        .notification_inventory("local")
        .unwrap()
        .0
        .into_iter()
        .filter(|s| s.enabled && s.workflow_id == pipeline)
        .count();
    assert_eq!(only_first, 1);
    let second_source = f.source_of_exit(&pipeline, &second_exit);
    assert_ne!(first.source_id, second_source.source_id);
    f.attach(&second_source, &consumer, &consumer_publication);

    // A run that ends through the first exit does not notify the second exit's subscriber.
    f.complete_through(
        &pipeline,
        &first_exit,
        "through-first",
        None,
        WorkflowRunStatus::Completed,
        "approved",
    );
    assert!(f.candidates(false).is_empty());
    f.complete_through(
        &pipeline,
        &second_exit,
        "through-second",
        None,
        WorkflowRunStatus::Completed,
        "rejected",
    );
    let candidates = f.candidates(false);
    assert_eq!(candidates.len(), 1);
    let (_, envelope) = &candidates[0];
    assert_eq!(envelope.source_id, second_source.source_id);
    assert_eq!(envelope.occurrence_id, "through-second");
    assert_eq!(envelope.output.message(), "rejected");
    // With both exits notifying, the name carries the exit's position among the workflow's exits.
    assert_eq!(envelope.display_name, "pipeline-2");
    cleanup(f);
}
