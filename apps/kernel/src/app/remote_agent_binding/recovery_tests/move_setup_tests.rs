// MP-08 / MP-09 / MP-10 / MP-11: exercise setup through public Move admission.
use super::*;

async fn install_move_setup_definition(fixture: &Fixture, session_id: &str) {
    let home = fixture.home.lock().await;
    let session = home.sessions().get_session(session_id).unwrap();
    let mut agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    agent.set_provider("dev-stub");
    agent.set_account_profile(None);
    agent.set_model(Some("default".into()));
    home.agents().restore_agent(agent);
    let definition = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "origin": "user_authored",
        "source": "commands",
        "target_platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        "source_path": null,
        "inputs": [],
        "path_entries": [{"base": "preparation_home", "path": "move-bin"}],
        "setup_steps": [{"kind": "command", "command": "if [ -f .move-setup-gate ]; then touch .move-setup-waiting; while [ ! -f .move-setup-release ]; do sleep 0.01; done; fi; mkdir -p \"$HOME/move-bin\"; printf '#!/bin/sh\\nprintf move-tool-ready\\n' > \"$HOME/move-bin/move-tool\"; chmod +x \"$HOME/move-bin/move-tool\"; pwd > .move-setup-ready"}],
        "validation_commands": ["test -s .move-setup-ready; test \"$(move-tool)\" = move-tool-ready"]
    })).unwrap();
    home.sessions
        .write()
        .update_project_environment_definition(
            session.project_id(),
            definition,
            session.owner_user_id(),
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mk04_public_return_runs_stored_project_setup_before_clearing_remote_route() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    let (fixture, session, _) = crash_fixture().await;
    move_request(&fixture, &session, true).await;
    install_move_setup_definition(&fixture, &session).await;
    move_request(&fixture, &session, false).await;
    let home = fixture.home.lock().await;
    let agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    assert!(agent.remote_execution().is_none());
    let path = PathBuf::from(agent.worktree_id().unwrap());
    assert_eq!(
        std::fs::read_to_string(path.join(".move-setup-ready"))
            .unwrap()
            .trim(),
        path.to_str().unwrap()
    );
    assert!(home
        .agent_moves
        .for_agent(&session, &fixture.agent_id)
        .unwrap()
        .composition
        .readiness
        .unwrap()
        .environment_setup_operation_id
        .is_some());
}
#[tokio::test(flavor = "multi_thread")]
async fn mk04_public_move_keeps_prepared_provider_for_first_prompt() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    let (fixture, session, _) = crash_fixture().await;
    install_move_setup_definition(&fixture, &session).await;
    move_request(&fixture, &session, true).await;
    let home = fixture.home.lock().await;
    let remote = home
        .agents()
        .get_agent(&fixture.agent_id)
        .unwrap()
        .remote_execution()
        .unwrap()
        .clone();
    let mut worker = fixture.workers[0].lock().await;
    let lease = worker
        .leased_agents
        .get(&remote.leased_agent_id)
        .unwrap()
        .clone();
    let run = worker
        .providers()
        .get_run_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap();
    assert_eq!(
        run.state(),
        crate::provider::ProviderRunState::Running,
        "Move must retain the ordinary prepared run for its first prompt"
    );
    assert!(!run.read_only_discovery());
    let first_prompt_run = worker
        .ensure_prompt_provider_run_for_agent(&lease.backing_session_id, &lease.backing_agent_id)
        .unwrap();
    assert_eq!(
        first_prompt_run,
        run.id(),
        "ordinary prompt admission must reuse the prepared run"
    );
    let environment = run.pty_env();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", "test \"$(move-tool)\" = move-tool-ready"])
        .env_clear()
        .envs(environment)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the next turn must find the tool installed under preparation HOME"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn mk04_public_move_ready_does_not_ignore_provider_teardown_error() {
    use crate::provider::{ProviderLifecycleFailureInjection, ProviderLifecycleFailureStage};
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    let (fixture, session, path) = crash_fixture().await;
    install_move_setup_definition(&fixture, &session).await;
    std::fs::write(path.join(".move-setup-gate"), "").unwrap();
    let (_, (injection, run)) = tokio::join!(move_request(&fixture, &session, true), async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let worker = fixture.workers[0].lock().await;
            if let Some(run) = worker.providers().list_runs().into_iter().find(|run| {
                run.working_directory()
                    .is_some_and(|path| path.join(".move-setup-waiting").is_file())
            }) {
                let injection = ProviderLifecycleFailureInjection::install(
                    run.id(),
                    ProviderLifecycleFailureStage::End,
                );
                std::fs::write(
                    run.working_directory().unwrap().join(".move-setup-release"),
                    "",
                )
                .unwrap();
                break (injection, run);
            }
            drop(worker);
            assert!(
                tokio::time::Instant::now() < deadline,
                "Move must reach its stored setup recipe"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let mut worker = fixture.workers[0].lock().await;
    // A successful setup retains its ordinary run. Prove that the failing
    // teardown was never attempted after Ready, rather than silently ignored.
    let result = worker.end_agent_provider_run(run.session_id(), run.agent_instance_id().unwrap());
    assert!(
        result.is_err(),
        "Ready must not consume and ignore a provider teardown error"
    );
    assert_eq!(
        worker.providers().get_run(run.id()).unwrap().state(),
        crate::provider::ProviderRunState::Running
    );
    drop(injection);
    worker
        .end_agent_provider_run(run.session_id(), run.agent_instance_id().unwrap())
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mk10_failed_target_setup_can_be_cancelled_and_retried_with_a_fixed_recipe() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    std::env::set_var("CHARIOX_PROVIDER_DEV_STUB", "1");
    let (fixture, session, _) = crash_fixture().await;
    {
        let home = fixture.home.lock().await;
        let session = home.sessions().get_session(&session).unwrap();
        let definition = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "origin": "user_authored",
            "source": "commands",
            "target_platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            "source_path": null,
            "inputs": [],
            "path_entries": [],
            "setup_steps": [{"kind": "command", "command": "exit 7"}],
            "validation_commands": ["true"]
        }))
        .unwrap();
        home.sessions
            .write()
            .update_project_environment_definition(
                session.project_id(),
                definition,
                session.owner_user_id(),
            )
            .unwrap();
    }
    let move_to_worker = serde_json::json!({"MoveAgentToRemote": {"session_id": session, "agent_ref": fixture.agent_id, "machine_ref": fixture.presences[0].machine_id}});
    let error = dispatch_move_value(&fixture, move_to_worker.clone())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Project setup failed"),
        "{error}"
    );
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "bound"
    );
    // The operation stays; the agent says so, so a client that lost the error can cancel or retry.
    let failed = agent_move_status(&fixture)
        .await
        .expect("failed Move is published");
    assert_eq!(
        failed.target_ref.as_deref(),
        Some(fixture.presences[0].machine_id.as_str())
    );
    assert!(
        matches!(&failed.stage, crate::agent_move::AgentMoveStage::Failed { reason } if reason.contains("Project setup failed")),
        "{failed:?}"
    );
    dispatch_move_value(&fixture, serde_json::json!({"MoveAgentToLocal": {"session_id": session, "agent_ref": fixture.agent_id, "abort_pending": true}}))
        .await
        .expect("a Move whose target setup failed must be cancellable before activation");
    assert!(
        agent_move_status(&fixture).await.is_none(),
        "cancelling settles the Move"
    );
    assert_eq!(
        move_journal(&fixture, &session).await["phase"]["phase"],
        "rolled_back"
    );
    let home = fixture.home.lock().await;
    assert!(!home
        .agent_moves
        .dispatch_fenced(&session, &fixture.agent_id));
    assert!(home
        .agents()
        .get_agent(&fixture.agent_id)
        .unwrap()
        .remote_execution()
        .is_none());
    drop(home);
    assert!(
        fixture.workers[0].lock().await.leased_agents.is_empty(),
        "cancelling must retire the target's backing agent"
    );
    install_move_setup_definition(&fixture, &session).await;
    dispatch_move_value(&fixture, move_to_worker)
        .await
        .expect("a fixed recipe must let the next generation move");
    assert_eq!(
        move_journal(&fixture, &session).await["identity"]["generation"],
        2
    );
}
