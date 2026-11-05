//! Draft with agent: an idle agent of the Project reads what the kernel shows it of the Project's
//! files and the answer becomes revision 1 of the Environment. Nothing is confirmed or run.
use super::metadata_utility_scratch::MetadataUtilityScratch;
use super::*;
use crate::local::{
    DraftProjectEnvironmentRequest, EnvironmentOperationKind, EnvironmentOperationState,
    EnvironmentOperationView,
};
use crate::project_environment::{
    draft_prompt, environment_error, parse_draft, register_environment_files, Begun,
    OperationHandle, ProjectEnvironmentRecord, SaveOutcome,
};
use crate::provider::{ProviderRunState, ProviderUtilityExecutionPolicy};
use crate::runtime::agent_utility_executor::{
    run_provider_utility_prompt, AgentUtilityPromptParts,
};
use std::path::PathBuf;

const OPERATION: &str = "draft Project environment";
const DRAFT_TIMEOUT: Duration = Duration::from_secs(180);
const PROVIDER_READY_TIMEOUT: Duration = Duration::from_secs(60);
const HIDDEN_CONTEXT: &str = "Classify only the supplied Project metadata. No tools or source contents are available. Answer with the JSON object the user message asks for and nothing else.";

impl KernelRuntimeState {
    pub(crate) fn draft_project_environment(
        &self,
        request: DraftProjectEnvironmentRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, None)?;
        if self.environment_store().load(project.id())?.is_some() {
            return Err(environment_error("this Project already has an Environment"));
        }
        let (session_id, agent_id) = self.idle_project_agent(&project)?;
        let view = EnvironmentOperationView {
            operation_id: request.operation_id,
            project_id: project.id().into(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            kind: EnvironmentOperationKind::Draft,
            state: EnvironmentOperationState::Running,
            started_at_ms: crate::session::unix_epoch_ms(),
            ended_at_ms: None,
            exit_code: None,
            reason: None,
            output_tail: String::new(),
            entries: Vec::new(),
            source_label: None,
        };
        let handle = match self.owned.environment_operations.begin(view.clone())? {
            Begun::Existing(operation) => {
                return Ok(LocalDaemonResponse::EnvironmentOperation { operation })
            }
            Begun::Started(handle) => handle,
        };
        let runtime = self.clone();
        let project_id = project.id().to_string();
        let checkouts: Vec<PathBuf> = project.workspace_ids().iter().map(PathBuf::from).collect();
        tokio::spawn(async move {
            let drafted = tokio::select! {
                drafted = runtime.draft_environment_fields(&session_id, &agent_id, checkout) => drafted,
                _ = handle.stopped() => return,
            };
            runtime.save_draft(&handle, &project_id, &checkouts, drafted);
        });
        Ok(LocalDaemonResponse::EnvironmentOperation { operation: view })
    }

    /// An agent of the Project that runs on this kernel and has no prompt running: the utility borrows its provider.
    pub(super) fn idle_project_agent(
        &self,
        project: &crate::session::RuntimeProject,
    ) -> Result<(String, String), DaemonError> {
        for session in self.owned.session_store.sessions_in_project(project.id())? {
            for agent in self.owned.agent_store.get_session_agents(session.id()) {
                if agent.remote_execution().is_none()
                    && self
                        .owned
                        .prompt_state_owner
                        .active_prompt_for_agent(&session, agent.id())
                        .is_none()
                {
                    return Ok((session.id().to_string(), agent.id().to_string()));
                }
            }
        }
        Err(environment_error(
            "this Project has no idle agent here; open a room in it first",
        ))
    }

    fn save_draft(
        &self,
        handle: &OperationHandle,
        project_id: &str,
        checkouts: &[PathBuf],
        drafted: Result<ProjectEnvironmentRecord, DaemonError>,
    ) {
        let outcome =
            drafted.and_then(|fields| self.environment_store().save(project_id, 0, fields));
        match outcome {
            Ok(SaveOutcome::Saved(record)) => {
                register_environment_files(checkouts, &record.files);
                handle.finish(EnvironmentOperationState::Succeeded, None);
            }
            Ok(SaveOutcome::Conflict(_)) => handle.finish(
                EnvironmentOperationState::Failed,
                Some("This Project already has an Environment".into()),
            ),
            Err(error) => handle.finish(EnvironmentOperationState::Failed, Some(error.to_string())),
        }
    }

    async fn draft_environment_fields(
        &self,
        session_id: &str,
        agent_id: &str,
        checkout: PathBuf,
    ) -> Result<ProjectEnvironmentRecord, DaemonError> {
        let prompt = tokio::task::spawn_blocking(move || draft_prompt(&checkout))
            .await
            .map_err(|_| environment_error("reading the Project failed"))?;
        let output = self
            .environment_utility_answer(session_id, agent_id, prompt)
            .await?;
        parse_draft(&output).map_err(environment_error)
    }

    /// The agent's answer to `visible_user_prompt`, run on its provider with no tools in a neutral directory.
    pub(super) async fn environment_utility_answer(
        &self,
        session_id: &str,
        agent_id: &str,
        visible_user_prompt: String,
    ) -> Result<String, DaemonError> {
        let started = tokio::time::Instant::now();
        let mut run = loop {
            let (_, run) = self
                .agent_utility_provider_run(session_id, agent_id, OPERATION)
                .await?;
            match run.state() {
                ProviderRunState::Running => break run,
                ProviderRunState::Starting if started.elapsed() < PROVIDER_READY_TIMEOUT => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                state => {
                    return Err(environment_error(format!(
                        "the agent's provider is not running ({state:?})"
                    )))
                }
            }
        };
        let scratch = MetadataUtilityScratch::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        )?;
        run.set_metadata_only_discovery(scratch.0.clone());
        let output = run_provider_utility_prompt(
            self,
            run,
            AgentUtilityPromptParts {
                visible_user_prompt,
                hidden_system_context: HIDDEN_CONTEXT.into(),
            },
            OPERATION,
            ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery,
            DRAFT_TIMEOUT,
        )
        .await?;
        Ok(output)
    }
}
