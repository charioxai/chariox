use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::workflow_actor::WorkflowRuntime;

pub(crate) async fn cleanup_runtime_lanes_after_response(
    agent_runtime: &AgentRuntime,
    workflow_runtime: &WorkflowRuntime,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    let Ok(response) = result else {
        return;
    };
    match response {
        LocalDaemonResponse::AgentDestroyed { agent } => {
            agent_runtime.remove_agent_lane(agent.id()).await;
        }
        LocalDaemonResponse::SessionDeleted { session }
        | LocalDaemonResponse::SessionEnded { session } => {
            cleanup_session_runtime_lanes(agent_runtime, workflow_runtime, session).await;
        }
        LocalDaemonResponse::KernelDeleted {
            deleted_sessions, ..
        } => {
            for session in deleted_sessions {
                cleanup_session_runtime_lanes(agent_runtime, workflow_runtime, session).await;
            }
        }
        _ => {}
    }
}

async fn cleanup_session_runtime_lanes(
    agent_runtime: &AgentRuntime,
    workflow_runtime: &WorkflowRuntime,
    session: &crate::session::RuntimeSession,
) {
    agent_runtime.remove_session_state(session.id());
    agent_runtime
        .remove_agent_lanes(session.agents().iter().map(|agent| agent.id()))
        .await;
    workflow_runtime.remove_session_lane(session.id()).await;
}
