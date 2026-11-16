//! A leased child's provider run can keep working after its home turn ended: a background
//! command still runs, or the provider's own follow-up turn executes. The home that gave up on
//! that run owes the worker a stop for it, and no home prompt names it any more.
use crate::error::DaemonError;
use crate::execution_lease::LeasedAgent;
use crate::provider::ProviderRunState;
use crate::session::{PromptCancellation, PromptQueueItem, PromptStatus};

use super::RemoteLeaseRuntime;

impl RemoteLeaseRuntime<'_> {
    /// Stops the exact run of the exact home prompt when the agent has no active home prompt and
    /// the run is still doing work; `None` leaves every other cancellation to the prompt-bound path.
    pub(super) fn stop_run_working_without_a_prompt(
        &mut self,
        leased_agent: &LeasedAgent,
        home_prompt_id: &str,
        worker_provider_run_id: &str,
    ) -> Result<Option<PromptCancellation>, DaemonError> {
        if leased_agent.active_home_prompt_id.is_some() {
            return Ok(None);
        }
        let Some(run) = self.app.providers.get_run_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        ) else {
            return Ok(None);
        };
        if run.id() != worker_provider_run_id || run.state() == ProviderRunState::Ended {
            return Ok(None);
        }
        // The run is reused by later prompts: the stop belongs to the prompt whose turn the run
        // outlived, and a stop for an earlier one finds its turn long settled.
        let Some(completion) = leased_agent
            .replayable_completion
            .as_ref()
            .filter(|completion| completion.provider_run_id == worker_provider_run_id)
        else {
            return Ok(None);
        };
        if completion.home_prompt_id.as_deref() != Some(home_prompt_id) {
            // A prompt this worker accepted and that is not the one the run outlived has
            // finished; a stop naming a prompt the worker never saw stays an error.
            return if self.leased_prompt_receipt_recorded(&leased_agent.id, home_prompt_id) {
                Err(DaemonError::NoActivePrompt {
                    session_id: leased_agent.backing_session_id.clone(),
                })
            } else {
                Ok(None)
            };
        }
        let background_command_runs = crate::app::background_calls(
            self.app
                .operational_history_store()
                .load_provider_tool_entries_for_run(
                    &leased_agent.backing_session_id,
                    worker_provider_run_id,
                )?,
        ) == crate::app::BackgroundCalls::Running;
        let follow_up_turn_runs = run.last_output_at_ms() > completion.completed_at_ms;
        if !background_command_runs && !follow_up_turn_runs {
            return Ok(None);
        }
        self.terminate_backing_provider_runtime(leased_agent);
        Ok(Some(PromptCancellation {
            prompt: PromptQueueItem::new(
                home_prompt_id,
                "",
                leased_agent.backing_agent_id.as_str(),
                "",
                PromptStatus::Cancelled,
            ),
            started_next: None,
        }))
    }
}
