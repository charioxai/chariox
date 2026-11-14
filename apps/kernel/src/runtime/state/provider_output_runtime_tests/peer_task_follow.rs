//! A coordinator follows one task of a lane agent in another room of the same owner through the
//! subscribe tool, and hears of its end as a peer event.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

struct Turn {
    room: String,
    agent: String,
    prompt: crate::session::PromptQueueItem,
    run: String,
}

/// One running turn of the root agent of each of two rooms of one owner.
async fn two_rooms(
    label: &str,
) -> (
    KernelRuntimeState,
    [Turn; 2],
    [crate::test_support::TestWorktree; 2],
) {
    let worktrees = [
        crate::test_support::TestWorktree::new(&format!("{label}-coordinator")),
        crate::test_support::TestWorktree::new(&format!("{label}-lane")),
    ];
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let mut turns = Vec::new();
    for worktree in &worktrees {
        let (room, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                room.id(),
                "client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let run = app
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    room.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .unwrap();
        let crate::session::PromptSubmissionOutcome::Started { prompt } = app
            .submit_prompt(
                room.id(),
                attachment.id(),
                Some(agent.id()),
                "work",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("the turn starts")
        };
        turns.push(Turn {
            room: room.id().into(),
            agent: agent.id().into(),
            prompt,
            run: run.id().into(),
        });
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    for turn in &turns {
        runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: turn.room.clone(),
                agent: turn.agent.clone(),
                prompt: turn.prompt.id().into(),
                run: Some(turn.run.clone()),
                now: 1,
            })
            .unwrap();
    }
    let [coordinator, lane] = <[Turn; 2]>::try_from(turns).ok().unwrap();
    (runtime, [coordinator, lane], worktrees)
}

#[tokio::test]
async fn a_coordinator_follows_a_lane_task_in_another_room_until_it_ends() {
    let (runtime, [coordinator, lane], _worktrees) = two_rooms("peer-task-follow").await;
    let token = runtime
        .owned
        .provider_store
        .get_run(&coordinator.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    let followed = runtime
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.events.subscribe",
            serde_json::json!({
                "task_id": coordinator.prompt.id(),
                "origin_prompt_id": coordinator.prompt.id(),
                "source_id": lane.prompt.id(),
                "room": lane.room,
            }),
        )
        .await
        .expect("a lane task in another room of the owner can be followed")
        .payload;
    assert_eq!(followed["source_id"], lane.prompt.id(), "{followed}");
    assert_eq!(followed["agent_id"], lane.agent.as_str(), "{followed}");
    let registration = followed["registration_id"].as_str().unwrap();

    let store = &runtime.owned.durable_state_store;
    let Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: lane.room.clone(),
            agent: lane.agent.clone(),
            prompt: lane.prompt.id().into(),
            run: lane.run.clone(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    runtime
        .owned
        .complete_local_prompt_without_advance(&lane.room, &lane.agent, Some(&lane.run))
        .unwrap();
    runtime
        .finish_agent_task_settlement(Some((task, correction)), &lane.prompt)
        .await
        .unwrap();

    let events: Vec<_> = store
        .agent_inbox(&coordinator.room, &coordinator.agent, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "peer_event")
        .collect();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].payload["event"], "turn_ended", "{events:?}");
    assert_eq!(events[0].payload["turn"]["turn_ref"], lane.prompt.id());
    assert!(events[0].payload.get("public_answer").is_none());
    assert!(
        !store
            .agent_registration(registration)
            .unwrap()
            .unwrap()
            .live,
        "the follow ends with the task it follows"
    );
}
