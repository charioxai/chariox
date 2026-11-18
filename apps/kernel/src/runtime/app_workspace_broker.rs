//! SDK calls (`workspace.*`): an App whose signed manifest declares
//! `capabilities.workspace` reads, commits and pushes a worktree. A call needs
//! a person, or an agent whose call a person approved. A push goes to the
//! worktree's GitHub `origin` as the owner's account bound to the declared
//! credential; the token goes to git by environment and the App never sees it.
//!
//! - `workspace.changes {worktree}` -> `{files, branch, defaultBranch, remote}`
//! - `workspace.commit {worktree, files, message, branch?}` -> `{commit, branch, createdBranch}`
//! - `workspace.push {worktree, account}` -> `{commit, branch}`
use super::app_approved_operations::ApprovedOperations;
use super::app_http::KernelAccountVault;
use super::app_lifecycle::EventConfig;
use super::app_operation_budget::AppOperationBudget;
use super::state::Acting;
use super::workspace_git_actions::{
    commit_workspace_changes, default_branch, github_https_repository, push_workspace_branch,
    PushCredential,
};
use super::workspace_git_changes::workspace_git_file_changes;
use super::workspace_git_common::{detect_git_branch, git_command_output, resolve_repo_root};
use crate::durable_state::DurableKernelStateStore;
use chariox_app_package::{RouteSafety, WorkspaceAccess, WORKSPACE_ACCOUNT_ORIGIN};
use chariox_app_runtime::{wire::RemoteError, worker_peer::BrokerRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Changes {
    worktree: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Commit {
    worktree: String,
    files: Vec<String>,
    message: String,
    branch: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Push {
    worktree: String,
    account: String,
}

#[derive(Clone)]
pub(crate) struct AppWorkspaceBroker {
    store: DurableKernelStateStore,
    owner: String,
    installation: String,
    declared: Option<WorkspaceAccess>,
    admission: Arc<Semaphore>,
    config: EventConfig,
    host: super::app_event_routing_broker::RoutingHostSlot,
}

impl AppWorkspaceBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        installation: String,
        declared: Option<WorkspaceAccess>,
        admission: Arc<Semaphore>,
        config: EventConfig,
        host: super::app_event_routing_broker::RoutingHostSlot,
    ) -> Self {
        Self {
            store,
            owner,
            installation,
            declared,
            admission,
            config,
            host,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let budget = AppOperationBudget::from_broker(&request);
        // An App that did not declare the capability has no such import; the
        // call can only come from a forged peer.
        let declared = self
            .declared
            .clone()
            .ok_or_else(|| error("METHOD_UNAVAILABLE", false))?;
        let host = self
            .host
            .get()
            .ok_or_else(|| error("CONNECTION_UNAVAILABLE", true))?;
        authorize(
            &request.callers,
            host.state.app_control().approved_operations(),
        )?;
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY", true))?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            budget
                .check()
                .map_err(|_| error("APP_OPERATION_STOPPED", false))?;
            match request.method.as_str() {
                "workspace.changes" => changes(input(request.params)?),
                "workspace.commit" => commit(input(request.params)?),
                "workspace.push" => service.push(&declared, input(request.params)?),
                _ => Err(error("METHOD_UNAVAILABLE", false)),
            }
        })
        .await
        .map_err(|_| error("APP_WORKSPACE_OUTCOME_UNCERTAIN", false))?
    }

    fn push(&self, declared: &WorkspaceAccess, push: Push) -> Result<Value, RemoteError> {
        let binding = self
            .store
            .app_account(&self.owner, &self.installation, &push.account)
            .map_err(|_| error("STORAGE_UNAVAILABLE", true))?
            .filter(|binding| binding.credential == declared.credential)
            .ok_or_else(|| error("ACCOUNT_UNKNOWN", false))?;
        if binding.allow < RouteSafety::Write {
            return Err(error("ACCOUNT_NOT_ALLOWED", false));
        }
        let token = KernelAccountVault(self.config.clone())
            .token(&binding.vault_credential_id, WORKSPACE_ACCOUNT_ORIGIN)
            .map_err(|_| error("ACCOUNT_UNAVAILABLE", true))?;
        let result = push_workspace_branch(
            &push.worktree,
            &push.worktree,
            &PushCredential::from_token(&token),
        )
        .map_err(failed)?;
        Ok(json!({"commit": result.commit_sha, "branch": result.branch}))
    }
}

/// A worktree changes only for a person, or for an agent whose call a person
/// approved; an agent alone or an event handler never does.
fn authorize(callers: &[Value], approved: &ApprovedOperations) -> Result<(), RemoteError> {
    match Acting::of(callers, approved) {
        Ok(Acting::Person | Acting::Approved(_)) => Ok(()),
        _ => Err(error("APPROVAL_REQUIRED", false)),
    }
}

fn changes(input: Changes) -> Result<Value, RemoteError> {
    let worktree = input.worktree.trim();
    resolve_repo_root(worktree).map_err(failed)?;
    let files = workspace_git_file_changes(worktree, "HEAD").map_err(failed)?;
    let remote = git_command_output(worktree, &["remote", "get-url", "origin"])
        .and_then(|url| github_https_repository(&url));
    Ok(json!({
        "files": files.iter().map(|file| json!({
            "path": file.path,
            "status": file.status,
            "additions": file.additions,
            "deletions": file.deletions,
        })).collect::<Vec<_>>(),
        "branch": detect_git_branch(worktree).map_err(failed)?,
        "defaultBranch": default_branch(worktree, &[]).map_err(failed)?,
        "remote": remote,
    }))
}

