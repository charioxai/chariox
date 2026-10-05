//! One kernel notification router. Local delivery uses durable inbox admission;
//! Both local and authenticated E2EE peer delivery feed the App receipt store.
use super::{KernelRuntimeOwnedState, KernelRuntimeState};
use crate::durable_state::{
    notification_target::WorkflowNotificationTarget,
    workflow_notifications::{
        self as store, NotificationOperation, NotificationOutcome, PreparedNotification,
        SourceAdmission,
    },
};
use crate::{error::DaemonError, local::*, session::SessionService};
use std::collections::BTreeSet;

pub(super) enum NotificationRoute {
    Local,
    Peer,
}
fn route(home_kernel: &str, target_kernel: &str) -> NotificationRoute {
    if home_kernel == target_kernel {
        NotificationRoute::Local
    } else {
        NotificationRoute::Peer
    }
}
pub(super) fn source_available(
    sessions: &SessionService,
    source: &WorkflowNotificationSource,
) -> bool {
    sessions.get_session(&source.session_id).is_ok_and(|s| {
        s.owner_user_id() == source.owner_user_id && s.host_daemon_id() == source.kernel_id
    }) && sessions
        .resolve_workflow_ref(&source.session_id, &source.workflow_id)
        .is_ok()
}
pub(super) fn summary(source: &WorkflowNotificationSource) -> WorkflowNotificationSourceSummary {
    let mut fields = ["subject", "status"].map(str::to_owned).to_vec();
    fields.extend(source.output_fields.clone());
    WorkflowNotificationSourceSummary {
        source_id: source.source_id.clone(),
        kernel_id: source.kernel_id.clone(),
        session_id: source.session_id.clone(),
        workflow_id: source.workflow_id.clone(),
        name: source.name.clone(),
        events: WorkflowNotificationEvents::Both,
        fields,
        available: source.available,
    }
}
impl KernelRuntimeState {
    pub(crate) fn execute_workflow_notification_request(
        &self,
        request: LocalDaemonRequest,
        owner: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.owned
            .durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let sessions = self.owned.session_store.read();
                match request {
                    LocalDaemonRequest::RegisterWorkflowNotificationSource(request) => {
                        let session = sessions.get_session(&request.session_id)?;
                        if session.owner_user_id() != owner {
                            return Err(store::error("source not available"));
                        }
                        let workflow =
                            sessions.resolve_workflow_ref(session.id(), &request.workflow_ref)?;
                        let source = WorkflowNotificationSource {
                            source_id: format!("wf_source_{:032x}", rand::random::<u128>()),
                            owner_user_id: owner.into(),
                            kernel_id: session.host_daemon_id().into(),
                            session_id: session.id().into(),
                            workflow_id: workflow.id().into(),
                            enabled: request.enabled,
                            available: true,
                            name: workflow.alias().unwrap_or(workflow.id()).into(),
                            output_fields: request.output_fields.unwrap_or_else(|| {
                                self.owned
                                    .durable_state_store
                                    .notification_inventory(owner)
                                    .ok()
                                    .and_then(|(sources, _, _)| {
                                        sources
                                            .into_iter()
                                            .find(|s| {
                                                s.workflow_id == workflow.id()
                                                    && s.session_id == session.id()
                                            })
                                            .map(|s| s.output_fields)
                                    })
                                    .unwrap_or_default()
                            }),
                        };
                        match self.owned.durable_state_store.notify(
                            NotificationOperation::Register(SourceAdmission {
                                source,
                                workflow_json: serde_json::to_string(&workflow)
                                    .map_err(|_| store::error("source encoding"))?,
                            }),
                        )? {
                            NotificationOutcome::Source(source) => {
                                Ok(LocalDaemonResponse::WorkflowNotificationSourceRegistered {
                                    source,
                                })
                            }
                            _ => Err(store::error("unexpected notification response")),
                        }
                    }
                    LocalDaemonRequest::AttachWorkflowNotification(request) => {
                        let (sources, _, _) = self
                            .owned
                            .durable_state_store
                            .notification_inventory(owner)?;
                        let source = sources
                            .into_iter()
                            .find(|s| {
                                s.source_id == request.source_id
                                    && s.enabled
                                    && source_available(&sessions, s)
                            })
                            .or_else(|| {
                                self.owned
                                    .durable_state_store
                                    .notification_cached_sources(owner)
                                    .ok()?
                                    .into_iter()
                                    .find(|s| s.source_id == request.source_id && s.available)
                                    .map(|s| WorkflowNotificationSource {
                                        source_id: s.source_id,
                                        owner_user_id: owner.into(),
                                        kernel_id: s.kernel_id,
                                        session_id: s.session_id,
                                        workflow_id: s.workflow_id,
                                        name: s.name,
                                        enabled: true,
                                        available: true,
                                        output_fields: vec![],
                                    })
                            })
                            .ok_or_else(|| store::error("source not available"))?;
                        let session = sessions.get_session(&request.session_id)?;
                        if session.owner_user_id() != owner {
                            return Err(store::error("notification target not owner"));
                        }
                        let target = WorkflowNotificationTarget::resolve(
                            &sessions,
                            owner,
                            session.id(),
                            &request.publication_ref,
                            request.queue_ref.as_deref(),
                        )
                        .map_err(|e| store::error(e.to_string()))?;
                        let publication = sessions.resolve_workflow_publication_ref(
                            session.id(),
                            &request.publication_ref,
                        )?;
                        let t = target.target();
                        let subscription = WorkflowNotificationSubscription {
                            delivery_mode: request.delivery_mode,
                            subscription_id: format!(
                                "wf_subscription_{:032x}",
                                rand::random::<u128>()
                            ),
                            source_id: source.source_id.clone(),
                            owner_user_id: owner.into(),
                            target_kernel_id: session.host_daemon_id().into(),
                            target_kind: WorkflowNotificationTargetKind::WorkflowEndpoint,
                            session_id: session.id().into(),
                            workflow_id: publication.workflow_id().into(),
                            publication_id: t.publication_id.clone(),
                            endpoint_id: t.endpoint_id.clone(),
                            queue_id: t.queue_id.clone(),
                            ttl_days: request.ttl_days,
                            source_available: true,
                            source_kernel_id: source.kernel_id.clone(),
                            events: request.events,
                            filters: request.filters,
                        };
                        let operation = if source.kernel_id == session.host_daemon_id() {
                            NotificationOperation::Attach {
                                subscription,
                                target,
                            }
                        } else {
                            NotificationOperation::RemoteAttach { subscription }
                        };
                        match self.owned.durable_state_store.notify(operation)? {
                            NotificationOutcome::Subscription(subscription) => {
                                Ok(LocalDaemonResponse::WorkflowNotificationAttached {
                                    subscription,
                                })
                            }
                            _ => Err(store::error("unexpected notification response")),
                        }
                    }
                    LocalDaemonRequest::ListWorkflowNotifications(request) => {
                        sessions.get_session(&request.session_id)?;
                        let (mut sources, mut subscriptions, diagnostics) = self
                            .owned
                            .durable_state_store
                            .notification_inventory(owner)?;
                        for source in &mut sources {
                            source.available = source_available(&sessions, source);
                        }
                        let cached = self
                            .owned
                            .durable_state_store
                            .notification_cached_sources(owner)?;
                        for sub in &mut subscriptions {
                            sub.source_available = sources
                                .iter()
                                .any(|s| s.source_id == sub.source_id && s.available && s.enabled)
                                || cached
                                    .iter()
                                    .any(|s| s.source_id == sub.source_id && s.available);
                        }
                        subscriptions.retain(|s| {
                            s.session_id == request.session_id
                                && s.target_kernel_id
                                    == self.owned.config_projection.snapshot().daemon_id
                        });
                        let mut sources = sources
                            .iter()
                            .filter(|s| s.enabled)
                            .map(summary)
                            .collect::<Vec<_>>();
                        sources.extend(cached);
                        Ok(LocalDaemonResponse::WorkflowNotifications {
                            sources,
                            subscriptions,
                            diagnostics,
                        })
                    }
                    _ => Err(store::error("unsupported notification request")),
                }
            })
    }
}
impl KernelRuntimeOwnedState {
    /// The existing bounded notification/App pass retains blocking ownership.
    /// No network, App worker, AEDS or registry dependency is involved.
    pub(super) fn route_workflow_notifications(&self) -> (BTreeSet<String>, bool) {
        let mut queued_sessions = BTreeSet::new();
        let now = crate::session::unix_epoch_ms();
        if self
            .durable_state_store
            .notify(NotificationOperation::Sweep { now })
            .is_err()
        {
            return (queued_sessions, false);
        }
        if !self.publication_activation.is_active() {
            return (queued_sessions, false);
        }
        // Expire already queued work through the ordinary durable session path;
        // dispatch admission also checks the deadline, including between ticks.
        let _ = self
            .durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let mut sessions = self.session_store.write();
                for mut session in sessions.list_sessions() {
                    if session.expire_workflow_notification_prompts(now) {
                        self.durable_state_store
                            .persist_workflow_runtime_transition(
                                &session,
                                "workflow_notification_expired",
                            )?;
                        sessions.restore_session(session);
                    }
                }
                Ok(())
            });
        let home = self.config_projection.snapshot().daemon_id;
        for accepted in [false, true] {
            let Ok(candidates) = self
                .durable_state_store
                .notification_candidates(accepted, now, 8)
            else {
                continue;
            };
            for (sub, env) in candidates {
                // Rotate failed/busy targets before attempting, so they never block
                // another subscription and retry never mutates its original deadline.
                let _ = self
                    .durable_state_store
                    .notify(NotificationOperation::Retry {
                        subscription_id: sub.subscription_id.clone(),
                        source_id: env.source_id.clone(),
                        occurrence_id: env.occurrence_id.clone(),
                        accepted,
                        at: now.saturating_add(1000),
                    });
                if !matches!(
                    route(&home, &sub.target_kernel_id),
                    NotificationRoute::Local
                ) {
                    continue;
                }
                let operation = self
                    .durable_state_store
                    .with_workflow_runtime_transition_lock(|| {
                        let mut sessions = self.session_store.write();
                        let (sources, _, _) = self
                            .durable_state_store
                            .notification_inventory(&sub.owner_user_id)?;
                        // Deletion/transfer leave pending rows alone; they naturally expire.
                        if !accepted
                            && !sources.iter().any(|s| {
                                s.source_id == env.source_id && source_available(&sessions, s)
                            })
                        {
                            return Ok(false);
                        }
                        let target = WorkflowNotificationTarget::resolve(
                            &sessions,
                            &sub.owner_user_id,
                            &sub.session_id,
                            &sub.publication_id,
                            Some(&sub.queue_id),
                        )
                        .map_err(|e| store::error(e.to_string()))?;
                        if sessions.get_session(&sub.session_id)?.owner_user_id()
                            != sub.owner_user_id
                            || target.target().endpoint_id != sub.endpoint_id
                        {
                            return Err(store::error("notification target changed"));
                        }
                        if accepted {
                            let prepared = PreparedNotification::prepare(
                                &mut sessions,
                                sub.clone(),
                                env.clone(),
                            )?;
                            let after = prepared.after.clone();
                            self.durable_state_store
                                .notify(NotificationOperation::Queue(Box::new(prepared)))?;
                            sessions.restore_session(after);
                            Ok(true)
                        } else {
                            match self.durable_state_store.notify(
                                NotificationOperation::Accept {
                                    subscription: sub.clone(),
                                    envelope: env.clone(),
                                },
                            )? {
                                NotificationOutcome::Ack(_) => {
                                    // Local acceptance shares the source receipt: Accepted
                                    // already promoted it. Terminal ACKs must retire any
                                    // remaining retryable source work, just like peer ACKs.
                                    self.durable_state_store.notify(
                                        NotificationOperation::Acknowledge {
                                            subscription_id: sub.subscription_id.clone(),
                                            occurrence_id: env.occurrence_id.clone(),
                                        },
                                    )?;
                                    Ok(false)
                                }
                                _ => Err(store::error("unexpected notification ACK")),
                            }
                        }
                    });
                if matches!(operation, Ok(true)) {
                    queued_sessions.insert(sub.session_id.clone());
                    let _ = self.session_snapshot(&sub.session_id);
                }
            }
        }
        // Includes delayed retries to keep the existing timer's one-second floor.
        let more = self
            .durable_state_store
            .notification_has_pending()
            .unwrap_or(false);
        (queued_sessions, more)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::workflow_notifications::tests::{cleanup, Fixture};
    use crate::{app::DaemonApp, config::DaemonConfig, runtime::router::CommandRouter};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    fn runtime(f: &mut Fixture) -> KernelRuntimeState {
        // Bootstrap only local kernel services with every mutable root owned by this fixture.
        let mut config = DaemonConfig::for_tests();
        config.user_config.state.path = Some(f.root.join("runtime/state.db").display().to_string());
        config.user_config_path = f.root.join("runtime/config.toml");
        config = config.with_session_history_root(f.root.join("history"));
        config.user_config.history.operational.path =
            Some(f.root.join("operational.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(f.root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(f.root.join("artifacts/index.db").display().to_string());
        let app = DaemonApp::bootstrap(config).unwrap();
        let runtime =
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
        f.store = runtime.owned.durable_state_store.clone();
        for session in f.sessions.list_sessions() {
            for agent in session.agents() {
                runtime.owned.agent_store.restore_agent(agent.clone());
            }
            runtime
                .owned
                .session_store
                .write()
                .restore_session(session.clone());
            f.store
                .persist_workflow_runtime_transition(&session, "notification router fixture")
                .unwrap();
        }
        runtime
    }
    // MP-08 / MP-10: binding edits terminally settle already-captured local work.
    #[test]
    fn same_kernel_router_settles_filtered_pending_occurrences_after_binding_edits() {
        for edit_events in [false, true] {
            let mut f = Fixture::new();
            let (a, _, _) = f.workflow("source");
            let (_, _, bp) = f.workflow("target");
            let kernel = runtime(&mut f);
            let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = kernel
                .execute_workflow_notification_request(
                    LocalDaemonRequest::RegisterWorkflowNotificationSource(
                        RegisterWorkflowNotificationSourceRequest {
                            session_id: f.session.clone(),
                            workflow_ref: a.clone(),
                            enabled: true,
                            output_fields: None,
                        },
                    ),
                    "local",
                )
                .unwrap()
            else {
                panic!()
            };
            let mut attach = AttachWorkflowNotificationRequest {
                delivery_mode: NotificationDeliveryMode::Queue,
                session_id: f.session.clone(),
                source_id: source.source_id,
                publication_ref: bp,
                queue_ref: None,
                ttl_days: 7,
                events: WorkflowNotificationEvents::Both,
                filters: serde_json::Value::Null,
            };
            kernel
                .execute_workflow_notification_request(
                    LocalDaemonRequest::AttachWorkflowNotification(attach.clone()),
                    "local",
                )
                .unwrap();
            f.complete(
                &a,
                "pending-before-edit",
                None,
                crate::session::WorkflowRunStatus::Completed,
                "opaque output",
            );
            kernel
                .owned
                .session_store
                .write()
                .restore_session(f.sessions.get_session(&f.session).unwrap());
            assert_eq!(f.candidates(false).len(), 1);
            if edit_events {
                attach.events = WorkflowNotificationEvents::Failure;
            } else {
                attach.filters = serde_json::json!({"status":"failure"});
            }
            kernel
                .execute_workflow_notification_request(
                    LocalDaemonRequest::AttachWorkflowNotification(attach.clone()),
                    "local",
                )
                .unwrap();
            let (queued, more) = kernel.owned.route_workflow_notifications();
            assert!(queued.is_empty());
            assert!(!more, "terminal Filtered ACK must release source backlog");
            assert!(kernel
                .owned
                .session_store
                .get_session(&f.session)
                .unwrap()
                .workflow_queued_prompts()
                .is_empty());
            let db = rusqlite::Connection::open(f.root.join("runtime/state.db")).unwrap();
            let state: String = db.query_row(
                "SELECT state FROM app_outbox WHERE source_kind='workflow_completion' AND occurrence_id='pending-before-edit'", [], |r| r.get(0)).unwrap();
            assert_eq!(state, "delivered");
            drop(db);
            drop(kernel);
            // Release the fixture's last cloned owner before reopening runtime state.
            f.store = crate::durable_state::DurableKernelStateStore::open_owned(
                f.root.join("kernel.sqlite"),
            )
            .unwrap();
            let kernel = runtime(&mut f);
            assert!(!f.store.notification_has_pending().unwrap());
            assert!(kernel.owned.route_workflow_notifications().0.is_empty());
            // A subsequent matching completion still flows through the same router.
            f.complete(
                &a,
                "matching-after-edit",
                None,
                crate::session::WorkflowRunStatus::Failed,
                "private failure detail",
            );
            kernel
                .owned
                .session_store
                .write()
                .restore_session(f.sessions.get_session(&f.session).unwrap());
            assert!(kernel
                .owned
                .route_workflow_notifications()
                .0
                .contains(&f.session));
            let snapshot = kernel.owned.session_store.get_session(&f.session).unwrap();
            assert_eq!(snapshot.workflow_queued_prompts().len(), 1);
            let prompt = snapshot
                .workflow_queued_prompts()
                .front()
                .unwrap()
                .prompt()
                .unwrap();
            assert!(prompt.contains("failure") && !prompt.contains("private failure detail"));
            drop(kernel);
            cleanup(f);
        }
    }

    #[test]
    fn same_kernel_router_queues_ordinary_prompt_and_missing_source_is_visible() {
        let mut f = Fixture::new();
        let (a, _, _) = f.workflow("a");
        let (b, _, bp) = f.workflow("b");
        let runtime = runtime(&mut f);
        let register = LocalDaemonRequest::RegisterWorkflowNotificationSource(
            RegisterWorkflowNotificationSourceRequest {
                session_id: f.session.clone(),
                workflow_ref: a.clone(),
                enabled: true,
                output_fields: None,
            },
        );
        assert!(runtime
            .execute_workflow_notification_request(register.clone(), "another-user")
            .is_err());
        let LocalDaemonResponse::WorkflowNotificationSourceRegistered { source } = runtime
            .execute_workflow_notification_request(register, "local")
            .unwrap()
        else {
            panic!()
        };
        let attach =
            LocalDaemonRequest::AttachWorkflowNotification(AttachWorkflowNotificationRequest {
                delivery_mode: crate::local::NotificationDeliveryMode::Queue,
                session_id: f.session.clone(),
                source_id: source.source_id,
                publication_ref: bp,
                queue_ref: None,
                ttl_days: 7,
                events: WorkflowNotificationEvents::Both,
                filters: serde_json::Value::Null,
            });
        assert!(runtime
            .execute_workflow_notification_request(attach.clone(), "another-user")
            .is_err());
        runtime
            .execute_workflow_notification_request(attach, "local")
            .unwrap();
        f.complete(
            &a,
            "finished",
            None,
            crate::session::WorkflowRunStatus::Completed,
            "kernel output",
        );
        runtime
            .owned
            .session_store
            .write()
            .restore_session(f.sessions.get_session(&f.session).unwrap());
        let (sessions, _) = runtime.owned.route_workflow_notifications();
        assert!(sessions.contains(&f.session));
        let session = runtime
            .owned
            .session_store
            .read()
            .get_session(&f.session)
            .unwrap();
        assert_eq!(session.workflow_queued_prompts().len(), 1);
        let queued = session.workflow_queued_prompts().front().unwrap();
        assert_eq!(queued.workflow_id(), b);
        assert!(queued.prompt().unwrap().contains("kernel output"));
        assert_eq!(
            queued.source(),
            crate::session::WorkflowQueuedPromptSource::Event
        );
        assert!(matches!(route("one", "two"), NotificationRoute::Peer));
        // Simulate restart at the queue-commit boundary: only durable hot state
        // is used for the next pump, without a source receipt to re-admit.
        let hot = f
            .store
            .load_workflow_hot_states(session.host_daemon_id())
            .unwrap();
        let (_, state) = hot.into_iter().find(|(id, _)| id == &f.session).unwrap();
        assert_eq!(state.workflow_queued_prompts.len(), 1);
        let recovered = runtime
            .owned
            .durable_state_store
            .pending_workflow_dispatch_sessions(session.host_daemon_id(), None, 8)
            .unwrap();
        assert!(
            recovered.contains(&f.session),
            "MP-08 / MP-10: committed receipt must recover its queue handoff"
        );
        // The actual bounded App pump must discover the persisted queue after
        // the source adapter's one-pass dispatch set has disappeared.
        let _blocker = runtime
            .owned
            .workspace_coordinator
            .acquire_worktree_write_claim(
                session.workspace_id(),
                session.worktree_id(),
                &f.session,
                None,
                "notification_handoff_restart",
            )
            .unwrap();
        runtime.fixture_app_event_pass();
        let recovered_session = runtime.owned.session_store.get_session(&f.session).unwrap();
        assert!(recovered_session
            .workflow_runs()
            .iter()
            .any(|run| run.workflow_id() == b));
        let mut transferred = runtime
            .owned
            .session_store
            .read()
            .get_session(&f.session)
            .unwrap();
        transferred.set_owner_user_id("another-user");
        runtime
            .owned
            .session_store
            .write()
            .restore_session(transferred);
        let LocalDaemonResponse::WorkflowNotifications { sources, .. } = runtime
            .execute_workflow_notification_request(
                LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                    session_id: f.session.clone(),
                }),
                "local",
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(!sources[0].available);
        let mut session = runtime
            .owned
            .session_store
            .read()
            .get_session(&f.session)
            .unwrap();
        session.set_owner_user_id("local");
        session.remove_workflow(&a);
        runtime
            .owned
            .session_store
            .write()
            .restore_session(session.clone());
        f.store
            .persist_workflow_runtime_transition(&session, "source deleted")
            .unwrap();
        let LocalDaemonResponse::WorkflowNotifications {
            sources,
            subscriptions,
            ..
        } = runtime
            .execute_workflow_notification_request(
                LocalDaemonRequest::ListWorkflowNotifications(ListWorkflowNotificationsRequest {
                    session_id: f.session.clone(),
                }),
                "local",
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(!sources[0].available);
        assert!(!subscriptions[0].source_available);
        drop(runtime);
        cleanup(f);
    }
}
