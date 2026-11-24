//! MP-08 / MP-10: an agent of the owner sees and follows the owner's workflow sources of another
//! kernel, with nothing cached yet and without a manual refresh step.
use super::availability::{on_a_big_stack, Drill};
use super::*;
use crate::local::LocalDaemonResponse;
use crate::session::{AgentTriggerNotificationSource, AgentTriggerSource, WorkflowRunStatus};
use crate::transport::runtime_tools::{NOTIFICATION_SOURCES, NOTIFICATION_SUBSCRIBE};

const AGENT: &str = "agent";

/// The target kernel's Room agent, holding the provider run its tool calls carry.
struct Caller {
    runtime: KernelRuntimeState,
    session: crate::session::RuntimeSession,
    agent: crate::agent::AgentInstance,
}

fn room_agent(drill: &Drill) -> Caller {
    let mut config = drill.target_runtime.owned.config_projection.snapshot();
    config.room_agent_tools = true;
    drill.target_runtime.owned.config_projection.update(config);
    let run = drill
        .target_runtime
        .owned
        .provider_store
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                &drill.target.session,
                "dev-stub",
                "dev-stub",
                "default",
                "room-model",
            )
            .with_agent_id(AGENT),
        )
        .unwrap();
    let session = drill
        .target_runtime
        .owned
        .session_store
        .get_session(&drill.target.session)
        .unwrap();
    let agent = session
        .agents()
        .iter()
        .find(|agent| agent.id() == AGENT)
        .unwrap()
        .clone();
    drill
        .target_runtime
        .owned
        .agent_store
        .restore_agent(agent.clone());
    Caller {
        runtime: drill
            .target_runtime
            .with_room_provider_origin(Some(AGENT), Some(run.id())),
        session,
        agent,
    }
}

impl Caller {
    async fn tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime
            .dispatch_notification_tool(&self.session, &self.agent, name, arguments)
            .await
    }
}

/// How often the agent's notification trigger fired, and why its last firing failed.
fn firings(drill: &Drill) -> (u64, Option<String>) {
    let session = drill
        .target_runtime
        .owned
        .session_snapshot(&drill.target.session)
        .unwrap();
    let trigger = session
        .agent_triggers()
        .iter()
        .find(|t| matches!(t.source(), AgentTriggerSource::Notification { .. }))
        .expect("the agent follows the source");
    (trigger.fired(), trigger.last_error().map(str::to_owned))
}

