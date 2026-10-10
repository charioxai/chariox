//! MP-08 / MP-10: focused real single-writer SQLite drills; no provider/network.
use super::*;
use crate::agent::{AgentInstance, GridPosition};
use crate::local::WorkflowNotificationEvents;
use crate::session::{
    CreateSessionRequest, SessionService, WorkflowOutputPayload, WorkflowRun, WorkflowRunStatus,
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
    pub(crate) fn source(&self, w: &str) -> WorkflowNotificationSource {
        let s = self.sessions.get_session(&self.session).unwrap();
        let workflow = self
            .sessions
            .resolve_workflow_ref(&self.session, w)
            .unwrap();
        let source = WorkflowNotificationSource {
            source_id: format!("source-{w}"),
            owner_user_id: "local".into(),
            kernel_id: s.host_daemon_id().into(),
            session_id: self.session.clone(),
            workflow_id: w.into(),
            enabled: true,
            available: true,
            name: String::new(),
            output_fields: vec![],
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
            delivery_mode: crate::local::NotificationDeliveryMode::Queue,
            subscription_id: format!("sub-{}-{w}", source.source_id),
            source_id: source.source_id.clone(),
            owner_user_id: "local".into(),
            target_kernel_id: source.kernel_id.clone(),
            target_kind: crate::local::WorkflowNotificationTargetKind::WorkflowEndpoint,
            session_id: self.session.clone(),
            workflow_id: w.into(),
            publication_id: p.into(),
            endpoint_id: t.endpoint_id.clone(),
            queue_id: t.queue_id.clone(),
            ttl_days: 7,
            source_available: true,
            source_kernel_id: source.kernel_id.clone(),
            events: WorkflowNotificationEvents::Both,
            filters: serde_json::Value::Null,
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
        let mut r = WorkflowRun::new(id, w, "endpoint", "node", None, None, vec![], vec![]);
        r.set_invocation_context(0, None, queue.map(str::to_owned), 0, None);
        r.set_final_output(
            Some(WorkflowOutputPayload::new(message, vec![])),
            Some(true),
            None,
            None,
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
fn completion_persists_before_router_restart_deduplicates_and_events_are_selected() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("a");
    let (b, _, bp) = f.workflow("b");
    let source = f.source(&a);
    let mut sub = f.attach(&source, &b, &bp);
    sub.events = WorkflowNotificationEvents::Success;
    let target =
        WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &bp, None).unwrap();
    f.store
        .notify(NotificationOperation::Attach {
            subscription: sub,
            target,
        })
        .unwrap();
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
    assert_eq!(env.output.as_ref().unwrap().message(), "final output");
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
            delivery_mode: crate::local::NotificationDeliveryMode::Queue,
            subscription_id: "cycle-tail".into(),
            source_id: src.source_id.clone(),
            owner_user_id: "local".into(),
            target_kernel_id: src.kernel_id.clone(),
            target_kind: crate::local::WorkflowNotificationTargetKind::WorkflowEndpoint,
            session_id: f.session.clone(),
            workflow_id: a.clone(),
            publication_id: ap.clone(),
            endpoint_id: t.target().endpoint_id.clone(),
            queue_id: t.target().queue_id.clone(),
            ttl_days: 7,
            source_available: true,
            source_kernel_id: src.kernel_id.clone(),
            events: WorkflowNotificationEvents::Both,
            filters: serde_json::Value::Null,
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
fn wrong_owner_ttl_static_cycles_and_occurrence_content_conflicts_are_refused() {
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
    for ttl in [0, 31] {
        let mut invalid = ab.clone();
        invalid.ttl_days = ttl;
        let t = WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &bp, None)
            .unwrap();
        assert!(f
            .store
            .notify(NotificationOperation::Attach {
                subscription: invalid,
                target: t
            })
            .is_err());
    }
    assert!(f
        .store
        .notification_inventory("another-user")
        .unwrap()
        .0
        .is_empty());
    f.complete(&a, "one", None, WorkflowRunStatus::Completed, "original");
    let (sub, mut env) = f.candidates(false).remove(0);
    env.output = Some(WorkflowOutputPayload::new("forged", vec![]));
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
fn enabled_source_emits_failure_bare_status() {
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
    assert_eq!(
        f.candidates(false).len(),
        1,
        "MP-08 / MP-10: failure completion must emit"
    );
    cleanup(f);
}

#[test]
fn d1_d2_opaque_fixture_subject_fields_filters_and_bare_failure() {
    for generator in ["github", "inventory"] {
        opaque_fixture_drill(generator);
    }
}
fn opaque_fixture_drill(generator: &str) {
    use crate::session::WorkflowPublicationInvocationEnvelope;
    let mut f = Fixture::new();
    let (reviewer, _, _) = f.workflow("reviewer");
    let (all, _, allp) = f.workflow("repo-subscriber");
    let (one, _, onep) = f.workflow("pr-873-subscriber");
    let mut source = f.source(&reviewer);
    source.output_fields = vec!["verdict".into(), "review_url".into()];
    let workflow = f
        .sessions
        .resolve_workflow_ref(&f.session, &reviewer)
        .unwrap();
    f.store
        .notify(NotificationOperation::Register(SourceAdmission {
            source: source.clone(),
            workflow_json: encode(&workflow).unwrap(),
        }))
        .unwrap();
    for (workflow, publication, filter) in [
        (&all, &allp, serde_json::json!({"repo":"fixture/repo"})),
        (
            &one,
            &onep,
            serde_json::json!({"repo":"fixture/repo","pr":[873]}),
        ),
    ] {
        let mut sub = f.attach(&source, workflow, publication);
        sub.filters = filter;
        let target = WorkflowNotificationTarget::resolve(
            &f.sessions,
            "local",
            &f.session,
            publication,
            None,
        )
        .unwrap();
        f.store
            .notify(NotificationOperation::Attach {
                subscription: sub,
                target,
            })
            .unwrap();
    }
    for (pr, status) in [
        (873, WorkflowRunStatus::Completed),
        (874, WorkflowRunStatus::Completed),
        (875, WorkflowRunStatus::Failed),
    ] {
        let invocation = WorkflowPublicationInvocationEnvelope {
            publication_id: "fixture-app-publication".into(),
            hook_id: Some("fixture-automation".into()),
            invocation_id: format!("pull-{pr}"),
            transport: "app_event".into(),
            endpoint_id: "endpoint".into(),
            queue_ref: None,
            input: serde_json::json!({"event_type":format!("{generator}.changed"),"payload":{"metadata":{"repo":"fixture/repo","pr":pr,"head_sha":format!("sha-{pr}"),"subject":format!("{generator}:opaque/{pr}"),"nested":{"category":["one","two"]}}}}),
            artifacts: vec![],
            mode: None,
            caller: serde_json::json!({"kind":"app_event","owner_id":"local"}),
        };
        let mut run = WorkflowRun::new(
            format!("review-{pr}"),
            &reviewer,
            "endpoint",
            "node",
            None,
            Some(invocation),
            vec![],
            vec![],
        );
        run.set_final_output(Some(WorkflowOutputPayload::new(serde_json::json!({"verdict":"approved","review_url":format!("https://example.test/review/{pr}"),"repo":"forged/repo","pr":999}).to_string(),vec![])),Some(true),None,None);
        run.set_status(status);
        let mut session = f.sessions.get_session(&f.session).unwrap();
        session.create_workflow_run(run);
        f.sessions.restore_session(session);
        f.persist();
    }
    let candidates = f.candidates(false);
    assert_eq!(
        candidates.len(),
        4,
        "MP-08 / MP-10 D2: repo gets 3, PR 873 gets 1"
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|(s, _)| s.workflow_id == one)
            .count(),
        1
    );
    for (sub, env) in candidates {
        let pr = env.fields["pr"].as_u64().unwrap();
        assert_eq!(env.subject, Some(format!("{generator}:opaque/{pr}")));
        assert_eq!(env.fields["repo"], "fixture/repo");
        assert_eq!(
            env.fields["nested"],
            serde_json::json!({"category":["one","two"]})
        );
        assert_eq!(env.fields["head_sha"], format!("sha-{pr}"));
        if pr == 875 {
            assert!(env.output.is_none());
            assert!(env.fields.get("verdict").is_none());
            assert_eq!(env.fields["status"], "failure");
        } else {
            assert_eq!(env.fields["verdict"], "approved");
            assert_eq!(
                env.fields["review_url"],
                format!("https://example.test/review/{pr}")
            );
        }
        let mut rejected = env.clone();
        rejected.fields["repo"] = serde_json::json!("other/repo");
        assert!(matches!(
            f.store
                .notify(NotificationOperation::Accept {
                    subscription: sub.clone(),
                    envelope: rejected
                })
                .unwrap(),
            NotificationOutcome::Ack(WorkflowNotificationAck::Filtered)
        ));
        assert!(matches!(
            f.store
                .notify(NotificationOperation::Accept {
                    subscription: sub.clone(),
                    envelope: env.clone()
                })
                .unwrap(),
            NotificationOutcome::Ack(WorkflowNotificationAck::Accepted)
        ));
        // Retries preserve the occurrence; target durable acceptance is independent of a run.
        assert!(matches!(
            f.store
                .notify(NotificationOperation::Accept {
                    subscription: sub.clone(),
                    envelope: env.clone()
                })
                .unwrap(),
            NotificationOutcome::Ack(WorkflowNotificationAck::Duplicate)
        ));
        f.queue(sub, env);
    }
    assert_eq!(
        f.sessions
            .get_session(&f.session)
            .unwrap()
            .workflow_queued_prompts()
            .len(),
        4
    );
    cleanup(f);
}

#[test]
fn round1_migration_preserves_pending_and_inbox_only_queue_lineage() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("source");
    let (b, _, bp) = f.workflow("target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    f.complete(
        &a,
        "legacy-pending",
        None,
        WorkflowRunStatus::Completed,
        "pending",
    );
    let (_, pending) = f.candidates(false).remove(0);
    let mut queued = pending.clone();
    queued.occurrence_id = "legacy-queued".into();
    let legacy_envelope = |env: &WorkflowNotificationEnvelope| {
        let mut value = serde_json::to_value(env).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("status");
        object.remove("subject");
        object.remove("fields");
        serde_json::to_string(&value).unwrap()
    };
    let mut legacy_sub = serde_json::to_value(&sub).unwrap();
    for field in ["source_kernel_id", "events", "filters", "target_kind"] {
        legacy_sub.as_object_mut().unwrap().remove(field);
    }
    drop(f.store);
    let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("DELETE FROM app_outbox WHERE source_kind='workflow_completion';
        DELETE FROM app_automations WHERE source_kind='workflow_completion';
        CREATE TABLE workflow_notification_subscriptions(subscription_id TEXT PRIMARY KEY, payload_json TEXT NOT NULL);
        CREATE TABLE workflow_notification_outbox(subscription_id TEXT, envelope_json TEXT, state TEXT);
        CREATE TABLE workflow_notification_inbox(subscription_id TEXT, envelope_json TEXT, state TEXT, queued_prompt_id TEXT);").unwrap();
    db.execute(
        "INSERT INTO workflow_notification_subscriptions VALUES (?1,?2)",
        params![sub.subscription_id, legacy_sub.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO workflow_notification_outbox VALUES (?1,?2,'pending')",
        params![sub.subscription_id, legacy_envelope(&pending)],
    )
    .unwrap();
    // Inbox-only migration must retain queue/session ancestry even without an
    // outgoing half. It also retains the id already embedded in legacy runs.
    db.execute(
        "INSERT INTO workflow_notification_inbox VALUES (?1,?2,'queued','legacy-queue')",
        params![sub.subscription_id, legacy_envelope(&queued)],
    )
    .unwrap();
    drop(db);
    f.store = DurableKernelStateStore::open_owned(f.root.join("kernel.sqlite")).unwrap();
    assert_eq!(f.candidates(false).len(), 1);
    assert_eq!(f.candidates(false)[0].1.deadline_ms, pending.deadline_ms);
    assert_eq!(
        f.store.notification_inventory("local").unwrap().1[0].events,
        WorkflowNotificationEvents::Success
    );
    let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
    let (session, queue, lineage, receipt): (String,String,String,String) = db.query_row("SELECT queued_session_id,queued_prompt_id,invocation_json,receipt_id FROM app_outbox WHERE occurrence_id='legacy-queued'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(session, f.session);
    assert_eq!(queue, "legacy-queue");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&lineage).unwrap()["ancestry"],
        serde_json::json!(queued.ancestry)
    );
    assert_eq!(
        receipt,
        format!(
            "{}:{}:{}",
            sub.subscription_id, queued.source_id, queued.occurrence_id
        )
    );
    let old_tables:u32=db.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('workflow_notification_subscriptions','workflow_notification_outbox','workflow_notification_inbox')",[],|r|r.get(0)).unwrap();
    assert_eq!(old_tables, 0);
    drop(db);
    cleanup(f);
}

// MP-08 / MP-10: TTL edits govern new occurrences, never old deliveries or ACKs.
#[test]
fn reattach_shorter_ttl_preserves_pending_occurrence_and_duplicate_ack() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("ttl-source");
    let (b, _, bp) = f.workflow("ttl-target");
    let source = f.source(&a);
    let mut sub = f.attach(&source, &b, &bp);
    f.complete(
        &a,
        "old-seven-day",
        None,
        WorkflowRunStatus::Completed,
        "old",
    );
    let (_, old) = f.candidates(false).remove(0);
    sub.ttl_days = 1;
    let target =
        WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &bp, None).unwrap();
    f.store
        .notify(NotificationOperation::Attach {
            subscription: sub.clone(),
            target,
        })
        .unwrap();
    for expected in [
        WorkflowNotificationAck::Accepted,
        WorkflowNotificationAck::Duplicate,
    ] {
        let NotificationOutcome::Ack(actual) = f
            .store
            .notify(NotificationOperation::Accept {
                subscription: sub.clone(),
                envelope: old.clone(),
            })
            .unwrap()
        else {
            panic!("ACK")
        };
        assert_eq!(actual, expected);
        if expected == WorkflowNotificationAck::Accepted {
            f.queue(sub.clone(), old.clone());
            let snapshot = f.sessions.get_session(&f.session).unwrap();
            assert_eq!(
                snapshot
                    .workflow_queued_prompts()
                    .back()
                    .unwrap()
                    .publication_invocation()
                    .unwrap()
                    .input["deadline_ms"],
                old.deadline_ms
            );
        }
    }
    f.complete(&a, "new-one-day", None, WorkflowRunStatus::Completed, "new");
    let (_, new) = f
        .candidates(false)
        .into_iter()
        .find(|(_, e)| e.occurrence_id == "new-one-day")
        .unwrap();
    assert!(old.deadline_ms - new.deadline_ms > 5 * 86_400_000);
    cleanup(f);
}

