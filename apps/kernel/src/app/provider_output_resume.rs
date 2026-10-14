//! MP-08 / MP-10 / MP-11: persist only the current run's confirmed native identity.
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ProviderPromptSignalBatch, RuntimeProviderRun};

pub(crate) fn persist_resolved_resume_state(
    app: &mut DaemonApp,
    provider_run: &RuntimeProviderRun,
    poll_result: &ProviderPromptSignalBatch,
) -> Result<(), DaemonError> {
    let Some(resume_state) = poll_result.resolved_resume_state.as_ref() else {
        return Ok(());
    };
    // A substitute run reruns one turn; it never becomes the agent's profile.
    let Some(agent_id) = provider_run
        .agent_instance_id()
        .filter(|_| provider_run.turn_substitute().is_none())
    else {
        return Ok(());
    };
    let durable_state_store = app.durable_state_store();
    {
        // MP-08 / MP-10 / MP-11: profile control retires the provider run
        // before taking the agent-store lock. Keep that same lock order here
        // so a source identity cannot commit onto a destination profile.
        let providers = app.providers.read();
        if providers
            .get_run_for_agent(provider_run.session_id(), agent_id)
            .is_none_or(|current| current.id() != provider_run.id())
        {
            return Ok(());
        }
        #[cfg(test)]
        AFTER_RUN_CHECK.with(|gate| {
            if let Some(gate) = gate.borrow_mut().take() {
                gate();
            }
        });
        app.agents.set_agent_provider_resume_state_durably(
            &durable_state_store,
            agent_id,
            resume_state.clone(),
            provider_run.id(),
        )?;
    }
    // Snapshot rebuilding reads the provider store again, outside the guard.
    let _ = crate::app::KernelSessionReadService::new(app)
        .session_snapshot(provider_run.session_id())?;
    Ok(())
}