/// Runs each kernel's pumps, one second apart as the kernel does, until the agent's trigger has
/// fired `fired` times: the source routes the occurrence to the peer, the target accepts it and
/// queues the prompt of the agent that follows.
async fn pump_until_fired(drill: &Drill, fired: u64) {
    for _ in 0..40 {
        for runtime in [&drill.source_runtime, &drill.target_runtime] {
            // The kernel's own pump delivers to peers before its local router rotates the rows.
            runtime.route_remote_workflow_notifications().await;
            runtime.owned.route_workflow_notifications();
            runtime.route_agent_notifications().await;
        }
        if firings(drill).0 >= fired {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[test]
fn an_agent_of_the_owner_follows_a_source_of_another_kernel_from_a_cold_cache() {
    on_a_big_stack(async {
        let mut drill = Drill::connect().await;
        let source_id = drill.source.source(&drill.workflow).source_id;
        let source_kernel = drill
            .source_runtime
            .owned
            .config_projection
            .snapshot()
            .daemon_id;
        assert!(
            drill
                .target
                .store
                .notification_cached_sources("local")
                .unwrap()
                .is_empty(),
            "the target has not asked its peers yet"
        );
        let caller = room_agent(&drill);

        let listed = caller
            .tool(NOTIFICATION_SOURCES, serde_json::json!({}))
            .await;
        let listed = listed.unwrap().payload;
        let remote = listed["workflows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["source"]["workflow"]["source_id"] == source_id.as_str())
            .unwrap_or_else(|| {
                panic!("the owner's source of the other kernel is listed: {listed}")
            });
        assert_eq!(remote["kernel_id"], source_kernel.as_str());
        assert_eq!(remote["available"], true);

        let created = caller
            .tool(
                NOTIFICATION_SUBSCRIBE,
                serde_json::json!({"source":{"workflow":{"source_id":source_id}},"prompt":"review it"}),
            )
            .await
            .unwrap();
        assert!(created.ok, "{:?}", created.payload);

        for (run, expected) in [("run-1", 1), ("run-2", 2)] {
            let workflow = drill.workflow.clone();
            drill
                .source
                .complete(&workflow, run, None, WorkflowRunStatus::Completed, run);
            // The fixture saved the run; the kernel's own room store takes it over.
            drill
                .source_runtime
                .owned
                .session_store
                .write()
                .restore_session(
                    drill
                        .source
                        .sessions
                        .get_session(&drill.source.session)
                        .unwrap(),
                );
            pump_until_fired(&drill, expected).await;
            assert_eq!(firings(&drill).0, expected, "one prompt per occurrence");
        }
        drill.finish().await;
    });
}

#[test]
fn the_trigger_command_names_a_source_of_another_kernel_by_id_or_name_with_a_prompt() {
    on_a_big_stack(async {
        let drill = Drill::connect().await;
        let source = drill.source.source(&drill.workflow);
        let source_kernel = drill
            .source_runtime
            .owned
            .config_projection
            .snapshot()
            .daemon_id;
        let caller = room_agent(&drill);
        let words = |text: &[&str]| text.iter().map(|w| w.to_string()).collect::<Vec<_>>();

        let (chosen, prompt) = caller
            .runtime
            .choose_notification_source(
                "local",
                "local",
                &drill.target.session,
                &words(&[&source.source_id, "review", "it"]),
            )
            .await
            .unwrap();
        assert_eq!(
            chosen,
            AgentTriggerNotificationSource::Workflow {
                source_id: source.source_id.clone()
            }
        );
        assert_eq!(prompt.as_deref(), Some("review it"));

        let display_name = drill
            .target
            .store
            .notification_cached_sources("local")
            .unwrap()
            .into_iter()
            .find(|s| s.source_id == source.source_id)
            .unwrap()
            .display_name;
        let qualified = format!("{source_kernel}/{display_name}");
        let (named, _) = caller
            .runtime
            .choose_notification_source(
                "local",
                "local",
                &drill.target.session,
                &words(&[&qualified]),
            )
            .await
            .unwrap();
        assert_eq!(named, chosen);

        let created = caller
            .runtime
            .create_agent_trigger(
                crate::local::CreateAgentTriggerRequest {
                    session_id: drill.target.session.clone(),
                    agent_id: AGENT.into(),
                    source: crate::local::AgentTriggerSourceRequest::Notification {
                        source: chosen,
                        options: Vec::new(),
                        once: false,
                    },
                    busy_policy: None,
                    prompt,
                },
                "local",
            )
            .await
            .unwrap();
        assert!(matches!(
            created,
            LocalDaemonResponse::AgentTriggerCreated { .. }
        ));

        // A collaborator's agent still sees this Room's sources only, and asks nobody.
        let guest = caller
            .runtime
            .choose_notification_source(
                "local",
                "guest",
                &drill.target.session,
                &words(&[&source.source_id]),
            )
            .await;
        assert!(guest.is_err(), "a guest names no source of another kernel");
        drill.finish().await;
    });
}

// MP-08 / MP-10: the owner's agent follows a peer kernel's "Agent needs your action" source before
// that Room has raised anything, and a question raised there reaches the agent as a prompt.
#[test]
fn a_question_raised_on_another_kernel_reaches_the_subscribed_agent() {
    on_a_big_stack(async {
        let drill = Drill::connect().await;
        let source_kernel = drill
            .source_runtime
            .owned
            .config_projection
            .snapshot()
            .daemon_id;
        let caller = room_agent(&drill);

        let listed = caller
            .tool(NOTIFICATION_SOURCES, serde_json::json!({}))
            .await
            .unwrap()
            .payload;
        let alert = listed["workflows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| {
                s["kernel_id"] == source_kernel.as_str()
                    && s["display_name"] == "Agent needs your action"
            })
            .unwrap_or_else(|| panic!("the peer Room's needs-action source is listed: {listed}"))
            .clone();
        let created = caller
            .tool(
                NOTIFICATION_SUBSCRIBE,
                serde_json::json!({"source":alert["source"],"prompt":"Triage the question"}),
            )
            .await
            .unwrap();
        assert!(created.ok, "{:?}", created.payload);

        use crate::durable_state::agent_lifecycle::{Operation, Outcome};
        let store = &drill.source_runtime.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: drill.source.session.clone(),
                agent: AGENT.into(),
                prompt: "question-drill".into(),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        let Outcome::Task(blocked) = store
            .agent_lifecycle(Operation::Block {
                task: "question-drill".into(),
                prompt: "question-drill".into(),
                reason: "private reason excluded from notifications".into(),
            })
            .unwrap()
        else {
            panic!()
        };
        drill
            .source_runtime
            .ensure_task_owner_interaction(blocked)
            .await
            .unwrap();
        pump_until_fired(&drill, 1).await;
        assert_eq!(firings(&drill), (1, None));
        drill.finish().await;
    });
}
