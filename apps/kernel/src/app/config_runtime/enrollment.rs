//! MP-08/MP-10/MP-11: Cloud enrollment keeps the OS owner's existing work.
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::DEFAULT_LOCAL_USER_ID;

impl DaemonApp {
    pub(crate) fn enroll_existing_local_state(&self) -> Result<(), DaemonError> {
        let Some(profile) = self.config.cloud_relay.as_ref() else {
            return Ok(());
        };
        let owner = profile.user_id.trim();
        if owner.is_empty() || owner == DEFAULT_LOCAL_USER_ID {
            return Ok(());
        }
        // Persist the account first, then idempotently adopt its local records.
        // Startup retries after any interrupted write; foreign-owned work is untouched.
        let mut sessions = self.sessions.write();
        for mut project in sessions.durable_projects() {
            if project.owner_user_id() != DEFAULT_LOCAL_USER_ID {
                continue;
            }
            project.enroll_local_owner(owner);
            self.durable_state.append_event(
                "project.updated",
                Some(project.id().into()),
                serde_json::json!({"project": &project}),
            )?;
            sessions.restore_projects(vec![project]);
        }
        let mut updated = Vec::new();
        for mut session in sessions.list_all_sessions() {
            if session.owner_user_id() != DEFAULT_LOCAL_USER_ID
                || session.host_daemon_id() != self.config.daemon_id
            {
                continue;
            }
            for mut agent in self.agents.get_session_agents(session.id()) {
                if agent.owner_user_id() != DEFAULT_LOCAL_USER_ID {
                    continue;
                }
                agent.set_owner_user_id(owner);
                self.durable_state.append_event(
                    "agent.updated",
                    Some(session.id().into()),
                    serde_json::json!({"agent": &agent}),
                )?;
                self.agents.restore_agent(agent);
            }
            for run in self
                .providers
                .write()
                .enroll_local_run_owner(session.id(), owner)?
            {
                self.update_provider_run_projection(run);
            }
            self.attachments
                .write()
                .enroll_local_session_owner(session.id(), owner);
            session.enroll_local_owner(owner);
            self.durable_state
                .enroll_gate_owner(DEFAULT_LOCAL_USER_ID, &session)?;
            self.durable_state.append_event("session.updated", Some(session.id().into()),
                serde_json::json!({"session": session.durable_runtime_snapshot(), "reason": "cloud_owner_enrolled"}))?;
            updated.push(sessions.restore_session(session));
        }
        drop(sessions);
        for session in updated {
            self.update_session_projection(session);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_enrollment_adopts_existing_work_and_preserves_collaborators_after_restart() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("enrollment-owner-restart");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path =
            Some(root.path().join("kernel/state.db").display().to_string());
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request())
            .unwrap();
        let (_, invite) = app
            .sessions
            .write()
            .create_session_invite(
                session.id(),
                "collaborator-invite".into(),
                DEFAULT_LOCAL_USER_ID.into(),
                None,
                Some(1),
                crate::session::CollaborationLevel::Private,
            )
            .unwrap();
        app.sessions
            .write()
            .join_session_invite(session.id(), invite.invite_id(), "collaborator".into(), 1)
            .unwrap();
        let trigger = app
            .sessions
            .write()
            .create_agent_schedule_trigger(
                session.id(),
                agent.id(),
                DEFAULT_LOCAL_USER_ID,
                crate::session::AgentSchedule::interval(
                    3600,
                    crate::session::AgentTriggerRepeat::Once,
                ),
                crate::session::AgentTriggerBusyPolicy::Queue,
                Some("once".into()),
            )
            .unwrap();
        let foreign_root = crate::test_support::TestWorktree::new("enrollment-foreign-owner");
        let (foreign, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                foreign_root
                    .session_request()
                    .with_owner_user_id("other-person"),
            )
            .unwrap();
        app.save_durable_state_snapshot().unwrap();
        // Simulate interruption after the normal enrollment record is persisted
        // and before local records are adopted. Startup must finish that work.
        app.config
            .persist_cloud_relay_profile(Some(crate::config::PersistedCloudRelayProfile {
                user_id: "enrolled-person".into(),
                realm_id: "realm".into(),
                kernel_id: Some(app.config.daemon_id.clone()),
                ..Default::default()
            }))
            .unwrap();
        let config = app.config.clone();
        drop(app);
        let app = DaemonApp::bootstrap(config).unwrap();
        let adopted = app.sessions.get_session(session.id()).unwrap();
        assert_eq!(adopted.owner_user_id(), "enrolled-person");
        assert!(!adopted.has_member(DEFAULT_LOCAL_USER_ID));
        assert_eq!(
            adopted
                .members()
                .iter()
                .find(|m| m.user_id() == "collaborator")
                .unwrap()
                .collaboration_level(),
            crate::session::CollaborationLevel::Private
        );
        assert_eq!(
            app.agents.get_agent(agent.id()).unwrap().owner_user_id(),
            "enrolled-person"
        );
        assert_eq!(
            adopted
                .agent_triggers()
                .iter()
                .find(|t| t.id() == trigger.id())
                .unwrap()
                .created_by_user_id(),
            Some("enrolled-person")
        );
        assert_eq!(
            app.sessions
                .get_project(session.project_id())
                .unwrap()
                .owner_user_id(),
            "enrolled-person"
        );
        assert_eq!(
            app.sessions
                .get_session(foreign.id())
                .unwrap()
                .owner_user_id(),
            "other-person"
        );
        assert!(!adopted.has_member("other-person"));
        let config = app.config.clone();
        drop(app);
        let app = DaemonApp::bootstrap(config).unwrap();
        let restored = app.sessions.get_session(session.id()).unwrap();
        assert_eq!(restored.owner_user_id(), "enrolled-person");
        assert_eq!(restored.agent_triggers().len(), 1);
    }

