// MP-08 / MP-09 / MP-10 / MP-11: exercise setup through public Move admission.
use super::*;

async fn install_move_setup_definition(fixture: &Fixture, session_id: &str) {
    let home = fixture.home.lock().await;
    let session = home.sessions().get_session(session_id).unwrap();
    let mut agent = home.agents().get_agent(&fixture.agent_id).unwrap();
    agent.set_provider("dev-stub");
    agent.set_account_profile(None);
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
