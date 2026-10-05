//! MP-08 / MP-10: focused real single-writer SQLite drills; no provider/network.
use super::*;
use crate::agent::{AgentInstance, GridPosition};
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
        let root =
            std::env::temp_dir().join(format!("chariox-wfnotify-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let mut sessions = SessionService::new(&crate::config::DaemonConfig::for_tests());
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
            target_kernel_id: source.kernel_id.clone(),
            session_id: self.session.clone(),
            workflow_id: w.into(),
            publication_id: p.into(),
            endpoint_id: t.endpoint_id.clone(),
            queue_id: t.queue_id.clone(),
            ttl_days: 7,
            source_available: true,
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
fn completion_persists_before_router_restart_deduplicates_and_failed_runs_never_emit() {
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
            subscription_id: "cycle-tail".into(),
            source_id: src.source_id.clone(),
            owner_user_id: "local".into(),
            target_kernel_id: src.kernel_id.clone(),
            session_id: f.session.clone(),
            workflow_id: a.clone(),
            publication_id: ap.clone(),
            endpoint_id: t.target().endpoint_id.clone(),
            queue_id: t.target().queue_id.clone(),
            ttl_days: 7,
            source_available: true,
        };
        let db = Connection::open(f.root.join("kernel.sqlite")).unwrap();
        db.execute(
            "INSERT OR REPLACE INTO workflow_notification_subscriptions VALUES (?1,?2,?3,?4,?5)",
            params![
                sub.subscription_id,
                sub.source_id,
                sub.owner_user_id,
                workflow_identity(&src.kernel_id, &f.session, &a),
                encode(&sub).unwrap()
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
            f.store
                .notify(NotificationOperation::Accept {
                    subscription: s.clone(),
                    envelope: e.clone(),
                })
                .unwrap();
            let q = f.queue(s, e);
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
            "SELECT state,envelope_json FROM workflow_notification_inbox",
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
