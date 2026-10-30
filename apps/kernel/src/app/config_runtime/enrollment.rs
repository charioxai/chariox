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
                3600,
                crate::session::AgentTriggerRepeat::Once,
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
}