#[cfg(test)]
thread_local! {
    static AFTER_RUN_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = Default::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_run_metadata_cannot_replace_current_native_resume() {
        let root = crate::test_support::TestWorktree::new("native-resume-fence");
        let mut app = crate::test_support::bootstrap_authenticated_app(
            crate::config::DaemonConfig::for_tests(),
        )
        .unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request())
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "default",
            "haiku",
        )
        .with_agent_id(agent.id());
        let make_run = |id: &str| {
            let mut run = RuntimeProviderRun::new(
                id,
                &request,
                crate::provider::ProviderLaunchResult {
                    endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                    process_label: "metadata-only-native-resume-test".into(),
                    pty_target: None,
                    pty_program: None,
                    pty_args: vec![],
                    pty_env: Default::default(),
                    pty_env_remove: vec![],
                    working_directory: None,
                    structured_endpoint: None,
                },
            );
            run.mark_running();
            run
        };
        let mut old = make_run("obsolete-native-run");
        old.mark_ended();
        app.providers_mut().insert_run_for_test(old.clone());
        app.providers_mut()
            .insert_run_for_test(make_run("current-native-run"));
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                "claude-headless",
                Some("haiku".into()),
                None,
                Some("default".into()),
                crate::provider::ProviderResumeState::from_claude_session_id(
                    "current-native-session",
                ),
            )
            .unwrap();
        assert_eq!(
            app.providers()
                .get_run_for_agent(session.id(), agent.id())
                .unwrap()
                .id(),
            "current-native-run",
            "fixture must select the replacement run before testing stale metadata",
        );
        persist_resolved_resume_state(
            &mut app,
            &old,
            &ProviderPromptSignalBatch {
                resolved_resume_state: Some(
                    crate::provider::ProviderResumeState::from_claude_session_id(
                        "obsolete-native-session",
                    ),
                ),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            app.agents()
                .get_agent(agent.id())
                .unwrap()
                .provider_resume_state()
                .claude_session_id(),
            Some("current-native-session")
        );
    }
    // MP-08 / MP-10 / MP-11: retire + commit the destination while a native
    // identity write is paused after its ownership check, exactly as profile control does.
    #[test]
    fn concurrent_profile_change_wins_over_source_native_metadata() {
        use std::sync::mpsc;
        use std::time::Duration;
        let root = crate::test_support::TestWorktree::new("native-resume-concurrent-fence");
        let mut app = crate::test_support::bootstrap_authenticated_app(
            crate::config::DaemonConfig::for_tests(),
        )
        .unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request())
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "source-account",
            "haiku",
        )
        .with_agent_id(agent.id());
        let mut run = RuntimeProviderRun::new(
            "source-native-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "metadata-only-concurrent-native-resume-test".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                "claude-headless",
                Some("haiku".into()),
                None,
                Some("source-account".into()),
                crate::provider::ProviderResumeState::from_claude_session_id(
                    "source-native-session",
                ),
            )
            .unwrap();
        let providers = app.providers().clone();
        let agents = app.agents().clone();
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let run_id = run.id().to_string();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (commit_tx, commit_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            start_rx.recv().unwrap();
            attempt_tx.send(()).unwrap();
            providers
                .terminate_run_provider_only(&session_id, &run_id)
                .unwrap();
            agents
                .set_agent_runtime_profile_with_account_profile(
                    &agent_id,
                    "claude-headless",
                    Some("sonnet".into()),
                    Some("high".into()),
                    Some("destination-account".into()),
                    Default::default(),
                )
                .unwrap();
            let _ = commit_tx.send(());
        });
        AFTER_RUN_CHECK.with(|gate| {
            *gate.borrow_mut() = Some(Box::new(move || {
                start_tx.send(()).unwrap();
                attempt_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                // A linearized implementation keeps retirement blocked until this
                // native write commits. The unfenced base commits the destination here.
                let _ = commit_rx.recv_timeout(Duration::from_millis(250));
            }))
        });
        persist_resolved_resume_state(
            &mut app,
            &run,
            &ProviderPromptSignalBatch {
                resolved_resume_state: Some(
                    crate::provider::ProviderResumeState::from_claude_session_id(
                        "late-source-session",
                    ),
                ),
                ..Default::default()
            },
        )
        .unwrap();
        writer.join().unwrap();
        let destination = app.agents().get_agent(agent.id()).unwrap();
        assert_eq!(
            destination.provider_account_profile(),
            "destination-account"
        );
        assert_eq!(destination.model(), Some("sonnet"));
        assert_eq!(destination.effort(), Some("high"));
        assert_eq!(
            destination.provider_resume_state().claude_session_id(),
            None
        );
    }
    // MP-08 / MP-10 / MP-11: native identity metadata describes execution,
    // while provider/model/effort/account remain the kernel's selected profile.
    #[test]
    fn native_identity_preserves_the_selected_profile() {
        let root = crate::test_support::TestWorktree::new("native-identity-selected-profile");
        let mut app = crate::test_support::bootstrap_authenticated_app(
            crate::config::DaemonConfig::for_tests(),
        )
        .unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(root.session_request())
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "source-account",
            "claude-haiku-4-5-20251001",
        )
        .with_agent_id(agent.id());
        let mut run = RuntimeProviderRun::new(
            "resolved-native-identity-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "metadata-only-selected-profile-test".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.agents_mut()
            .set_agent_runtime_profile_with_account_profile(
                agent.id(),
                "claude-headless",
                Some("haiku".into()),
                Some("high".into()),
                Some("source-account".into()),
                Default::default(),
            )
            .unwrap();
        persist_resolved_resume_state(
            &mut app,
            &run,
            &ProviderPromptSignalBatch {
                resolved_resume_state: Some(
                    crate::provider::ProviderResumeState::from_claude_session_id(
                        "confirmed-native-session",
                    ),
                ),
                ..Default::default()
            },
        )
        .unwrap();
        let selected = app.agents().get_agent(agent.id()).unwrap();
        assert_eq!(selected.provider(), "claude-headless");
        assert_eq!(selected.model(), Some("haiku"));
        assert_eq!(selected.effort(), Some("high"));
        assert_eq!(selected.provider_account_profile(), "source-account");
        assert_eq!(
            selected.provider_resume_state().claude_session_id(),
            Some("confirmed-native-session")
        );
    }
}
