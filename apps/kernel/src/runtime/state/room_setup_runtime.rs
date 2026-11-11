//! A new Room's setup: the kernel runs the steps the person asked for in the background and
//! every client sees their progress in the Room's projection. Prompts wait for the steps that
//! prepare the folder (see `RoomSetupStore`).
use super::room_setup_publish::short_reason;
use super::*;
use crate::runtime::workspace_git_common::{git_command_output, run_git};
use crate::runtime::workspace_worktrees::create_waiting_room_worktree;
use crate::session::{RoomPublishRequest, RoomSetupRequest, RoomSetupStepId};

impl KernelRuntimeState {
    /// Runs the steps of a Room just created with `setup`, in order, and returns at once.
    pub(super) fn spawn_room_setup(
        &self,
        session_id: &str,
        agent_id: &str,
        setup: RoomSetupRequest,
    ) {
        let runtime = self.as_kernel();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            runtime.run_room_setup(&session_id, &agent_id, setup).await;
        });
    }

    async fn run_room_setup(&self, session_id: &str, agent_id: &str, setup: RoomSetupRequest) {
        let steps_ran = async {
            if setup.git_init
                && !self
                    .run_room_setup_step(
                        session_id,
                        RoomSetupStepId::GitInit,
                        self.room_git_init(session_id),
                    )
                    .await
            {
                return;
            }
            if setup.new_worktree
                && !self
                    .run_room_setup_step(
                        session_id,
                        RoomSetupStepId::Worktree,
                        self.room_new_worktree(session_id, agent_id),
                    )
                    .await
            {
                return;
            }
            if let Some(publish) = setup.publish.as_ref() {
                self.run_room_publish_step(session_id, agent_id, publish)
                    .await;
            }
        };
        steps_ran.await;
        self.release_room_prompts(session_id);
    }

    async fn run_room_publish_step(
        &self,
        session_id: &str,
        agent_id: &str,
        publish: &RoomPublishRequest,
    ) {
        self.run_room_setup_step(
            session_id,
            RoomSetupStepId::Publish,
            self.run_room_publish(session_id, agent_id, publish),
        )
        .await;
    }

    /// Marks the step running, awaits it and records its result. Returns whether it succeeded.
    async fn run_room_setup_step(
        &self,
        session_id: &str,
        step: RoomSetupStepId,
        work: impl std::future::Future<Output = Result<(), String>>,
    ) -> bool {
        self.owned.room_setup.start(session_id, step);
        self.publish_room_setup(session_id);
        let result = work.await;
        let succeeded = result.is_ok();
        if let Err(message) = &result {
            tracing::warn!(
                session_id,
                step = step.label(),
                message,
                "room setup step failed"
            );
        }
        self.owned.room_setup.finish(session_id, step, result);
        self.publish_room_setup(session_id);
        succeeded
    }

    /// Shows the current setup to every client of the Room.
    fn publish_room_setup(&self, session_id: &str) {
        if let Err(error) = self.owned.session_snapshot(session_id) {
            tracing::warn!(session_id, %error, "room setup progress could not be projected");
        }
    }

    /// When the Room's prompts no longer wait, starts the ones that queued up.
    fn release_room_prompts(&self, session_id: &str) {
        if self.owned.room_setup.holds_prompts(session_id) {
            return;
        }
        for agent in self.owned.agent_store.get_session_agents(session_id) {
            if let Err(error) = self.spawn_idle_local_prompt_queue_promotion(session_id, agent.id())
            {
                tracing::warn!(session_id, %error, "queued prompt could not start after room setup");
            }
        }
    }

    async fn room_git_init(&self, session_id: &str) -> Result<(), String> {
        let folder = self
            .owned
            .session_store
            .get_session(session_id)
            .map_err(|error| setup_reason(&error))?
            .worktree_id()
            .to_string();
        tokio::task::spawn_blocking(move || {
            if git_command_output(&folder, &["rev-parse", "--git-dir"]).is_some() {
                return Err(
                    "this folder is already a Git repository; dismiss to continue in it"
                        .to_string(),
                );
            }
            run_git(std::path::Path::new(&folder), &["init"]).map_err(|error| {
                format!(
                    "git init failed: {}; initialize Git in the folder yourself, then dismiss",
                    setup_reason(&error)
                )
            })
        })
        .await
        .unwrap_or_else(|_| {
            Err("git init stopped unexpectedly; dismiss to continue without it".into())
        })
    }

    /// Creates a new worktree of the Room's workspace and moves the first agent into it, the way
    /// an agent config update moves it.
    async fn room_new_worktree(&self, session_id: &str, agent_id: &str) -> Result<(), String> {
        let session = self
            .owned
            .session_store
            .get_session(session_id)
            .map_err(|error| setup_reason(&error))?;
        let workspace = session.workspace_id().to_string();
        let current = session.worktree_id().to_string();
        let owner = session.owner_user_id().to_string();
        let created = {
            let (workspace, current) = (workspace.clone(), current.clone());
            tokio::task::spawn_blocking(move || {
                create_waiting_room_worktree(&workspace, None, None, None, Some(&current), Some(&workspace))
            })
            .await
            .map_err(|_| "creating the worktree stopped unexpectedly; dismiss to continue in the current folder".to_string())?
            .map_err(|error| failed_worktree(&error))?
        };
        self.copy_environment_files_into_worktree(
            std::path::Path::new(&current),
            std::path::Path::new(&created.path),
        )
        .map_err(|error| failed_worktree(&error))?;
        self.update_agent_config(
            session_id,
            agent_id,
            &owner,
            None,
            None,
            None,
            Some(Some(created.path)),
        )
        .await
        .map_err(|error| failed_worktree(&error))?;
        Ok(())
    }

    /// The name a new Room created now in `workspace_id` would get; creates nothing.
    pub(crate) fn suggest_session_alias(&self, workspace_id: &str) -> String {
        // Creation names the Room after its canonical workspace; a linked worktree names its repository.
        let workspace = crate::runtime::workspace_git_common::canonical_workspace_path(
            workspace_id,
            workspace_id,
        )
        .unwrap_or_else(|| workspace_id.to_string());
        self.owned
            .session_store
            .read()
            .default_session_alias(&workspace)
    }

    pub(crate) async fn retry_room_setup(
        &self,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.ensure_room_control(session_id, caller_user_id)?;
        let (agent_id, publish) = self.owned.room_setup.retry_publish(session_id)?;
        let session = self.owned.session_snapshot(session_id)?;
        let runtime = self.as_kernel();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            runtime
                .run_room_publish_step(&session_id, &agent_id, &publish)
                .await;
            runtime.release_room_prompts(&session_id);
        });
        Ok(session)
    }

    pub(crate) fn dismiss_room_setup(
        &self,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.ensure_room_control(session_id, caller_user_id)?;
        let dismissed = self.owned.room_setup.dismiss(session_id);
        let session = self.owned.session_snapshot(session_id)?;
        if dismissed {
            self.release_room_prompts(session_id);
        }
        Ok(session)
    }
}

fn setup_reason(error: &DaemonError) -> String {
    match error {
        DaemonError::LocalTransport { message, .. } => short_reason(message),
        other => short_reason(&other.to_string()),
    }
}

fn failed_worktree(error: &DaemonError) -> String {
    format!(
        "{}; dismiss to continue in the current folder",
        setup_reason(error)
    )
}
