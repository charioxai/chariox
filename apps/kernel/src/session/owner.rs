use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

use crate::error::DaemonError;

use super::{
    CanonicalViewport, CreateSessionRequest, EnvironmentError, PromptQueueItem,
    RoomEnvironmentSnapshot, RuntimeProject, RuntimeSession, SessionConfigState, SessionService,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionStateStore {
    inner: Arc<RwLock<SessionService>>,
    identity: Arc<()>,
}

impl SessionStateStore {
    pub(crate) fn new(sessions: SessionService) -> Self {
        Self {
            inner: Arc::new(RwLock::new(sessions)),
            identity: Arc::new(()),
        }
    }

    /// Cloned handles share this process-local identity. Pending responders
    /// retain only the weak token, never the session store or a persisted ID.
    pub(crate) fn weak_identity(&self) -> Weak<()> {
        Arc::downgrade(&self.identity)
    }

    pub(crate) fn matches_identity(&self, identity: &Weak<()>) -> bool {
        Weak::ptr_eq(identity, &self.weak_identity())
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<'_, SessionService> {
        self.inner.read().expect("session state rwlock poisoned")
    }

    pub(crate) fn write(&self) -> RwLockWriteGuard<'_, SessionService> {
        self.inner.write().expect("session state rwlock poisoned")
    }

    pub(crate) fn snapshot(&self) -> SessionService {
        self.read().clone()
    }

    pub(crate) fn prompt_id_allocator(&self) -> super::PromptIdAllocator {
        self.read().prompt_id_allocator()
    }

    pub(crate) fn reserve_prompt_id(&self) -> String {
        self.read().reserve_prompt_id()
    }

    pub(crate) fn retire_ended_session(&self, session_id: &str) {
        self.write().retire_ended_session(session_id);
    }

    pub(crate) fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.read().get_session(session_id)
    }

    pub(crate) fn session_owner_user_id(&self, session_id: &str) -> Option<String> {
        self.read().session_owner_user_id(session_id)
    }

    pub(crate) fn try_get_session_for_projection(
        &self,
        session_id: &str,
    ) -> Option<Result<RuntimeSession, DaemonError>> {
        match self.inner.try_read() {
            Ok(service) => Some(service.get_session(session_id)),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("session state rwlock poisoned"),
        }
    }

    pub(crate) fn has_session(&self, session_id: &str) -> bool {
        self.read().has_session(session_id)
    }

    pub(crate) fn list_sessions(&self) -> Vec<RuntimeSession> {
        self.read().list_sessions()
    }

    pub(crate) fn list_non_ended_sessions_including_hidden(&self) -> Vec<RuntimeSession> {
        self.read().list_non_ended_sessions_including_hidden()
    }

    pub(crate) fn list_all_sessions(&self) -> Vec<RuntimeSession> {
        self.read().list_all_sessions()
    }

    pub(crate) fn list_projects(
        &self,
        owner_user_id: &str,
        include_archived: bool,
    ) -> Vec<RuntimeProject> {
        self.read().list_projects(owner_user_id, include_archived)
    }

    pub(crate) fn list_visible_projects(
        &self,
        owner_user_id: &str,
        include_archived: bool,
        is_member: impl Fn(&RuntimeSession) -> bool,
    ) -> Vec<RuntimeProject> {
        self.read()
            .list_visible_projects(owner_user_id, include_archived, is_member)
    }

    pub(crate) fn get_project(&self, project_id: &str) -> Result<RuntimeProject, DaemonError> {
        self.read().get_project(project_id)
    }

    /// A leased session retains the source Project even when its checkout is newly created.
    pub(crate) fn environment_project(&self, session: &RuntimeSession) -> Option<RuntimeProject> {
        self.get_project(session.project_id()).ok()
    }

    pub(crate) fn environment_project_for_checkout(
        &self,
        owner_user_id: &str,
        checkout: &str,
    ) -> Result<Option<RuntimeProject>, DaemonError> {
        let projects =
            self.projects_owning_checkout(self.list_projects(owner_user_id, false), checkout)?;
        match projects.as_slice() {
            [] => Ok(None),
            [project] => Ok(Some(project.clone())),
            _ => Err(crate::project_environment::environment_error(
                "this checkout belongs to several Projects; use a registered Project folder",
            )),
        }
    }

    /// Workspace roots, Project sessions and linked Git checkouts share one Environment.
    pub(crate) fn project_has_checkout(
        &self,
        project: &RuntimeProject,
        checkout: &str,
    ) -> Result<bool, DaemonError> {
        Ok(!self
            .projects_owning_checkout(vec![project.clone()], checkout)?
            .is_empty())
    }

    /// Workspace roots take precedence over session worktrees, then Git membership.
    pub(crate) fn projects_owning_checkout(
        &self,
        projects: Vec<RuntimeProject>,
        checkout: &str,
    ) -> Result<Vec<RuntimeProject>, DaemonError> {
        use crate::runtime::workspace_git_common::same_fs_path;
        let workspaces: Vec<_> = projects
            .iter()
            .filter(|project| {
                project
                    .workspace_ids()
                    .iter()
                    .any(|path| same_fs_path(path, checkout))
            })
            .cloned()
            .collect();
        if !workspaces.is_empty() {
            return Ok(workspaces);
        }
        let mut candidates = Vec::new();
        for project in projects {
            let mut paths = project.workspace_ids().to_vec();
            paths.extend(
                self.sessions_in_project(project.id())?
                    .into_iter()
                    .map(|session| session.worktree_id().to_string()),
            );
            candidates.push((project, paths));
        }
        let directly_owns =
            |paths: &[String]| paths.iter().any(|path| same_fs_path(path, checkout));
        if candidates.iter().any(|(_, paths)| directly_owns(paths)) {
            return Ok(candidates
                .into_iter()
                .filter(|(_, paths)| directly_owns(paths))
                .map(|(project, _)| project)
                .collect());
        }
        Ok(candidates
            .into_iter()
            .filter(|(_, paths)| {
                paths.iter().any(|root| {
                    crate::runtime::workspace_worktrees::list_workspace_worktrees(root, None)
                        .is_ok_and(|worktrees| {
                            worktrees
                                .iter()
                                .any(|worktree| same_fs_path(&worktree.path, checkout))
                        })
                })
            })
            .map(|(project, _)| project)
            .collect())
    }

    pub(crate) fn sessions_in_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<RuntimeSession>, DaemonError> {
        self.read().sessions_in_project(project_id)
    }

    pub(crate) fn durable_projects(&self) -> Vec<RuntimeProject> {
        self.read().durable_projects()
    }

    pub(crate) fn restore_projects(&self, projects: Vec<RuntimeProject>) {
        self.write().restore_projects(projects)
    }

    pub(crate) fn remove_projects_without_visible_sessions(
        &self,
    ) -> Result<Vec<RuntimeProject>, DaemonError> {
        self.write().remove_projects_without_visible_sessions()
    }

    pub(crate) fn reconcile_duplicate_project_names(&self) -> Vec<RuntimeProject> {
        self.write().reconcile_duplicate_project_names()
    }

    pub(crate) fn migrate_default_project_workspace(
        &self,
        session_id: &str,
        workspace_id: &str,
        default_project_name_hint: Option<&str>,
        replaced_project_ids: &std::collections::BTreeSet<String>,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        self.write().migrate_default_project_workspace(
            session_id,
            workspace_id,
            default_project_name_hint,
            replaced_project_ids,
        )
    }

    pub(crate) fn set_project_pinned(
        &self,
        project_id: &str,
        pinned: bool,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .set_project_pinned(project_id, pinned, caller_user_id)
    }

    pub(crate) fn rename_project(
        &self,
        project_id: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .rename_project(project_id, name, caller_user_id)
    }

    pub(crate) fn update_project_workspaces(
        &self,
        project_id: &str,
        workspace_ids: Vec<String>,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .update_project_workspaces(project_id, workspace_ids, caller_user_id)
    }

    pub(crate) fn archive_project(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write().archive_project(project_id, caller_user_id)
    }

    pub(crate) fn restore_project_status(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .restore_project_status(project_id, caller_user_id)
    }

    pub(crate) fn delete_project_record(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<RuntimeProject, DaemonError> {
        self.write()
            .delete_project_record(project_id, caller_user_id)
    }

    pub(crate) fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().create_session(request)
    }

    pub(crate) fn create_ephemeral_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().create_ephemeral_session(request)
    }

    pub(crate) fn restore_ephemeral_session(
        &self,
        session: RuntimeSession,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().restore_ephemeral_session(session)
    }

    pub(crate) fn is_ephemeral_session(&self, session_id: &str) -> bool {
        self.read().is_ephemeral_session(session_id)
    }

    pub(crate) fn restore_session(&self, session: RuntimeSession) -> RuntimeSession {
        self.write().restore_session(session)
    }

    pub(crate) fn commit_publication_runtime_configuration(
        &self,
        session: RuntimeSession,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .commit_publication_runtime_configuration(session)
    }

    pub(crate) fn restore_session_with_default_project_name_hint(
        &self,
        session: RuntimeSession,
        default_project_name_hint: Option<&str>,
    ) -> RuntimeSession {
        self.write()
            .restore_session_with_default_project_name_hint(session, default_project_name_hint)
    }

    pub(crate) fn restore_ended_session(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .transition_session(session_id, super::SessionStatus::Parked)
    }

    pub(crate) fn remove_restored_session(&self, session_id: &str) -> Option<RuntimeSession> {
        self.write().remove_restored_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn create_room_environment(
        &self,
        session_id: &str,
        environment_id: impl Into<String>,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .create_room_environment(session_id, environment_id, viewport)
    }

    pub(crate) fn room_environment_snapshot(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.read().room_environment_snapshot(session_id)
    }

    pub(crate) fn room_environment_events_after(
        &self,
        session_id: &str,
        cursor: u64,
    ) -> Result<super::EnvironmentReplay, EnvironmentError> {
        self.read()
            .room_environment_events_after(session_id, cursor)
    }

    pub(crate) fn room_environment_action_history(
        &self,
        session_id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<super::EnvironmentActionHistoryPage, EnvironmentError> {
        self.read()
            .room_environment_action_history(session_id, before_sequence, limit)
    }

    pub(crate) fn start_room_environment(
        &self,
        session_id: &str,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().start_room_environment(session_id, viewport)
    }

    pub(crate) fn stop_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().stop_room_environment(session_id)
    }

    pub(crate) fn begin_stop_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().begin_stop_room_environment(session_id)
    }

    pub(crate) fn complete_stop_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().complete_stop_room_environment(session_id)
    }

    pub(crate) fn retry_room_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().retry_room_environment(session_id)
    }

    pub(crate) fn transition_room_environment(
        &self,
        session_id: &str,
        lifecycle: super::EnvironmentLifecycle,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .transition_room_environment(session_id, lifecycle)
    }

    pub(crate) fn update_room_environment_component_health(
        &self,
        session_id: &str,
        component: super::EnvironmentComponent,
        state: super::EnvironmentComponentHealthState,
        diagnostic_code: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().update_room_environment_component_health(
            session_id,
            component,
            state,
            diagnostic_code,
        )
    }

    pub(crate) fn set_room_browser_bar_visible_as_actor(
        &self,
        session_id: &str,
        actor: super::EnvironmentActor,
        visible: bool,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .set_room_browser_bar_visible_as_actor(session_id, actor, visible)
    }

    pub(crate) fn preview_update_room_environment_viewport_as_actor(
        &self,
        session_id: &str,
        actor: super::EnvironmentActor,
        expected_revision: u64,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.read()
            .preview_update_room_environment_viewport_as_actor(
                session_id,
                actor,
                expected_revision,
                viewport,
            )
    }

    pub(crate) fn update_room_environment_viewport_as_actor(
        &self,
        session_id: &str,
        actor: super::EnvironmentActor,
        expected_revision: u64,
        viewport: CanonicalViewport,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().update_room_environment_viewport_as_actor(
            session_id,
            actor,
            expected_revision,
            viewport,
        )
    }

    pub(crate) fn update_room_environment_pointer_as_actor(
        &self,
        session_id: &str,
        actor: super::EnvironmentActor,
        runtime_generation: u64,
        viewport_revision: u64,
        position: Option<super::EnvironmentPointerPosition>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().update_room_environment_pointer_as_actor(
            session_id,
            actor,
            runtime_generation,
            viewport_revision,
            position,
        )
    }

    pub(crate) fn reconcile_room_environment_actors(
        &self,
        session_id: &str,
        actors: Vec<super::EnvironmentActor>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .reconcile_room_environment_actors(session_id, actors)
    }

    pub(crate) fn reconcile_room_environment_controller_tabs(
        &self,
        session_id: &str,
        tabs: Vec<super::EnvironmentTabObservation>,
        focused_runtime_target_id: Option<&str>,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write().reconcile_room_environment_controller_tabs(
            session_id,
            tabs,
            focused_runtime_target_id,
        )
    }

    pub(crate) fn room_environment_prepare_app_recovery(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .room_environment_prepare_app_recovery(session_id)
    }

    pub(crate) fn room_environment_cancel_app_recovery_focus(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .room_environment_cancel_app_recovery_focus(session_id)
    }

    pub(crate) fn set_room_environment_app_tabs(
        &self,
        session_id: &str,
        apps: std::collections::BTreeMap<String, (String, crate::session::AppPanelLayout)>,
        app_panels: bool,
    ) -> Result<std::collections::BTreeMap<String, (u32, u32)>, EnvironmentError> {
        self.write()
            .set_room_environment_app_tabs(session_id, apps, app_panels)
    }

    pub(crate) fn room_environment_controller_tab_binding(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<super::EnvironmentTabRuntimeBinding, EnvironmentError> {
        self.read()
            .room_environment_controller_tab_binding(session_id, tab_id)
    }

    pub(crate) fn room_environment_tab_id_for_controller_target(
        &self,
        session_id: &str,
        controller_target_id: &str,
    ) -> Result<Option<String>, EnvironmentError> {
        self.read()
            .room_environment_tab_id_for_controller_target(session_id, controller_target_id)
    }

    pub(crate) fn register_room_environment_element_references(
        &self,
        session_id: &str,
        tab_id: &str,
        runtime_generation: u64,
        document_revision: u64,
        controller_node_refs: impl IntoIterator<Item = String>,
    ) -> Result<std::collections::BTreeMap<String, String>, EnvironmentError> {
        self.write().register_room_environment_element_references(
            session_id,
            tab_id,
            runtime_generation,
            document_revision,
            controller_node_refs,
        )
    }

    pub(crate) fn resolve_room_environment_element_reference(
        &self,
        session_id: &str,
        reference_id: &str,
    ) -> Result<super::EnvironmentElementTarget, EnvironmentError> {
        self.read()
            .resolve_room_environment_element_reference(session_id, reference_id)
    }

    pub(crate) fn submit_room_environment_action(
        &self,
        session_id: &str,
        request: super::EnvironmentActionRequest,
    ) -> Result<(super::ActionAdmission, RoomEnvironmentSnapshot), EnvironmentError> {
        self.write()
            .submit_room_environment_action(session_id, request)
    }

    pub(crate) fn existing_room_environment_action(
        &self,
        session_id: &str,
        request: &super::EnvironmentActionRequest,
    ) -> Result<Option<super::ActionAdmission>, EnvironmentError> {
        self.read()
            .existing_room_environment_action(session_id, request)
    }

    pub(crate) fn finish_room_environment_action(
        &self,
        session_id: &str,
        action_id: &str,
        terminal: super::EnvironmentActionTerminal,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .finish_room_environment_action(session_id, action_id, terminal)
    }

    pub(crate) fn begin_room_environment_browser_controller_recovery(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .begin_room_environment_browser_controller_recovery(session_id)
    }

    pub(crate) fn complete_room_environment_browser_controller_recovery(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .complete_room_environment_browser_controller_recovery(session_id)
    }

    pub(crate) fn request_room_environment_takeover_as_actor(
        &self,
        session_id: &str,
        actor: super::EnvironmentActor,
        target: super::InputTarget,
    ) -> Result<(super::TakeoverOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        self.write()
            .request_room_environment_takeover_as_actor(session_id, actor, target)
    }

    pub(crate) fn release_room_environment_input(
        &self,
        session_id: &str,
        actor_id: &str,
        target: &super::InputTarget,
    ) -> Result<RoomEnvironmentSnapshot, EnvironmentError> {
        self.write()
            .release_room_environment_input(session_id, actor_id, target)
    }

    pub(crate) fn cancel_room_environment_action_as_actor(
        &self,
        session_id: &str,
        actor: super::EnvironmentActor,
        action_id: &str,
    ) -> Result<(super::ActionCancellationOutcome, RoomEnvironmentSnapshot), EnvironmentError> {
        self.write()
            .cancel_room_environment_action_as_actor(session_id, actor, action_id)
    }

    pub(crate) fn replace_publication_runtime_workflows(
        &self,
        session_id: &str,
        workflows: Vec<super::WorkflowDefinition>,
        workflow_prompt_queues: Vec<super::WorkflowPromptQueueDefinition>,
        workflow_watchdogs: Vec<super::WorkflowWatchdogDefinition>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().replace_publication_runtime_workflows(
            session_id,
            workflows,
            workflow_prompt_queues,
            workflow_watchdogs,
        )
    }

    pub(crate) fn end_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.write().end_session(session_id)
    }

    pub(crate) fn delete_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.write().delete_session(session_id)
    }

    pub(crate) fn delete_session_with_project_cleanup(
        &self,
        session_id: &str,
    ) -> Result<(RuntimeSession, Option<RuntimeProject>), DaemonError> {
        self.write().delete_session_with_project_cleanup(session_id)
    }

    pub(crate) fn set_active_provider_run(
        &self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .set_active_provider_run(session_id, provider_run_id)
    }

    pub(crate) fn mirror_agent_prompt_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .mirror_agent_prompt_state(session_id, agent_id, active_prompt, queued_prompts)
    }

    pub(crate) fn note_session_activity(
        &self,
        session_id: &str,
        timestamp_ms: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write().note_session_activity(session_id, timestamp_ms)
    }

    pub(crate) fn note_prompt_sent(
        &self,
        session_id: &str,
        agent_id: &str,
        timestamp_ms: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .note_prompt_sent(session_id, agent_id, timestamp_ms)
    }

    pub(crate) fn clear_agent_unread_output(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        self.write().clear_agent_unread_output(session_id, agent_id)
    }

    pub(crate) fn note_agent_output_sequence(
        &self,
        session_id: &str,
        agent_id: &str,
        sequence: u64,
    ) -> Result<RuntimeSession, DaemonError> {
        self.write()
            .note_agent_output_sequence(session_id, agent_id, sequence)
    }

    pub(crate) fn record_workflow_node_thinking_trace_for_node_run(
        &self,
        session_id: &str,
        workflow_node_run_id: &str,
        message: impl Into<String>,
    ) -> Result<Option<RuntimeSession>, DaemonError> {
        self.write()
            .record_workflow_node_thinking_trace_for_node_run(
                session_id,
                workflow_node_run_id,
                message,
            )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SessionStateReader {
    store: SessionStateStore,
}

impl SessionStateReader {
    pub(crate) fn new(store: SessionStateStore) -> Self {
        Self { store }
    }

    pub(crate) fn get_session(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store.read().get_session(session_id)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SessionStateOwner {
    store: SessionStateStore,
}

impl SessionStateOwner {
    pub(crate) fn new(store: SessionStateStore) -> Self {
        Self { store }
    }

    pub(crate) fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store.write().create_session(request)
    }

    pub(crate) fn assign_session_alias(
        &mut self,
        session_id: &str,
        alias: String,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store.write().assign_session_alias(session_id, alias)
    }

    pub(crate) fn update_config(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        values: BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<(RuntimeSession, SessionConfigState), DaemonError> {
        self.store
            .write()
            .update_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        self.store.write().end_session(session_id)
    }

    pub(crate) fn set_active_provider_run(
        &mut self,
        session_id: &str,
        provider_run_id: Option<String>,
    ) -> Result<RuntimeSession, DaemonError> {
        self.store
            .write()
            .set_active_provider_run(session_id, provider_run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked_project_fixture() -> (
        crate::test_support::TestWorktree,
        SessionStateStore,
        std::path::PathBuf,
    ) {
        let root = crate::test_support::TestWorktree::new("explicit-linked-project");
        let main = root.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        for args in [
            vec!["init"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "Initial",
            ],
        ] {
            assert!(crate::process_spawn::command("git")
                .args(args)
                .current_dir(&main)
                .output()
                .unwrap()
                .status
                .success());
        }
        let linked = root.path().join("linked");
        crate::git_worktree_placement::prepare_git_worktree(
            &crate::agent::GitWorktreePlacement {
                target_directory: Some(linked.display().to_string()),
                branch: Some("linked".into()),
                from_ref: Some("HEAD".into()),
            },
            &main,
            None,
            "test linked Project",
        )
        .unwrap();
        let store = SessionStateStore::new(SessionService::new(&crate::DaemonConfig::for_tests()));
        store.restore_projects(vec![
            RuntimeProject::new(
                "a-main",
                "owner",
                main.display().to_string(),
                "Main",
                crate::session::RuntimeProjectKind::Named,
            ),
            RuntimeProject::new(
                "z-linked",
                "owner",
                linked.display().to_string(),
                "Linked",
                crate::session::RuntimeProjectKind::Named,
            ),
        ]);
        (root, store, linked)
    }

    // MP-08/MP-10/MP-11: Git membership cannot override a checkout's registered Project.
    #[test]
    fn environment_explicit_project_owns_its_linked_checkout() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let (root, store, linked) = linked_project_fixture();
        store
            .create_session(
                CreateSessionRequest::new(
                    root.path().join("main").display().to_string(),
                    linked.display().to_string(),
                )
                .with_owner_user_id("owner")
                .with_project_selection(
                    crate::session::SessionProjectSelection::Existing {
                        project_id: "a-main".into(),
                    },
                ),
            )
            .unwrap();
        let hidden = store
            .create_session(
                CreateSessionRequest::new("remote-lease:test", linked.display().to_string())
                    .with_owner_user_id("owner")
                    .with_hidden(true)
                    .with_project_selection(crate::session::SessionProjectSelection::Existing {
                        project_id: "z-linked".into(),
                    }),
            )
            .unwrap();
        assert_eq!(store.environment_project(&hidden).unwrap().id(), "z-linked");
    }

    #[test]
    fn leased_worktree_placement_keeps_the_source_project_environment_when_unbound_is_ambiguous() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let (root, store, linked) = linked_project_fixture();
        let third = root.path().join("third");
        crate::git_worktree_placement::prepare_git_worktree(
            &crate::agent::GitWorktreePlacement {
                target_directory: Some(third.display().to_string()),
                branch: Some("third".into()),
                from_ref: Some("HEAD".into()),
            },
            &linked,
            None,
            "test ambiguous leased checkout",
        )
        .unwrap();
        let hidden = store
            .create_ephemeral_session(
                CreateSessionRequest::new("remote-lease:test", third.display().to_string())
                    .with_owner_user_id("owner")
                    .with_hidden(true),
            )
            .unwrap();
        assert!(
            store.environment_project(&hidden).is_none(),
            "Git membership alone cannot choose between registered Projects"
        );
    }

    #[test]
    fn environment_file_copy_uses_only_the_registered_checkout_project() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let (root, store, linked) = linked_project_fixture();
        let environment =
            crate::project_environment::ProjectEnvironmentStore::new(&root.path().join("state"));
        for (project, file) in [
            ("a-main", "main-only.local"),
            ("z-linked", "linked-only.local"),
        ] {
            environment
                .save(
                    project,
                    0,
                    crate::project_environment::ProjectEnvironmentRecord {
                        schema: crate::project_environment::PROJECT_ENVIRONMENT_SCHEMA,
                        files: vec![file.into()],
                        ..Default::default()
                    },
                )
                .unwrap();
            std::fs::write(linked.join(file), "fixture").unwrap();
        }
        let target = root.path().join("next-linked");
        crate::git_worktree_placement::prepare_git_worktree(
            &crate::agent::GitWorktreePlacement {
                target_directory: Some(target.display().to_string()),
                branch: Some("next-linked".into()),
                from_ref: Some("HEAD".into()),
            },
            &linked,
            None,
            "test Project file copy",
        )
        .unwrap();
        environment
            .copy_files_into_worktree(&store, &linked, &target)
            .unwrap();
        assert!(target.join("linked-only.local").is_file());
        assert!(
            !target.join("main-only.local").exists(),
            "the Project rooted in this checkout decides which files travel"
        );
    }

    #[test]
    fn independent_session_reads_can_hold_the_store_concurrently() {
        let store = SessionStateStore::new(SessionService::new(&crate::DaemonConfig::for_tests()));
        let first_reader = store.read();

        let second_reader = store
            .inner
            .try_read()
            .expect("a read must not exclude another read");

        drop(second_reader);
        drop(first_reader);
        assert!(store.inner.try_write().is_ok());
    }
}
