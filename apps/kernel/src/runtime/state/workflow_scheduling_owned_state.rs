//! Workflow launch scheduling administration.
//!
//! Owns watchdog CRUD and manual launch queue commands. Direct run execution lives in
//! `workflow_run_request_runtime_state` and node dispatch scheduling lives in `workflow_dispatch`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_create_watchdog(
        &self,
        request: crate::local::CreateWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().create_workflow_watchdog(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.queue_ref.as_deref(),
            request.interval_seconds,
            request.invocation_prompt,
            request.policy,
            if request.max_wakeups_configured {
                Some(request.max_wakeups)
            } else {
                None
            },
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
            watchdog,
            workflow,
            endpoint,
            session,
        })
    }

    pub(super) fn workflow_list_watchdogs(
        &self,
        request: crate::local::ListWorkflowWatchdogsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
            watchdogs: self
                .session_store
                .read()
                .list_workflow_watchdogs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_create_schedule(
        &self,
        request: crate::local::CreateWorkflowScheduleRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let schedule = self.session_store.write().create_workflow_schedule(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.queue_ref.as_deref(),
            request.trigger,
            request.invocation_prompt,
            request.overlap_policy,
            if request.max_runs_configured {
                Some(request.max_runs)
            } else {
                None
            },
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowScheduleCreated {
            schedule,
            workflow,
            endpoint,
            session,
        })
    }

    pub(super) fn workflow_list_schedules(
        &self,
        request: crate::local::ListWorkflowSchedulesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowSchedulesListed {
            schedules: self
                .session_store
                .read()
                .list_workflow_schedules(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    pub(super) fn workflow_set_schedule_enabled(
        &self,
        request: crate::local::SetWorkflowScheduleEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let schedule = self.session_store.write().set_workflow_schedule_enabled(
            &request.session_id,
            &request.schedule_ref,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowScheduleUpdated { schedule, session })
    }

    pub(super) fn workflow_remove_schedule(
        &self,
        request: crate::local::RemoveWorkflowScheduleRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let schedule = self
            .session_store
            .write()
            .remove_workflow_schedule(&request.session_id, &request.schedule_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowScheduleRemoved { schedule, session })
    }

    pub(super) fn workflow_preview_schedule(
        &self,
        request: crate::local::PreviewWorkflowScheduleRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowSchedulePreviewed {
            preview: self.session_store.read().preview_workflow_schedule(
                request.trigger,
                request.after_ms,
                request.count.unwrap_or(3),
            )?,
        })
    }

    pub(super) fn workflow_set_watchdog_enabled(
        &self,
        request: crate::local::SetWorkflowWatchdogEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().set_workflow_watchdog_enabled(
            &request.session_id,
            &request.watchdog_ref,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
    }

    pub(super) fn workflow_remove_watchdog(
        &self,
        request: crate::local::RemoveWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self
            .session_store
            .write()
            .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
    }

    pub(super) fn workflow_list_prompt_queues(
        &self,
        request: crate::local::ListWorkflowPromptQueuesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowPromptQueuesListed {
            queues: self.session_store.read().list_workflow_prompt_queues(
                &request.session_id,
                request.workflow_ref.as_deref(),
            )?,
        })
    }

    pub(super) fn workflow_create_prompt_queue(
        &self,
        request: crate::local::CreateWorkflowPromptQueueRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_ref = self.workflow_prompt_queue_request_workflow_ref(
            &request.session_id,
            request.workflow_ref.as_deref(),
        )?;
        let queue = self.session_store.write().create_workflow_prompt_queue(
            &request.session_id,
            &workflow_ref,
            request.alias,
            request.priority,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPromptQueueCreated { queue, session })
    }

    pub(super) fn workflow_update_prompt_queue(
        &self,
        request: crate::local::UpdateWorkflowPromptQueueRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_ref = self.workflow_prompt_queue_request_workflow_ref(
            &request.session_id,
            request.workflow_ref.as_deref(),
        )?;
        let queue = self.session_store.write().update_workflow_prompt_queue(
            &request.session_id,
            &workflow_ref,
            &request.queue_ref,
            request.alias,
            request.priority,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPromptQueueUpdated { queue, session })
    }

    pub(super) fn workflow_remove_prompt_queue(
        &self,
        request: crate::local::RemoveWorkflowPromptQueueRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_ref = self.workflow_prompt_queue_request_workflow_ref(
            &request.session_id,
            request.workflow_ref.as_deref(),
        )?;
        let queue = self.session_store.write().remove_workflow_prompt_queue(
            &request.session_id,
            &workflow_ref,
            &request.queue_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPromptQueueRemoved { queue, session })
    }

    pub(super) fn workflow_list_queued_prompts(
        &self,
        request: crate::local::ListQueuedWorkflowPromptsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::QueuedWorkflowPromptsListed {
            queued_prompts: self
                .session_store
                .read()
                .list_queued_workflow_prompts(&request.session_id)?,
        })
    }

    pub(super) fn workflow_update_queued_prompt(
        &self,
        request: crate::local::UpdateQueuedWorkflowPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_prompt = self.session_store.write().update_queued_workflow_prompt(
            &request.session_id,
            &request.queue_item_ref,
            request.prompt,
            request.queue_ref.as_deref(),
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowPromptUpdated {
            queued_prompt,
            session,
        })
    }

    pub(super) fn workflow_remove_queued_prompt(
        &self,
        request: crate::local::RemoveQueuedWorkflowPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_prompt = self
            .session_store
            .write()
            .remove_queued_workflow_prompt(&request.session_id, &request.queue_item_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowPromptRemoved {
            queued_prompt,
            session,
        })
    }

    pub(super) fn workflow_clear_prompt_queue(
        &self,
        request: crate::local::ClearWorkflowPromptQueueRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_ref = self.workflow_prompt_queue_request_workflow_ref(
            &request.session_id,
            request.workflow_ref.as_deref(),
        )?;
        let queued_prompts = self.session_store.write().clear_workflow_queue(
            &request.session_id,
            &workflow_ref,
            &request.queue_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowPromptQueueCleared {
            queued_prompts,
            session,
        })
    }

    fn workflow_prompt_queue_request_workflow_ref(
        &self,
        session_id: &str,
        workflow_ref: Option<&str>,
    ) -> Result<String, DaemonError> {
        if let Some(workflow_ref) = workflow_ref {
            return Ok(workflow_ref.to_string());
        }
        let session = self.session_store.read().get_session(session_id)?;
        let mut workflows = session.workflows().iter();
        let Some(workflow) = workflows.next() else {
            return Err(DaemonError::WorkflowNotFound {
                session_id: session_id.to_string(),
                workflow_id: "workflow".to_string(),
            });
        };
        if workflows.next().is_some() {
            return Err(DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: "workflow".to_string(),
                reference: "workflow".to_string(),
                message: "workflow_ref is required when a session has multiple workflows",
            });
        }
        Ok(workflow.id().to_string())
    }
}
