use super::computer_input_reconcile_test_support::{install_screen_tool, TestRoom, TestTools};
use super::*;

#[tokio::test]
async fn dispatched_browser_action_settles_after_grant_revocation() {
    crate::test_support::isolated_env_test!();
    for (terminal, needs_recovery) in [
        (EnvironmentActionTerminal::Completed, false),
        (EnvironmentActionTerminal::Failed, false),
        (EnvironmentActionTerminal::Cancelled, false),
        (EnvironmentActionTerminal::Failed, true),
        (EnvironmentActionTerminal::Cancelled, true),
    ] {
        let room = TestRoom::new("revoked-dispatched-browser");
        let grant = room.runtime.insert_access_grant_for_test(&room.session_id);
        let request = crate::local::LocalDaemonRequest::GetSessionState(
            crate::local::GetSessionStateRequest {
                session_id: room.session_id.clone(),
            },
        );
        let external = room
            .runtime
            .with_external_command_authority(Some((&grant, &request)));
        let before = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap();
        let tab = &before.tabs[0];
        let result = external
            .execute_browser_mutation_as_agent(
                &room.session_id,
                &room.agent_id,
                &tab.tab_id,
                tab.document_revision,
                "click",
                Some("revoked-action"),
                async {
                    let running = room
                        .runtime
                        .room_environment_snapshot(&room.session_id)
                        .unwrap();
                    assert_eq!(
                        running.actions.last().unwrap().state,
                        EnvironmentActionState::Running
                    );
                    room.runtime
                        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
                        .unwrap();
                    assert!(external.authorize_current_external_command().is_err());
                    match terminal {
                        EnvironmentActionTerminal::Completed => Ok(()),
                        EnvironmentActionTerminal::Cancelled => {
                            Err(DaemonError::BrowserControllerActionCancelled {
                                controller_fenced: needs_recovery,
                            })
                        }
                        _ if needs_recovery => {
                            Err(DaemonError::BrowserControllerRecoveryRequired {
                                runtime_generation: before.runtime_generation + 1,
                            })
                        }
                        _ => Err(action_dispatch_error("controller operation failed".into())),
                    }
                },
            )
            .await;
        let settled = room
            .runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap();
        assert_eq!(
            settled.runtime_generation, before.runtime_generation,
            "revoked commands must not start new controller recovery effects"
        );
        let expected = match terminal {
            EnvironmentActionTerminal::Completed => EnvironmentActionState::Completed,
            EnvironmentActionTerminal::Cancelled => EnvironmentActionState::Cancelled,
            _ => EnvironmentActionState::Failed,
        };
        assert_eq!(
            settled.actions.last().unwrap().state,
            expected,
            "dispatched terminal result must settle despite revocation: {result:?}"
        );
        let following = room.runtime.execute_browser_mutation_as_agent(
            &room.session_id,
            &room.agent_id,
            &tab.tab_id,
            tab.document_revision,
            "click",
            None,
            async { Ok(()) },
        );
        tokio::time::timeout(Duration::from_secs(2), following)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn dispatched_computer_action_settles_after_grant_revocation() {
    crate::test_support::isolated_env_test!();
    let tools = TestTools::new("revoked-dispatched-computer");
    let start = tools.screen_tool.with_file_name("input-started");
    let finish = tools.screen_tool.with_file_name("input-finish");
    std::fs::write(
        &tools.screen_tool,
        format!(
            "#!/bin/sh\nset -eu\n: > '{}'\nwhile [ ! -e '{}' ]; do sleep 0.01; done\n",
            start.display(),
            finish.display()
        ),
    )
    .unwrap();
    let _screen_tool = install_screen_tool(&tools.screen_tool);
    let room = TestRoom::new("revoked-dispatched-computer");
    let grant = room.runtime.insert_access_grant_for_test(&room.session_id);
    let request =
        crate::local::LocalDaemonRequest::GetSessionState(crate::local::GetSessionStateRequest {
            session_id: room.session_id.clone(),
        });
    let external = room
        .runtime
        .with_external_command_authority(Some((&grant, &request)));
    let pending = external.execute_computer_input_as_agent(
        &room.session_id,
        &room.agent_id,
        RoomComputerInputAction::PointerMove { x: 10, y: 10 },
    );
    let revoke = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            room.runtime
                .room_environment_snapshot(&room.session_id)
                .unwrap()
                .actions
                .last()
                .unwrap()
                .state,
            EnvironmentActionState::Running
        );
        room.runtime
            .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
            .unwrap();
        std::fs::write(&finish, "finish").unwrap();
    };
    let (result, ()) = tokio::join!(pending, revoke);
    let settled = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    assert_eq!(
        settled.actions.last().unwrap().state,
        EnvironmentActionState::Completed,
        "completed input must settle after revocation: {result:?}"
    );
    let following = room.runtime.execute_computer_input_as_agent(
        &room.session_id,
        &room.agent_id,
        RoomComputerInputAction::PointerMove { x: 20, y: 20 },
    );
    tokio::time::timeout(Duration::from_secs(2), following)
        .await
        .unwrap()
        .unwrap();
}
