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

// MP-08 / MP-10 / MP-11: revoke Computer only; membership and Browser survive.
#[tokio::test]
async fn mp08_room_computer_revoke_denies_queued_and_new_input_and_regrants() {
    crate::test_support::isolated_env_test!();
    let room = TestRoom::new("room-computer-revoke");
    // MP-08 / MP-10 / MP-11: session actors and transport routers construct
    // independent states from the same kernel app; authority must be shared.
    let owner_runtime = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        std::sync::Arc::clone(&room.runtime.app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    )
    .runtime_state();
    room.runtime
        .reconcile_room_environment_actors(&room.session_id, None)
        .unwrap();
    let snapshot = room
        .runtime
        .room_environment_snapshot(&room.session_id)
        .unwrap();
    let actor = crate::session::agent_environment_actor_id(&room.agent_id);
    let submit = || {
        room.runtime
            .submit_room_environment_action(
                &room.session_id,
                EnvironmentActionRequest::computer_mutation(
                    &actor,
                    snapshot.runtime_generation,
                    "pointer_move",
                    None,
                ),
            )
            .unwrap()
            .0
    };
    let ActionAdmission::Accepted { action_id: first } = submit() else {
        panic!("first admission")
    };
    let ActionAdmission::Queued {
        action_id: queued, ..
    } = submit()
    else {
        panic!("queued admission")
    };
    let before_cursor = room
        .runtime
        .room_computer_grant_snapshot("local", "home")
        .unwrap()["cursor"]
        .as_u64()
        .unwrap();
    owner_runtime
        .set_room_computer_access("local", Some(&room.agent_id), false)
        .unwrap();
    let revoked_snapshot = room
        .runtime
        .room_computer_grant_snapshot("local", "home")
        .unwrap();
    let revoked_cursor = revoked_snapshot["cursor"].as_u64().unwrap();
    assert!(revoked_cursor > before_cursor);
    assert!(revoked_snapshot["room_computer"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["agent_id"] == room.agent_id && row["allowed"] == false));
    assert!(room
        .runtime
        .authorize_admitted_browser_action(&room.session_id, &queued)
        .is_err());
    assert!(room
        .runtime
        .execute_computer_input_as_agent(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::PointerMove { x: 10, y: 10 }
        )
        .await
        .is_err());
    assert!(room
        .runtime
        .execute_computer_input_as_agent(
            &room.session_id,
            &room.agent_id,
            RoomComputerInputAction::TargetAction {
                tree_revision: 1,
                target_id: "atspi-old".into(),
                action: "click".into()
            }
        )
        .await
        .is_err());
    assert_eq!(
        room.runtime
            .owned
            .agent_store
            .get_agent(&room.agent_id)
            .unwrap()
            .session_id(),
        room.session_id
    );
    assert_eq!(
        room.runtime
            .room_environment_snapshot(&room.session_id)
            .unwrap()
            .actions
            .iter()
            .find(|a| a.action_id == queued)
            .unwrap()
            .state,
        EnvironmentActionState::Cancelled
    );
    room.runtime
        .finish_room_environment_action(
            &room.session_id,
            &first,
            EnvironmentActionTerminal::Completed,
        )
        .unwrap();
    room.runtime
        .set_room_computer_access("foreign", Some(&room.agent_id), true)
        .unwrap_err();
    owner_runtime
        .set_room_computer_access("local", Some(&room.agent_id), true)
        .unwrap();
    room.runtime
        .require_room_computer_access(&room.agent_id)
        .unwrap();
    let granted_cursor = room
        .runtime
        .room_computer_grant_snapshot("local", "home")
        .unwrap()["cursor"]
        .as_u64()
        .unwrap();
    assert!(granted_cursor > revoked_cursor);
    room.runtime
        .set_room_computer_access("local", Some(&room.agent_id), true)
        .unwrap();
    assert_eq!(
        room.runtime
            .room_computer_grant_snapshot("local", "home")
            .unwrap()["cursor"]
            .as_u64(),
        Some(granted_cursor),
        "idempotent grants do not manufacture changes"
    );
    assert!(matches!(submit(), ActionAdmission::Accepted { .. }));
}
