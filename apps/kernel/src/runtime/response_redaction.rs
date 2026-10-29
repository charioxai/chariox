use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::projection::ProviderRunProjectionStore;
use crate::runtime::provider_process_control::provider_processes_visible_to_user_from_projection;
use crate::runtime::session_projection_refresh::redact_agent_activity_for_session;
use crate::session::RuntimeSession;

pub(crate) fn redact_response_for_user(
    response: LocalDaemonResponse,
    caller_user_id: &str,
    provider_run_projection: &ProviderRunProjectionStore,
    workflow_run_context: Option<&RuntimeSession>,
    workflow_history: Option<&crate::durable_state::DurableKernelStateStore>,
) -> Result<LocalDaemonResponse, DaemonError> {
    let redact_session = |session: RuntimeSession| match workflow_history {
        Some(history) => session.redacted_for_user_with_workflow_history(caller_user_id, history),
        None => Ok(session.redacted_for_user(caller_user_id)),
    };
    Ok(match response {
        LocalDaemonResponse::AgentTriggerCreated { trigger, session } => {
            LocalDaemonResponse::AgentTriggerCreated {
                trigger,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::AgentTriggerUpdated { trigger, session } => {
            LocalDaemonResponse::AgentTriggerUpdated {
                trigger,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::AgentTriggerDeleted { trigger, session } => {
            LocalDaemonResponse::AgentTriggerDeleted {
                trigger,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::SessionCreated { session, agent, .. } => {
            LocalDaemonResponse::SessionCreated {
                session: redact_session(session)?,
                agent,
                event_subscription_error: None,
            }
        }
        LocalDaemonResponse::RoomPinned { session } => LocalDaemonResponse::RoomPinned {
            session: redact_session(session)?,
        },
        LocalDaemonResponse::SessionResolved { session } => LocalDaemonResponse::SessionResolved {
            session: redact_session(session)?,
        },
        LocalDaemonResponse::SessionState {
            room_workflows: _,
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let room_workflows =
                crate::runtime::projection::RoomWorkflowInventory::project_with_history(
                    &session,
                    caller_user_id,
                    workflow_history,
                )?;
            let session = redact_session(session)?;
            LocalDaemonResponse::SessionState {
                room_workflows,
                agent_activity: redact_agent_activity_for_session(agent_activity, &session),
                agent_activity_revision,
                session,
            }
        }
        LocalDaemonResponse::MetaagentTaskUpdated { session, task } => {
            let session = redact_session(session)?;
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
                .map(redact_session)
                .collect::<Result<Vec<_>, DaemonError>>()?,
        },
        LocalDaemonResponse::SessionInviteCreated { invite, session } => {
            LocalDaemonResponse::SessionInviteCreated {
                invite,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::SessionInviteJoined { member, session } => {
            LocalDaemonResponse::SessionInviteJoined {
                member,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::SessionInviteRevoked { invite, session } => {
            LocalDaemonResponse::SessionInviteRevoked {
                invite,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkspaceLinkCreated { link, session } => {
            LocalDaemonResponse::WorkspaceLinkCreated {
                link,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session,
        } => LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkspaceLinkDetached {
            link,
            detached,
            session,
        } => LocalDaemonResponse::WorkspaceLinkDetached {
            link,
            detached,
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkspaceLiveSyncModeUpdated { session, effects } => {
            LocalDaemonResponse::WorkspaceLiveSyncModeUpdated {
                session: redact_session(session)?,
                effects,
            }
        }
        LocalDaemonResponse::PromptSubmitted {
            outcome,
            session,
            agent_activity,
            agent_activity_revision,
        } => {
            let session = redact_session(session)?;
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
            let session = redact_session(session)?;
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
            let session = redact_session(session)?;
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
            let session = redact_session(session)?;
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
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::AgentConfigUpdated { agent, session } => {
            LocalDaemonResponse::AgentConfigUpdated {
                agent,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
            LocalDaemonResponse::AgentProfileUpdated {
                agent,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::AgentAliased { agent, session } => LocalDaemonResponse::AgentAliased {
            agent,
            session: redact_session(session)?,
        },
        LocalDaemonResponse::SessionEnded { session } => LocalDaemonResponse::SessionEnded {
            session: redact_session(session)?,
        },
        LocalDaemonResponse::SessionDeleted { session } => LocalDaemonResponse::SessionDeleted {
            session: redact_session(session)?,
        },
        LocalDaemonResponse::SessionAliased { session } => LocalDaemonResponse::SessionAliased {
            session: redact_session(session)?,
        },
        LocalDaemonResponse::AgentsListed { agents } => LocalDaemonResponse::AgentsListed {
            agents: agents
                .into_iter()
                .filter(|agent| agent.owner_user_id() == caller_user_id)
                .collect(),
        },
        LocalDaemonResponse::ProviderRun { provider_run } => {
            if !provider_run.owned_by(caller_user_id) {
                return Err(DaemonError::OwnershipAccessDenied {
                    user_id: caller_user_id.to_string(),
                    owner_user_id: provider_run.owner_user_id().to_string(),
                    resource: format!("provider run `{}`", provider_run.id()),
                    operation: "read provider run",
                });
            }
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
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowCodeApplied { result, session } => {
            LocalDaemonResponse::WorkflowCodeApplied {
                result,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowCodeRun { result, session } => {
            LocalDaemonResponse::WorkflowCodeRun {
                result,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowDesignOpAccepted { session, event } => {
            LocalDaemonResponse::WorkflowDesignOpAccepted {
                session: redact_session(session)?,
                event,
            }
        }
        LocalDaemonResponse::WorkflowAliased { workflow, session } => {
            LocalDaemonResponse::WorkflowAliased {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: redact_session(session)?,
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
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowPublicationDisabled {
            publication,
            session,
        } => LocalDaemonResponse::WorkflowPublicationDisabled {
            publication,
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id,
            session,
            agent_id_map,
        } => LocalDaemonResponse::WorkflowPublicationMaterialized {
            publication_id,
            session: redact_session(session)?,
            agent_id_map,
        },
        LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeAdded {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeRemoved {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node: node.redacted_for_user(caller_user_id),
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow,
            session,
        } => LocalDaemonResponse::WorkflowCanvasLayoutUpdated {
            layout,
            workflow: workflow.redacted_for_user(caller_user_id),
            session: redact_session(session)?,
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
            session: redact_session(session)?,
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
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs,
            next_cursor,
        } => LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: workflow_runs
                .into_iter()
                .filter_map(|run| {
                    let projected = workflow_run_context.map_or(Ok(None), |session| {
                        session.project_workflow_run_for_user(run, caller_user_id, workflow_history)
                    });
                    match projected {
                        Ok(Some(run)) => Some(Ok(run)),
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    }
                })
                .collect::<Result<Vec<_>, DaemonError>>()?,
            next_cursor,
        },
        LocalDaemonResponse::WorkflowRun { workflow_run } => LocalDaemonResponse::WorkflowRun {
            workflow_run: {
                let session = workflow_run_context.ok_or_else(|| {
                    DaemonError::MissingSessionCallerIdentity {
                        operation: "read workflow run".into(),
                    }
                })?;
                session
                    .project_workflow_run_for_user(workflow_run, caller_user_id, workflow_history)?
                    .ok_or_else(|| DaemonError::OwnershipAccessDenied {
                        user_id: caller_user_id.into(),
                        owner_user_id: session.owner_user_id().into(),
                        resource: "workflow run".into(),
                        operation: "read workflow run",
                    })?
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
                session: redact_session(session)?,
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
                session: redact_session(session)?,
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
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session } => {
            LocalDaemonResponse::WorkflowFlushContextUpdated {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session } => {
            LocalDaemonResponse::WorkflowRunOutputSchemaUpdated {
                workflow: workflow.redacted_for_user(caller_user_id),
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowPromptQueueCreated { queue, session } => {
            LocalDaemonResponse::WorkflowPromptQueueCreated {
                queue,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowPromptQueueUpdated { queue, session } => {
            LocalDaemonResponse::WorkflowPromptQueueUpdated {
                queue,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::WorkflowPromptQueueRemoved { queue, session } => {
            LocalDaemonResponse::WorkflowPromptQueueRemoved {
                queue,
                session: redact_session(session)?,
            }
        }
        LocalDaemonResponse::QueuedWorkflowPromptUpdated {
            queued_prompt,
            session,
        } => LocalDaemonResponse::QueuedWorkflowPromptUpdated {
            queued_prompt,
            session: redact_session(session)?,
        },
        LocalDaemonResponse::QueuedWorkflowPromptRemoved {
            queued_prompt,
            session,
        } => LocalDaemonResponse::QueuedWorkflowPromptRemoved {
            queued_prompt,
            session: redact_session(session)?,
        },
        LocalDaemonResponse::WorkflowPromptQueueCleared {
            queued_prompts,
            session,
        } => LocalDaemonResponse::WorkflowPromptQueueCleared {
            queued_prompts,
            session: redact_session(session)?,
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
                session: redact_session(session)?,
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
        session.add_member("owner", None, crate::session::CollaborationLevel::Full);
        session.add_member(
            "collaborator",
            None,
            crate::session::CollaborationLevel::Transparent,
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
        // These transparent-read fixtures have immutable agent provenance. Legacy
        // autonomous history needs the configured durable authority instead.
        (
            session,
            active.with_creator(Some("agent-redaction".into())),
            archived.with_creator(Some("agent-redaction".into())),
        )
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
            None,
        )
        .expect("workflow run response should redact")
    }

    #[test]
    fn workflow_run_list_preserves_authorized_owner_and_transparent_inputs() {
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
            assert!(workflow_runs[0].invocation_prompt().is_some());
            assert!(workflow_runs[1].invocation_prompt().is_some());
        }
    }

    #[test]
    fn workflow_run_get_preserves_authorized_owner_and_transparent_inputs() {
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
                assert!(workflow_run.invocation_prompt().is_some());
            }
        }
    }
    // MP-08 / MP-10 / MP-11: neither archived nor active foreign reads bypass private membership.
    #[test]
    fn private_workflow_read_responses_exclude_unproven_history() {
        let (session, active, archived) = workflow_run_redaction_fixture();
        let mut wire = serde_json::to_value(&session).unwrap();
        for member in wire["members"].as_array_mut().unwrap() {
            member["collaboration_level"] = serde_json::json!("private");
        }
        let session: RuntimeSession = serde_json::from_value(wire).unwrap();
        let response = redacted_runs_for(
            "collaborator",
            LocalDaemonResponse::WorkflowRunsListed {
                workflow_runs: vec![active.clone(), archived.clone()],
                next_cursor: None,
            },
            &session,
        );
        let LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } = response else {
            panic!("list response")
        };
        assert!(workflow_runs.is_empty());
        for run in [active, archived] {
            assert!(redact_response_for_user(
                LocalDaemonResponse::WorkflowRun { workflow_run: run },
                "collaborator",
                &ProviderRunProjectionStore::default(),
                Some(&session),
                None
            )
            .is_err());
        }
    }
}
