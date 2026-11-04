//! Check with agent and the proposals it leaves: an idle agent of the Project compares the Project's
//! files with the saved Environment and proposes single changes with their evidence. Nothing changes
//! until the owner accepts one, which is an ordinary Save.
use super::*;
use crate::local::{
    CheckProjectEnvironmentRequest, EnvironmentOperationKind, EnvironmentOperationState,
    EnvironmentOperationView, ResolveEnvironmentProposalRequest,
};
use crate::project_environment::{
    apply_proposal, check_prompt, draft_facts, environment_error, parse_proposals,
    register_environment_files, Begun, ProposalStore, SaveOutcome,
};
use std::path::PathBuf;

impl KernelRuntimeState {
    pub(super) fn proposal_store(&self) -> ProposalStore {
        ProposalStore::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        )
    }

    pub(crate) fn check_project_environment(
        &self,
        request: CheckProjectEnvironmentRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, None)?;
        let record = self
            .environment_store()
            .load(project.id())?
            .ok_or_else(|| environment_error("this Project has no Environment to check yet"))?;
        let (session_id, agent_id) = self.idle_project_agent(&project)?;
        let view = EnvironmentOperationView {
            operation_id: request.operation_id,
            project_id: project.id().into(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            kind: EnvironmentOperationKind::Check,
            state: EnvironmentOperationState::Running,
            started_at_ms: crate::session::unix_epoch_ms(),
            ended_at_ms: None,
            exit_code: None,
            reason: None,
            updates: 0,
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
        tokio::spawn(async move {
            let work = async {
                let facts = tokio::task::spawn_blocking(move || draft_facts(&checkout))
                    .await
                    .map_err(|_| environment_error("reading the Project failed"))?;
                let answer = runtime
                    .environment_utility_answer(
                        &session_id,
                        &agent_id,
                        check_prompt(&facts, &record),
                    )
                    .await?;
                let proposals = parse_proposals(&answer, &record).map_err(environment_error)?;
                runtime.proposal_store().replace(&project_id, proposals)
            };
            let outcome = tokio::select! {
                outcome = work => outcome,
                _ = handle.stopped() => return,
            };
            match outcome {
                Ok(()) => handle.finish(EnvironmentOperationState::Succeeded, None),
                Err(error) => {
                    handle.finish(EnvironmentOperationState::Failed, Some(error.to_string()))
                }
            }
        });
        Ok(LocalDaemonResponse::EnvironmentOperation { operation: view })
    }

    pub(crate) fn resolve_environment_proposal(
        &self,
        request: ResolveEnvironmentProposalRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, None)?;
        let proposals = self.proposal_store();
        let record = self.environment_store().load(project.id())?;
        if request.accept {
            let record =
                record.ok_or_else(|| environment_error("this Project has no Environment"))?;
            let proposal = proposals
                .list(project.id())?
                .into_iter()
                .find(|proposal| proposal.id == request.proposal_id)
                .ok_or_else(|| environment_error("this proposal is gone"))?;
            let fields = apply_proposal(&record, &proposal)?;
            match self
                .environment_store()
                .save(project.id(), record.revision, fields)?
            {
                SaveOutcome::Saved(saved) => {
                    proposals.resolve(project.id(), &request.proposal_id, false)?;
                    let checkouts: Vec<PathBuf> =
                        project.workspace_ids().iter().map(PathBuf::from).collect();
                    register_environment_files(&checkouts, &saved.files);
                    return Ok(LocalDaemonResponse::ProjectEnvironmentSaved {
                        environment: self.project_environment_view(
                            &project,
                            checkout,
                            Some(saved),
                        )?,
                    });
                }
                SaveOutcome::Conflict(current) => {
                    return Ok(LocalDaemonResponse::ProjectEnvironmentConflict {
                        environment: self.project_environment_view(
                            &project,
                            checkout,
                            Some(current),
                        )?,
                    })
                }
            }
        }
        proposals.resolve(project.id(), &request.proposal_id, true)?;
        Ok(LocalDaemonResponse::ProjectEnvironmentSaved {
            environment: self.project_environment_view(&project, checkout, record)?,
        })
    }
}
