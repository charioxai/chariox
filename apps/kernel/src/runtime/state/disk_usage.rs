//! MP-08 / MP-09 / MP-10 / MP-11: disk scopes follow the kernel-owned Room and process roster.
use super::*;

impl KernelRuntimeState {
    pub(crate) fn session_disk_usage(
        &self,
        session_id: &str,
    ) -> Vec<crate::runtime::disk_usage::DiskUsageEntry> {
        let state = self.clone();
        self.owned.disk_usage.read(session_id, move |target| {
            let id = target.to_string();
            use crate::runtime::disk_usage::Directory;
            use crate::runtime::process_resources::SessionProcessOwner;
            let Ok(session) = state.owned.session_store.get_session(&id) else {
                return (Vec::new(), Vec::new());
            };
            let mut directories = vec![Directory {
                path: session.worktree_id().into(),
                owner: None,
            }];
            for agent in state.owned.agent_store.get_session_agents(&id) {
                let owner = Some(SessionProcessOwner::Agent(agent.id().to_string()));
                directories.push(Directory {
                    path: agent.worktree_id().unwrap_or(session.worktree_id()).into(),
                    owner: owner.clone(),
                });
                if let Some(path) = crate::agent_temp::dir(&id, agent.id()) {
                    directories.push(Directory {
                        path,
                        owner: owner.clone(),
                    });
                }
                if let Some(run) = state
                    .owned
                    .provider_store
                    .get_run_for_agent(&id, agent.id())
                {
                    if let Some(path) = run.working_directory() {
                        directories.push(Directory {
                            path: path.clone(),
                            owner: owner.clone(),
                        });
                    }
                    for key in [
                        "CODEX_HOME",
                        "CLAUDE_CONFIG_DIR",
                        "OPENCODE_CONFIG_DIR",
                        "XDG_STATE_HOME",
                    ] {
                        if let Some(path) = run.pty_env().get(key) {
                            directories.push(Directory {
                                path: path.into(),
                                owner: owner.clone(),
                            });
                        }
                    }
                }
            }
            (directories, state.session_processes(&id))
        })
    }
}