    fn gated_admission(
        app: &DaemonApp,
        session_id: &str,
        workflow_id: &str,
        occurrence: &str,
    ) -> crate::durable_state::workflow_gate::Admission {
        let session = app.sessions.get_session(session_id).unwrap();
        let workflow = session
            .workflows()
            .iter()
            .find(|w| w.id() == workflow_id)
            .unwrap()
            .clone();
        let queue = session
            .workflow_prompt_queues()
            .iter()
            .find(|q| q.workflow_id() == workflow_id)
            .unwrap()
            .id()
            .to_string();
        let template =
            crate::session::WorkflowQueuedPrompt::new(crate::session::WorkflowQueuedPromptInput {
                id: format!("queued-{occurrence}"),
                queue_id: queue,
                workflow_id: workflow_id.into(),
                endpoint_id: "endpoint".into(),
                prompt: Some("gated".into()),
                publication_invocation: None,
                source: crate::session::WorkflowQueuedPromptSource::Manual,
                schedule_id: None,
            });
        crate::durable_state::workflow_gate::Admission::from_prompt(
            &session,
            &workflow,
            template,
            "binding".into(),
            chariox_app_runtime::workflow_gate::TriggerKind::Manual,
            occurrence.into(),
            crate::session::unix_epoch_ms() + 120_000,
            None,
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn cloud_enrollment_reowns_gate_entries_and_admits_for_the_enrolled_owner() {
        use crate::durable_state::workflow_gate::{Operation, Outcome};
        use chariox_app_runtime::workflow_gate as gate;
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("enrollment-gate-owner");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.state.path =
            Some(root.path().join("kernel/state.db").display().to_string());
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request())
            .unwrap();
        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("gated".into()))
            .unwrap();
        let component = b"gate-component".to_vec();
        let definition = gate::Definition {
            abi: gate::ABI.into(),
            module_sha256: gate::sha256(&component),
            config: Default::default(),
            config_sha256: gate::sha256(b"{}"),
        };
        let mut gated = app.sessions.get_session(session.id()).unwrap();
        let w = gated.workflow_mut(workflow.id()).unwrap();
        w.add_endpoint(crate::session::WorkflowEndpointDefinition::new(
            "endpoint", None, "triage",
        ));
        w.set_gate(Some(definition.clone()));
        let gated = app.sessions.write().restore_session(gated);
        app.durable_state
            .persist_workflow_runtime_transition(&gated, "test_gate")
            .unwrap();
        app.durable_state
            .gate(Operation::Put {
                owner: DEFAULT_LOCAL_USER_ID.into(),
                digest: definition.module_sha256.clone(),
                component,
            })
            .unwrap();
        let accept = |app: &DaemonApp, occurrence: &str| match app.durable_state.gate(
            Operation::Accept(Box::new(gated_admission(
                app,
                session.id(),
                workflow.id(),
                occurrence,
            ))),
        ) {
            Ok(Outcome::Entry(entry)) => Ok(entry),
            Ok(_) => panic!("accept returns an entry"),
            Err(error) => Err(error.to_string()),
        };
        let first = accept(&app, "first").unwrap();
        accept(&app, "second").unwrap();
        app.config
            .persist_cloud_relay_profile(Some(crate::config::PersistedCloudRelayProfile {
                user_id: "enrolled-person".into(),
                realm_id: "realm".into(),
                kernel_id: Some(app.config.daemon_id.clone()),
                ..Default::default()
            }))
            .unwrap();
        let config = app.config.clone();
        drop(app);
        let app = DaemonApp::bootstrap(config).unwrap();
        let entries = |app: &DaemonApp, owner: &str| {
            app.durable_state
                .gate_entries(owner, session.id(), workflow.id(), None, 100)
                .unwrap()
                .0
                .len()
        };
        assert_eq!(entries(&app, "enrolled-person"), 2);
        assert_eq!(entries(&app, DEFAULT_LOCAL_USER_ID), 0);
        match app
            .durable_state
            .gate(Operation::Cancel {
                owner: "enrolled-person".into(),
                id: first.entry_id.clone(),
                reason: "owner".into(),
            })
            .unwrap()
        {
            Outcome::Entry(entry) => assert_eq!(
                entry.disposition,
                crate::durable_state::workflow_gate::Disposition::Cancelled
            ),
            _ => panic!("cancel returns the entry"),
        }
        accept(&app, "after-enrollment").unwrap();
        // The enrolled endpoint owner is durable, not only in this process.
        let config = app.config.clone();
        drop(app);
        let app = DaemonApp::bootstrap(config).unwrap();
        accept(&app, "after-restart").unwrap();
        assert_eq!(entries(&app, "enrolled-person"), 4);
    }

