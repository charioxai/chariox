//! MP-08/MP-10/MP-11: one poll containing task, prompt and provider changes.
use crate::durable_state::agent_lifecycle::Operation;
use crate::{provider::LaunchProviderRequest, session::PromptSubmissionOutcome, DaemonApp};

pub(crate) struct TaskDeltaPrompt {
    pub(crate) agent: String,
    run: String,
    prompt: String,
}

impl TaskDeltaPrompt {
    pub(crate) fn start(app: &mut DaemonApp, room: &str, attachment: &str) -> Self {
        let agent = app.agents().get_session_agents(room)[0].id().to_string();
        let run = app
            .launch_provider(
                LaunchProviderRequest::new(room, "dev-stub", "claude-code", "default", "sonnet")
                    .with_agent_id(&agent),
            )
            .unwrap();
        app.update_provider_run_projection(run.clone());
        let PromptSubmissionOutcome::Started { prompt } = app
            .submit_prompt(
                room,
                attachment,
                Some(&agent),
                "finish this task",
                Vec::new(),
            )
            .unwrap()
        else {
            panic!("first prompt must start")
        };
        app.durable_state_store()
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: room.into(),
                agent: agent.clone(),
                prompt: prompt.id().into(),
                run: Some(run.id().into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        Self {
            agent,
            run: run.id().into(),
            prompt: prompt.id().into(),
        }
    }

    pub(crate) fn finish(&self, app: &mut DaemonApp, room: &str) {
        app.complete_active_prompt(room, &self.agent, Some(&self.run))
            .unwrap();
        app.durable_state_store()
            .agent_lifecycle(Operation::Settle {
                room: room.into(),
                agent: self.agent.clone(),
                prompt: self.prompt.clone(),
                run: self.run.clone(),
                has_answer: true,
                cancelled: false,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        let run = app
            .providers()
            .update_run_selection(&self.run, Some("updated-model".into()), None, false)
            .unwrap();
        app.update_provider_run_projection(run);
        let session = app.sessions().get_session(room).unwrap();
        app.session_state_projection_store().update(session);
    }
}
