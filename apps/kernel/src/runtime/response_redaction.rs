use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::projection::ProviderRunProjectionStore;
use crate::runtime::provider_process_control::provider_processes_visible_to_user_from_projection;
use crate::runtime::provider_run_control::ensure_provider_run_visible_to_user;
use crate::runtime::session_projection_refresh::redact_agent_activity_for_session;
use crate::session::RuntimeSession;

pub(crate) fn redact_response_for_user(
    response: LocalDaemonResponse,
    caller_user_id: &str,
    provider_run_projection: &ProviderRunProjectionStore,
    workflow_run_context: Option<&RuntimeSession>,
) -> Result<LocalDaemonResponse, DaemonError> {
    Ok(match response {
        LocalDaemonResponse::SessionCreated { session, agent } => {
            LocalDaemonResponse::SessionCreated {
                session: session.redacted_for_user(caller_user_id),
                agent,
            }
        }
        LocalDaemonResponse::SessionResolved { session } => LocalDaemonResponse::SessionResolved {
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::SessionState {
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                agent_activity_revision,
                session,
            }
        }
        LocalDaemonResponse::MetaagentTaskUpdated { session, task } => {
            let session = session.redacted_for_user(caller_user_id);
            let task = task.filter(|task| {
                session
                    .metaagent_tasks()
                    .iter()
                    .any(|visible| visible.task_id() == task.task_id())
            });
            LocalDaemonResponse::MetaagentTaskUpdated { session, task }
        }
        LocalDaemonResponse::SessionsListed { sessions } => LocalDaemonResponse::SessionsListed {
            sessions: sessions
                .into_iter()
                .map(|session| session.redacted_for_user(caller_user_id))
                .collect(),
        },
        LocalDaemonResponse::SessionInviteCreated { invite, session } => {
            LocalDaemonResponse::SessionInviteCreated {
                invite,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::SessionInviteJoined { member, session } => {
            LocalDaemonResponse::SessionInviteJoined {
                member,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::SessionInviteRevoked { invite, session } => {
            LocalDaemonResponse::SessionInviteRevoked {
                invite,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkspaceLinkCreated { link, session } => {
            LocalDaemonResponse::WorkspaceLinkCreated {
                link,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session,
        } => LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkspaceLinkDetached {
            link,
            detached,
            session,
        } => LocalDaemonResponse::WorkspaceLinkDetached {
            link,
            detached,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkspaceLiveSyncModeUpdated { session, effects } => {
            LocalDaemonResponse::WorkspaceLiveSyncModeUpdated {
                session: session.redacted_for_user(caller_user_id),
                effects,
            }
        }
        LocalDaemonResponse::PromptSubmitted {
            outcome,
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::PromptSubmitted {
                outcome,
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                agent_activity_revision,
                session,
            }
        }
        LocalDaemonResponse::QueuedPromptSteered {
            prompt,
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::QueuedPromptSteered {
                prompt,
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                agent_activity_revision,
                session,
            }
        }
        LocalDaemonResponse::QueuedPromptCancelled {
            prompt,
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::QueuedPromptCancelled {
                prompt,
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                agent_activity_revision,
                session,
            }
        }
        LocalDaemonResponse::QueuedPromptUpdated {
            prompt,
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::QueuedPromptUpdated {
                prompt,
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                agent_activity_revision,
                session,
            }
        }
        LocalDaemonResponse::SessionConfigUpdated { config, session } => {
            LocalDaemonResponse::SessionConfigUpdated {
                config,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::AgentConfigUpdated { agent, session } => {
            LocalDaemonResponse::AgentConfigUpdated {
                agent,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            LocalDaemonResponse::AgentProfileUpdated {
                agent,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::AgentAliased { agent, session } => LocalDaemonResponse::AgentAliased {
            agent,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::SessionEnded { session } => LocalDaemonResponse::SessionEnded {
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::SessionDeleted { session } => LocalDaemonResponse::SessionDeleted {
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::SessionAliased { session } => LocalDaemonResponse::SessionAliased {
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::AgentsListed { agents } => LocalDaemonResponse::AgentsListed {
            agents: agents
                .into_iter()
                .filter(|agent| agent.owner_user_id() == caller_user_id)
                .collect(),
        },
        LocalDaemonResponse::ProviderRun { provider_run } => {
            ensure_provider_run_visible_to_user(&provider_run, caller_user_id)?;
            LocalDaemonResponse::ProviderRun { provider_run }
        }
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            LocalDaemonResponse::ProviderProcessesListed {
                processes: provider_processes_visible_to_user_from_projection(
                    processes,
                    provider_run_projection,
                    caller_user_id,
                ),
            }
        }
        LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
            LocalDaemonResponse::ProviderProcessesTornDown {
                processes: provider_processes_visible_to_user_from_projection(
                    processes,
                    provider_run_projection,
                    caller_user_id,
                ),
            }
        }
        LocalDaemonResponse::WorkflowCreated { workflow, session } => {
            LocalDaemonResponse::WorkflowCreated {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowCodeApplied { result, session } => {
            LocalDaemonResponse::WorkflowCodeApplied {
                result,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowCodeRun { result, session } => {
            LocalDaemonResponse::WorkflowCodeRun {
                result,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowDesignOpAccepted { session, event } => {
            LocalDaemonResponse::WorkflowDesignOpAccepted {
                session: session.redacted_for_user(caller_user_id),
                event,
            }
        }
        LocalDaemonResponse::WorkflowAliased { workflow, session } => {
            LocalDaemonResponse::WorkflowAliased {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            LocalDaemonResponse::WorkflowsListed {
                workflows: workflows
                    .into_iter()
                    .map(|workflow| workflow.redacted_for_user(caller_user_id))
                    .collect(),
            }
        }
        LocalDaemonResponse::WorkflowResolved { workflow } => {
            LocalDaemonResponse::WorkflowResolved {
                workflow: workflow.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowPublicationCreated {
            publication,
            session,
        } => LocalDaemonResponse::WorkflowPublicationCreated {
            publication,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowPublicationDisabled {
            publication,
            session,
        } => LocalDaemonResponse::WorkflowPublicationDisabled {
            publication,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id,
            session,
            agent_id_map,
        } => LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id,
            session: session.redacted_for_user(caller_user_id),
            agent_id_map,
        },
        LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeAdded {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeRemoved {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowRunInvoked {
            workflow_run,
            workflow,
            endpoint,
            session,
        } => LocalDaemonResponse::WorkflowRunInvoked {
            workflow_run: workflow_run.redacted_for_user(Some(&workflow), caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            endpoint,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowPromptEnqueued {
            queued_prompt,
            workflow,
            endpoint,
            session,
        } => LocalDaemonResponse::WorkflowPromptEnqueued {
            queued_prompt,
            workflow: workflow.redacted_for_user(caller_user_id),
            endpoint,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs,
            next_cursor,
        } => LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: workflow_runs
                .into_iter()
                .map(|workflow_run| {
                    let workflow = workflow_run_context.and_then(|session| {
                        session
                            .workflows()
                            .iter()
                            .find(|workflow| workflow.id() == workflow_run.workflow_id())
                    });
                    workflow_run.redacted_for_user(workflow, caller_user_id)
                })
                .collect(),
            next_cursor,
        },
        LocalDaemonResponse::WorkflowRun { workflow_run } => LocalDaemonResponse::WorkflowRun {
            workflow_run: {
                let workflow = workflow_run_context.and_then(|session| {
                    session
                        .workflows()
                        .iter()
                        .find(|workflow| workflow.id() == workflow_run.workflow_id())
                });
                workflow_run.redacted_for_user(workflow, caller_user_id)
            },
        },
        LocalDaemonResponse::WorkflowRunCancelled {
            workflow_run,
            session,
        } => {
            let redacted_run = {
                let workflow = session
                    .workflows()
                    .iter()
                    .find(|workflow| workflow.id() == workflow_run.workflow_id());
                workflow_run.redacted_for_user(workflow, caller_user_id)
            };
            LocalDaemonResponse::WorkflowRunCancelled {
                workflow_run: redacted_run,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowRunPaused {
            workflow_run,
            session,
        } => {
            let redacted_run = {
                let workflow = session
                    .workflows()
                    .iter()
                    .find(|workflow| workflow.id() == workflow_run.workflow_id());
                workflow_run.redacted_for_user(workflow, caller_user_id)
            };
            LocalDaemonResponse::WorkflowRunPaused {
                workflow_run: redacted_run,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowRunResumed {
            workflow_run,
            session,
        } => {
            let redacted_run = {
                let workflow = session
                    .workflows()
                    .iter()
                    .find(|workflow| workflow.id() == workflow_run.workflow_id());
                workflow_run.redacted_for_user(workflow, caller_user_id)
            };
            LocalDaemonResponse::WorkflowRunResumed {
                workflow_run: redacted_run,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session } => {
            LocalDaemonResponse::WorkflowFlushContextUpdated {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session } => {
            LocalDaemonResponse::WorkflowRunOutputSchemaUpdated {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowPromptQueueCreated { queue, session } => {
            LocalDaemonResponse::WorkflowPromptQueueCreated {
                queue,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowPromptQueueUpdated { queue, session } => {
            LocalDaemonResponse::WorkflowPromptQueueUpdated {
                queue,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::WorkflowPromptQueueRemoved { queue, session } => {
            LocalDaemonResponse::WorkflowPromptQueueRemoved {
                queue,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::QueuedWorkflowPromptUpdated {
            queued_prompt,
            session,
        } => LocalDaemonResponse::QueuedWorkflowPromptUpdated {
            queued_prompt,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::QueuedWorkflowPromptRemoved {
            queued_prompt,
            session,
        } => LocalDaemonResponse::QueuedWorkflowPromptRemoved {
            queued_prompt,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowPromptQueueCleared {
            queued_prompts,
            session,
        } => LocalDaemonResponse::WorkflowPromptQueueCleared {
            queued_prompts,
            session: session.redacted_for_user(caller_user_id),
        },
        LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        } => {
            let redacted_run = {
                let workflow = session
                    .workflows()
                    .iter()
                    .find(|workflow| workflow.id() == workflow_run.workflow_id());
                workflow_run.redacted_for_user(workflow, caller_user_id)
            };
            LocalDaemonResponse::WorkflowTurnAcknowledged {
                workflow_run: redacted_run,
                session: session.redacted_for_user(caller_user_id),
            }
        }
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        RuntimeSession, WorkflowDefinition, WorkflowEndpointDefinition, WorkflowNodeDefinition,
        WorkflowRun, WorkflowRunStatus,
    };

    fn workflow_run_redaction_fixture() -> (RuntimeSession, WorkflowRun, WorkflowRun) {
        let mut session = RuntimeSession::new(
            "session-redaction",
            None,
            "workspace-redaction",
            "worktree-redaction",
            "machine-redaction",
            "daemon-redaction",
        );
        let mut workflow = WorkflowDefinition::new("workflow-redaction", None);
        let mut node = WorkflowNodeDefinition::new("node-redaction", "agent-redaction");
        node.set_owner_user_id("owner");
        workflow.add_node(node);
        let mut endpoint =
            WorkflowEndpointDefinition::new("endpoint-redaction", None, "node-redaction");
        endpoint.set_owner_user_id("owner");
        workflow.add_endpoint(endpoint);
        session.create_workflow(workflow);

        let active = WorkflowRun::new(
            "run-active",
            "workflow-redaction",
            "endpoint-redaction",
            "node-redaction",
            Some("active private prompt".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        );
        let mut archived = WorkflowRun::new(
            "run-archived",
            "workflow-redaction",
            "endpoint-redaction",
            "node-redaction",
            Some("archived private prompt".to_string()),
            None,
            Vec::new(),
            Vec::new(),
        );
        archived.set_status(WorkflowRunStatus::Completed);
        (session, active, archived)
    }

    fn redacted_runs_for(
        caller_user_id: &str,
        response: LocalDaemonResponse,
        session: &RuntimeSession,
    ) -> LocalDaemonResponse {
        redact_response_for_user(
            response,
            caller_user_id,
            &ProviderRunProjectionStore::default(),
            Some(session),
        )
        .expect("workflow run response should redact")
    }

    #[test]
    fn workflow_run_list_preserves_owner_inputs_and_redacts_collaborators() {
        let (session, active, archived) = workflow_run_redaction_fixture();
        for caller in ["owner", "collaborator"] {
            let response = redacted_runs_for(
                caller,
                LocalDaemonResponse::WorkflowRunsListed {
                    workflow_runs: vec![active.clone(), archived.clone()],
                    next_cursor: Some("cursor-1".to_string()),
                },
                &session,
            );
            let LocalDaemonResponse::WorkflowRunsListed {
                workflow_runs,
                next_cursor,
            } = response
            else {
                panic!("unexpected response")
            };
            assert_eq!(next_cursor.as_deref(), Some("cursor-1"));
            assert_eq!(workflow_runs.len(), 2);
            assert_eq!(
                workflow_runs[0].invocation_prompt().is_some(),
                caller == "owner"
            );
            assert_eq!(
                workflow_runs[1].invocation_prompt().is_some(),
                caller == "owner"
            );
        }
    }

    #[test]
    fn workflow_run_get_preserves_owner_inputs_and_redacts_collaborators() {
        let (session, active, archived) = workflow_run_redaction_fixture();
        for run in [active, archived] {
            for caller in ["owner", "collaborator"] {
                let response = redacted_runs_for(
                    caller,
                    LocalDaemonResponse::WorkflowRun {
                        workflow_run: run.clone(),
                    },
                    &session,
                );
                let LocalDaemonResponse::WorkflowRun { workflow_run } = response else {
                    panic!("unexpected response")
                };
                assert_eq!(
                    workflow_run.invocation_prompt().is_some(),
                    caller == "owner"
                );
            }
        }
    }
}