// MP-08 / MP-10: generic provenance must not override the current terminal status.
#[test]
fn chained_completion_status_remains_authoritative_for_filters() {
    for (first, second, expected) in [
        (
            WorkflowRunStatus::Completed,
            WorkflowRunStatus::Failed,
            "failure",
        ),
        (
            WorkflowRunStatus::Failed,
            WorkflowRunStatus::Completed,
            "success",
        ),
    ] {
        let mut f = Fixture::new();
        let (a, _, _) = f.workflow("chain-a");
        let (b, _, bp) = f.workflow("chain-b");
        let (c, _, cp) = f.workflow("chain-c");
        let sa = f.source(&a);
        let sb = f.source(&b);
        let ab = f.attach(&sa, &b, &bp);
        let mut bc = f.attach(&sb, &c, &cp);
        bc.filters = serde_json::json!({"status":expected});
        let target =
            WorkflowNotificationTarget::resolve(&f.sessions, "local", &f.session, &cp, None)
                .unwrap();
        f.store
            .notify(NotificationOperation::Attach {
                subscription: bc,
                target,
            })
            .unwrap();
        f.complete(&a, "a-run", None, first, "first");
        let (_, env) = f.candidates(false).remove(0);
        f.store
            .notify(NotificationOperation::Accept {
                subscription: ab.clone(),
                envelope: env.clone(),
            })
            .unwrap();
        let queued_id = f.queue(ab, env);
        let item = f
            .sessions
            .get_session(&f.session)
            .unwrap()
            .workflow_queued_prompts()
            .back()
            .unwrap()
            .clone();
        let mut run = WorkflowRun::new(
            "b-run",
            &b,
            "endpoint",
            "node",
            None,
            item.publication_invocation().cloned(),
            vec![],
            vec![],
        );
        run.set_invocation_context(0, None, Some(queued_id), 0, None);
        run.set_final_output(
            Some(WorkflowOutputPayload::new("second", vec![])),
            Some(true),
            None,
            None,
        );
        run.set_status(second);
        let mut session = f.sessions.get_session(&f.session).unwrap();
        session.create_workflow_run(run);
        f.sessions.restore_session(session);
        f.persist();
        let outgoing = f
            .candidates(false)
            .into_iter()
            .find(|(_, e)| e.occurrence_id == "b-run")
            .expect("subscriber filtering on current status receives the chained completion")
            .1;
        assert_eq!(outgoing.fields["status"], expected);
        assert_eq!(
            outgoing.status,
            if second == WorkflowRunStatus::Failed {
                crate::local::WorkflowNotificationStatus::Failure
            } else {
                crate::local::WorkflowNotificationStatus::Success
            }
        );
        assert_eq!(
            outgoing.output.is_none(),
            second == WorkflowRunStatus::Failed
        );
        cleanup(f);
    }
}

