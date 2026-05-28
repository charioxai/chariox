use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::projection::ProviderRunProjectionStore;
use crate::runtime::provider_process_control::provider_processes_visible_to_user_from_projection;
use crate::runtime::provider_run_control::ensure_provider_run_visible_to_user;
use crate::runtime::session_projection_refresh::redact_agent_activity_for_session;

pub(crate) fn redact_response_for_user(
    response: LocalDaemonResponse,
    caller_user_id: &str,
    provider_run_projection: &ProviderRunProjectionStore,
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
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::SessionState {
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                session,
            }
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
        LocalDaemonResponse::WorkspaceLiveSyncModeUpdated { session } => {
            LocalDaemonResponse::WorkspaceLiveSyncModeUpdated {
                session: session.redacted_for_user(caller_user_id),
            }
        }
        LocalDaemonResponse::PromptSubmitted {
            outcome,
            session,
            agent_activity,
        } => {
            let session = session.redacted_for_user(caller_user_id);
            LocalDaemonResponse::PromptSubmitted {
                outcome,
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
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
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => {
            LocalDaemonResponse::WorkflowRunsListed {
                workflow_runs: workflow_runs
                    .into_iter()
                    .map(|workflow_run| workflow_run.redacted_for_user(None, caller_user_id))
                    .collect(),
            }
        }
        LocalDaemonResponse::WorkflowRun { workflow_run } => LocalDaemonResponse::WorkflowRun {
            workflow_run: workflow_run.redacted_for_user(None, caller_user_id),
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
        LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session } => {
            LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated {
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
