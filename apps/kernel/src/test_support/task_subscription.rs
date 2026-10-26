//! MP-08 / MP-10 A12-S02: authoritative mixed updates for both subscription writers.
use crate::agent::CreateAgentRequest;
use crate::app::{KernelSessionReadService, KernelSessionService};
use crate::durable_state::agent_lifecycle::Operation;
use crate::session::{PromptQueueItem, PromptStatus, RuntimeSession};
use crate::DaemonApp;

pub(crate) fn mixed_task_subscription_updates(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    case: usize,
) -> RuntimeSession {
    let mut session = KernelSessionReadService::new(app)
        .session_snapshot(session_id)
        .unwrap();
    let agent_id = session.agents()[0].id().to_string();
    let store = app.durable_state_store();
    let now = crate::session::unix_epoch_ms();
    match case {
        0 => {
            store
                .agent_lifecycle(Operation::Begin {
                    owner: session.owner_user_id().into(),
                    room: session_id.into(),
                    agent: agent_id.clone(),
                    prompt: "subscription-task".into(),
                    run: None,
                    now,
                })
                .unwrap();
            session.set_alias(Some("task-meta".into()));
            app.sessions_mut().restore_session(session);
        }
        1 => {
            let agent = KernelSessionService::new(app)
                .spawn_agent(CreateAgentRequest::new(session_id, "codex").with_alias("new"))
                .unwrap();
            store
                .agent_lifecycle(Operation::Begin {
                    owner: session.owner_user_id().into(),
                    room: session_id.into(),
                    agent: agent.id().into(),
                    prompt: "new-agent-task".into(),
                    run: None,
                    now,
                })
                .unwrap();
        }
        2 => {
            store
                .agent_lifecycle(Operation::Progress {
                    task: "subscription-task".into(),
                    prompt: "subscription-task".into(),
                    receipt: "queue-progress".into(),
                    now,
                })
                .unwrap();
            app.prompt_owner_submit_prepared_prompt(
                session_id,
                PromptQueueItem::new(
                    "queued",
                    attachment_id,
                    &agent_id,
                    "next",
                    PromptStatus::Queued,
                ),
                true,
            )
            .unwrap();
        }
        3 => {
            let run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
                "subscription-run",
                session_id.into(),
                Some(agent_id),
                "codex".into(),
            );
            app.providers_mut().insert_run_for_test(run.clone());
            app.sessions_mut()
                .set_active_provider_run(session_id, Some(run.id().into()))
                .unwrap();
            store
                .agent_lifecycle(Operation::Progress {
                    task: "subscription-task".into(),
                    prompt: "subscription-task".into(),
                    receipt: "provider-progress".into(),
                    now,
                })
                .unwrap();
        }
        _ => unreachable!(),
    }
    let mut session = KernelSessionReadService::new(app)
        .session_snapshot(session_id)
        .unwrap();
    session.set_agent_tasks(store.agent_tasks(Some(session_id), None).unwrap());
    app.update_session_projection(session.clone());
    session
}