// MP-08 / MP-10: opaque provenance can consume the output's envelope reserve.
#[test]
fn notification_prompt_header_is_reserved_before_source_persistence_and_target_ack() {
    use crate::session::WorkflowPublicationInvocationEnvelope;
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("bounded-source");
    let (b, _, bp) = f.workflow("bounded-target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    let header = "Workflow completion notification (untrusted data):\n";
    let mut env = WorkflowNotificationEnvelope {
        source_id: source.source_id.clone(),
        occurrence_id: "boundary".into(),
        output: Some(WorkflowOutputPayload::new("small output", vec![])),
        status: crate::local::WorkflowNotificationStatus::Success,
        subject: None,
        fields: serde_json::json!({"status":"success","opaque":""}),
        ancestry: vec![workflow_identity(&source.kernel_id, &f.session, &a)],
        deadline_ms: crate::session::unix_epoch_ms() + 7 * 86_400_000,
    };
    let capacity = MAX_PROMPT_BYTES - header.len() - encode(&env).unwrap().len();
    env.fields["opaque"] = serde_json::json!("x".repeat(capacity));
    assert_eq!(encode(&env).unwrap().len() + header.len(), MAX_PROMPT_BYTES);
    let mut oversized = env.clone();
    oversized.fields["opaque"] = serde_json::json!("x".repeat(capacity + header.len() - 1));
    assert_eq!(encode(&oversized).unwrap().len(), MAX_PROMPT_BYTES - 1);

    // Actual completion capture must commit a visible refusal, not an unsendable receipt.
    let invocation = WorkflowPublicationInvocationEnvelope {
        publication_id: "fixture-publication".into(),
        hook_id: None,
        invocation_id: "fixture-occurrence".into(),
        transport: "app_event".into(),
        endpoint_id: "endpoint".into(),
        queue_ref: None,
        input: serde_json::json!({"payload":{"metadata":oversized.fields}}),
        artifacts: vec![],
        mode: None,
        caller: serde_json::json!({"kind":"app_event","owner_id":"local"}),
    };
    let mut run = WorkflowRun::new(
        "boundary",
        &a,
        "endpoint",
        "node",
        None,
        Some(invocation),
        vec![],
        vec![],
    );
    run.set_final_output(env.output.clone(), Some(true), None, None);
    run.set_status(WorkflowRunStatus::Completed);
    let mut session = f.sessions.get_session(&f.session).unwrap();
    session.create_workflow_run(run);
    f.sessions.restore_session(session);
    f.persist();
    let pending = f.candidates(false);
    let diagnostics = f.store.notification_inventory("local").unwrap().2;
    // A different target's first admission must apply the same bound before ACK.
    let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
    db.execute(
        "DELETE FROM app_outbox WHERE source_kind='workflow_completion'",
        [],
    )
    .unwrap();
    drop(db);
    let refusal = f.store.notify(NotificationOperation::Accept {
        subscription: sub.clone(),
        envelope: oversized,
    });
    let accepted_after_refusal = f.candidates(true);
    // Boundary control: an exactly fitting rendered prompt is accepted and queued.
    env.occurrence_id = "boundary-control".into();
    // Occurrence ID is longer, so shrink provenance by that exact encoded difference.
    let excess = encode(&env).unwrap().len() + header.len() - MAX_PROMPT_BYTES;
    env.fields["opaque"] = serde_json::json!("x".repeat(capacity - excess));
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
        pending.is_empty(),
        "MP-08: source must not persist an envelope whose rendered prompt exceeds the bound"
    );
    assert!(diagnostics
        .iter()
        .any(|d| d.code == "workflow_notification_prompt_limit"));
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
    last.ttl_days = 1;
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
    env.output = Some(WorkflowOutputPayload::new("🦀".repeat(7000), vec![]));
    env.fields["opaque"] = serde_json::json!("界".repeat(10000));
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
fn security_f8_notification_receipts_redact_output_and_fields() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("source");
    let (b, _, bp) = f.workflow("target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    let synthetic = format!("ghp_{}", "a".repeat(36));
    f.complete(&a, "seed", None, WorkflowRunStatus::Completed, &synthetic);
    let (_, mut env) = f.candidates(false).pop().unwrap();
    assert_eq!(
        env.output.as_ref().unwrap().message(),
        crate::secret_redaction::redact_secrets(&synthetic),
        "MP-11 F8: source outbox must protect public output before forwarding"
    );
    env.occurrence_id = "inbound".into();
    env.output = Some(WorkflowOutputPayload::new(&synthetic, vec![]));
    env.fields = serde_json::json!({"password":"synthetic-not-a-secret","summary":"useful"});
    f.store
        .notify(NotificationOperation::Accept {
            subscription: sub.clone(),
            envelope: env.clone(),
        })
        .unwrap();
    let (_, stored) = f.candidates(true).pop().unwrap();
    assert_ne!(stored.fields["password"], env.fields["password"]);
    assert_eq!(stored.fields["summary"], "useful");
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
    env.fields["password"] = serde_json::json!("changed-synthetic-value");
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
fn security_f8_redaction_expansion_cannot_accept_oversize_receipts() {
    let mut f = Fixture::new();
    let (a, _, _) = f.workflow("source");
    let (b, _, bp) = f.workflow("target");
    let source = f.source(&a);
    let sub = f.attach(&source, &b, &bp);
    f.complete(&a, "seed", None, WorkflowRunStatus::Completed, "fixture");
    let (_, mut env) = f.candidates(false).pop().unwrap();
    env.occurrence_id = "redaction-expansion".into();
    env.fields = serde_json::json!({"items":(0..2500).map(|_|serde_json::json!({"password":"x"})).collect::<Vec<_>>()});
    assert!(encode_notification_envelope(&env).is_ok());
    assert!(
        f.store
            .notify(NotificationOperation::Accept {
                subscription: sub,
                envelope: env
            })
            .is_err(),
        "MP-11 F8: admission must bound the protected bytes actually persisted and delivered"
    );
    assert!(f.candidates(true).is_empty());
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
        env.output = Some(WorkflowOutputPayload::new(&synthetic, vec![]));
        env.subject = Some(synthetic);
        env.fields = serde_json::json!({"password":"synthetic-not-a-secret","summary":"useful"});
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
        assert_ne!(safe.subject, env.subject);
        assert_ne!(safe.fields["password"], env.fields["password"]);
        assert_eq!(safe.fields["summary"], "useful");
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
        env.fields["password"] = serde_json::json!("changed-synthetic-value");
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