fn commit(input: Commit) -> Result<Value, RemoteError> {
    let outcome = commit_workspace_changes(
        &input.worktree,
        &input.worktree,
        &input.files,
        &input.message,
        input.branch.as_deref(),
    )
    .map_err(failed)?;
    Ok(json!({
        "commit": outcome.result.commit_sha,
        "branch": outcome.result.branch,
        "createdBranch": outcome.created_branch,
    }))
}

fn input<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, RemoteError> {
    serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))
}

/// A git refusal says what to do next; it never carries the token (the push
/// scrubs it) and the App needs the words to tell the person.
fn failed(failure: crate::error::DaemonError) -> RemoteError {
    RemoteError {
        code: "WORKSPACE_ACTION_FAILED".into(),
        message: failure.to_string(),
        retryable: Some(false),
    }
}

fn error(code: &str, retryable: bool) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: code.to_ascii_lowercase().replace('_', " "),
        retryable: Some(retryable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::app_accounts::{AccountBinding, AccountCommand};
    use crate::test_temp_dir::TestTempDir;

    fn broker(
        store: DurableKernelStateStore,
        declared: Option<WorkspaceAccess>,
    ) -> AppWorkspaceBroker {
        AppWorkspaceBroker::new(
            store,
            "alice".into(),
            "github".into(),
            declared,
            Arc::new(Semaphore::new(1)),
            EventConfig::default(),
            Default::default(),
        )
    }

    fn request(method: &str, params: Value) -> BrokerRequest {
        let (_signal, cancellation) =
            chariox_app_runtime::worker_peer::BrokerCancellation::fixture();
        BrokerRequest {
            id: "call".into(),
            method: method.into(),
            params,
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(30),
            cancellation,
            callers: vec![],
            open_calls: vec![],
        }
    }

    fn declared() -> Option<WorkspaceAccess> {
        Some(WorkspaceAccess {
            credential: "github".into(),
        })
    }

    #[tokio::test]
    async fn an_app_without_the_capability_or_a_live_kernel_cannot_reach_a_worktree() {
        let dir = TestTempDir::new("workspace-broker-gate");
        let store = DurableKernelStateStore::open_owned(dir.join("kernel.sqlite")).unwrap();
        let params = json!({"worktree": dir.display().to_string()});
        let undeclared = broker(store.clone(), None);
        assert_eq!(
            undeclared
                .dispatch(request("workspace.changes", params.clone()))
                .await
                .unwrap_err()
                .code,
            "METHOD_UNAVAILABLE"
        );
        let no_host = broker(store, declared());
        assert_eq!(
            no_host
                .dispatch(request("workspace.changes", params))
                .await
                .unwrap_err()
                .code,
            "CONNECTION_UNAVAILABLE"
        );
    }

    #[test]
    fn only_a_person_or_an_approved_agent_call_acts_on_a_worktree() {
        let approved = ApprovedOperations::default();
        let person = json!({"kind": "human", "id": "alice"});
        let agent =
            |operation: &str| json!({"kind": "agent", "id": "agent-1", "operation_id": operation});
        authorize(std::slice::from_ref(&person), &approved).unwrap();
        let refused = |callers: &[Value], approved: &ApprovedOperations| {
            authorize(callers, approved).unwrap_err().code
        };
        assert_eq!(refused(&[agent("op-1")], &approved), "APPROVAL_REQUIRED");
        assert_eq!(refused(&[], &approved), "APPROVAL_REQUIRED");
        assert_eq!(
            refused(&[json!({"kind": "background", "id": "wake"})], &approved),
            "APPROVAL_REQUIRED"
        );
        let _approval = approved.approve("op-1");
        authorize(&[agent("op-1")], &approved).unwrap();
        // A person's call beside an agent's lends the agent nothing.
        assert_eq!(
            refused(&[person, agent("op-2")], &approved),
            "APPROVAL_REQUIRED"
        );
    }

    fn bind(store: &DurableKernelStateStore, credential: &str, allow: RouteSafety) {
        store
            .app_account_command(AccountCommand::Bind {
                owner: "alice".into(),
                installation: "github".into(),
                binding: AccountBinding {
                    account: "work".into(),
                    credential: credential.into(),
                    vault_credential_id: "vault-1".into(),
                    allow,
                    bound_at_ms: 1,
                },
            })
            .unwrap();
    }

    #[test]
    fn a_push_needs_an_account_of_the_declared_credential_that_may_write() {
        let dir = TestTempDir::new("workspace-broker-push");
        let store = DurableKernelStateStore::open_owned(dir.join("kernel.sqlite")).unwrap();
        let broker = broker(store.clone(), declared());
        let declared = declared().unwrap();
        let push = |account: &str| Push {
            worktree: dir.display().to_string(),
            account: account.into(),
        };
        assert_eq!(
            broker.push(&declared, push("work")).unwrap_err().code,
            "ACCOUNT_UNKNOWN"
        );
        bind(&store, "other", RouteSafety::Destructive);
        assert_eq!(
            broker.push(&declared, push("work")).unwrap_err().code,
            "ACCOUNT_UNKNOWN"
        );
        bind(&store, "github", RouteSafety::Read);
        assert_eq!(
            broker.push(&declared, push("work")).unwrap_err().code,
            "ACCOUNT_NOT_ALLOWED"
        );
    }
}
