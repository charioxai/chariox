use super::*;

fn launch_slow_structured_run(app: &mut DaemonApp, session_id: &str, agent_id: &str) -> String {
    app.launch_provider(
        LaunchProviderRequest::new(
            session_id,
            "dev-stub",
            "slow-structured",
            "default",
            "default",
        )
        .with_agent_id(agent_id),
    )
    .expect("slow structured provider run should launch")
    .id()
    .to_string()
}

mod attachment_scoping;
mod basic_pump;
mod native_output;
mod native_output_batch;
mod parallel_prompts;
