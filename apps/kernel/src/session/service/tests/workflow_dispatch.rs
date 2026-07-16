use super::*;

fn completion_with_message(message: impl Into<String>) -> WorkflowCompletionSnapshot {
    WorkflowCompletionSnapshot::new(
        "done",
        Some(crate::session::WorkflowOutputPayload::new(
            message.into(),
            Vec::new(),
        )),
    )
}

mod downstream_dispatch;
mod join_inputs;
mod routed_edges;
mod run_output_validation;
mod schema_validation;