    /// Every owner-keyed table is either adopted by enrollment or named here
    /// with the reason it is not. A new owner-keyed table fails until classified.
    #[test]
    fn every_owner_keyed_table_is_classified_for_enrollment() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("enrollment-owner-tables");
        let path = root.path().join("kernel/state.db");
        let store = crate::durable_state::DurableKernelStateStore::open(path.clone()).unwrap();
        drop(store);
        let db = rusqlite::Connection::open(path).unwrap();
        let mut q = db
            .prepare("SELECT m.name FROM sqlite_master m JOIN pragma_table_info(m.name) c WHERE m.type='table' AND c.name='owner_id' ORDER BY m.name")
            .unwrap();
        let tables = q
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .unwrap();
        // Adopted by `enroll_gate_owner`.
        const ADOPTED: &[&str] = &[
            "workflow_gate_entries",
            "workflow_gate_lanes",
            "workflow_gate_modules",
            "workflow_gate_state",
        ];
        // owner_id is the kernel (host daemon) id, which enrollment keeps.
        const KERNEL_KEYED: &[&str] = &[
            "durable_event_delivery_receipts",
            "durable_room_environments",
            "durable_state_metadata",
            "durable_state_owner_checkpoint_entities",
            "durable_state_owner_checkpoint_manifest",
            "durable_workflow_dispatch_intents",
            "durable_workflow_hot_entities",
            "durable_workflow_runs",
            "retired_sessions",
        ];
        // Kept under the identity a Room made before enrollment keeps for its owner
        // (`local`): the enrolled owner reaches them as that Room's owner
        // (`acts_for_room_owner`, `room_member_user_id_in`), so they are not re-owned.
        const KEPT_FOR_ROOM: &[&str] = &[
            "workflow_notification_exit_sources",
            "workflow_notification_generations",
            "workflow_notification_grants",
            "workflow_notification_remote_installs",
            "workflow_notification_source_cache",
            "workflow_notification_source_fence",
        ];
        // User-keyed and NOT adopted by enrollment today: App installations,
        // consents, grants and publisher trust stay with the consenting owner,
        // and created resources follow them. Known gap, reported for an owner
        // decision; not silently exempt.
        const NOT_ADOPTED: &[&str] = &[
            "app_automations",
            "app_connection_grants",
            "app_deployment_consents",
            "app_dev_identities",
            "app_file_exports",
            "app_file_grants",
            "app_file_picks",
            "app_host_actions",
            "app_inbox",
            "app_inbox_routes",
            "app_installation_deployments",
            "app_installation_operations",
            "app_installation_stage_trust",
            "app_installations",
            "app_logs",
            "app_outbox",
            "app_outbox_replay_floors",
            "app_publisher_decisions",
            "app_publisher_keys",
            "app_publisher_operations",
            "app_release_versions",
            "app_restore_receipts",
            "app_validations",
            "app_wakes",
            "app_worker_lifecycle",
            "created_resources",
        ];
        let classified = ADOPTED
            .iter()
            .chain(KERNEL_KEYED)
            .chain(KEPT_FOR_ROOM)
            .chain(NOT_ADOPTED)
            .map(|t| t.to_string())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            tables.difference(&classified).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "classify each new owner_id table for Cloud enrollment"
        );
        assert_eq!(
            classified.difference(&tables).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "drop classifications for tables that no longer exist"
        );
    }
}
