//! `chariox.file.copy`, the home side: the calling agent copies a file between its own workspace
//! and the workspace of a leased agent of its session, over the execution-lease link. The result
//! is the size and SHA-256; the bytes never reach the model.
use super::*;
use crate::agent_file_copy::{
    pull, push, Copied, CopyOptions, LinkError, RemoteSource, RemoteWorkspace, SourceSlice,
    Workspace, WriteSlice,
};
use crate::runtime::kernel_files::StagedWrite;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RemoteFileBytes};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const FILE_COPY: &str = "chariox.file.copy";
/// One slice is at most 4 MiB, so the wait covers it over a slow uplink.
const SLICE_TIMEOUT: Duration = Duration::from_secs(90);
/// Worker answers that no retry can change: the lease does not admit this home, the leased agent
/// is gone, or the worker does not know the request.
const DEFINITIVE_LINK_CODES: [&str; 5] = [
    "unauthorized",
    "invalid_request",
    "kernel_runtime_role_denied",
    "leased_agent_not_found",
    "execution_lease_not_found",
];

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileCopyArgs {
    source: Endpoint,
    destination: Endpoint,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    expect_sha256: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    /// A leased agent of the caller's session; absent for the caller's own workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
    path: String,
}

pub(crate) fn is_file_copy_tool(tool_name: &str) -> bool {
    tool_name == FILE_COPY || tool_name == "chariox_file_copy"
}

pub(crate) fn file_copy_spec() -> RuntimeToolSpec {
    let endpoint = serde_json::json!({"type":"object","required":["path"],"properties":{
        "agent":{"type":"string","description":"A leased agent of this session (alias, agent ref or id) whose workspace is on another kernel; omit for your own workspace"},
        "path":{"type":"string","description":"File path relative to that workspace"}},
        "additionalProperties":false});
    RuntimeToolSpec {
        name: FILE_COPY.into(),
        description: "Copy one file byte for byte between your workspace and the workspace of a leased agent of this session on another kernel. Exactly one of source and destination names that agent. Returns only the size and SHA-256; the content never enters your context. For a commit, have the agent run `git bundle create` and copy the bundle; for a folder, copy a tar. Paths stay inside the workspaces, through real folders; nothing is written under .git or .chariox*, into a plan-mode agent, or over an existing file unless overwrite is true. At most 256 MiB.".into(),
        input_schema: serde_json::json!({"type":"object","required":["source","destination"],"properties":{
            "source": endpoint,
            "destination": endpoint,
            "overwrite":{"type":"boolean","default":false},
            "expect_sha256":{"type":"string","description":"Copy only when the source has this SHA-256"}},
            "additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn file_copy_tool(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args: FileCopyArgs =
            serde_json::from_value(arguments).map_err(|error| DaemonError::LocalTransport {
                operation: "chariox.file.copy",
                message: format!("invalid arguments: {error}"),
            })?;
        match self.copy_file(token, &args).await {
            Ok(copied) => Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "bytes": copied.bytes,
                    "sha256": copied.sha256,
                    "source": args.source,
                    "destination": args.destination,
                }),
            }),
            Err(reason) => Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({ "error": reason }),
            }),
        }
    }

    async fn copy_file(&self, token: &str, args: &FileCopyArgs) -> Result<Copied, String> {
        let (remote_reference, pulling) = match (&args.source.agent, &args.destination.agent) {
            (Some(agent), None) => (agent, true),
            (None, Some(agent)) => (agent, false),
            _ => {
                return Err(
                    "Name a leased agent on exactly one side: source.agent to fetch from it, destination.agent to send to it"
                        .to_owned(),
                )
            }
        };
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err("A file copy needs one running agent".to_owned());
        };
        if run.state() == crate::provider::ProviderRunState::Ended {
            return Err("Your provider run has ended".to_owned());
        }
        let root = run
            .working_directory()
            .cloned()
            .ok_or_else(|| "Your workspace is not available".to_owned())?;
        let local = Workspace {
            root,
            read_only: run.execution_mode() == crate::provider::AgentExecutionMode::Plan,
        };
        let agents = self.session_agents(run.session_id());
        let target = super::tool_dispatch::resolve_session_agent(&agents, remote_reference)?;
        let binding = target.remote_execution().cloned().ok_or_else(|| {
            format!(
                "`{remote_reference}` runs on this kernel; copy inside its workspace with your shell"
            )
        })?;
        let lookup = binding.clone();
        let config = self
            .with_app_side_effect(move |app| app.relay_config_for_remote_execution(&lookup))
            .await;
        let remote = LeasedWorkspace {
            runtime: self,
            config,
            binding,
        };
        let options = CopyOptions {
            overwrite: args.overwrite,
            expect_sha256: args.expect_sha256.clone(),
            ..Default::default()
        };
        if pulling {
            pull(
                &remote,
                &args.source.path,
                &local,
                &args.destination.path,
                &options,
            )
            .await
        } else {
            push(
                &local,
                &args.source.path,
                &remote,
                &args.destination.path,
                &options,
            )
            .await
        }
    }
}

/// A leased agent's workspace, reached through the one home-to-worker route of its lease.
struct LeasedWorkspace<'a> {
    runtime: &'a KernelRuntimeState,
    config: crate::config::DaemonConfig,
    binding: crate::agent::RemoteAgentBinding,
}

impl LeasedWorkspace<'_> {
    async fn ask(&self, request: RelayPeerRequest) -> Result<RelayPeerResponse, LinkError> {
        let response = match self
            .runtime
            .enqueue_leased_worker_request(
                &self.config,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(self.binding.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                request,
                SLICE_TIMEOUT,
            )
            .await
        {
            Ok(waiter) => waiter.wait().await,
            Err(error) => Err(error),
        };
        match response {
            Ok(RelayPeerResponse::FileRefused { reason }) => Err(LinkError::Refused(reason)),
            Ok(response) => Ok(response),
            Err(DaemonError::RelayTransport { code, message, .. })
                if DEFINITIVE_LINK_CODES.contains(&code.as_str()) =>
            {
                Err(LinkError::Refused(format!(
                    "The leased agent's kernel refused the copy: {message}"
                )))
            }
            Err(error) => Err(LinkError::Offline(error.to_string())),
        }
    }
}

fn unexpected() -> LinkError {
    LinkError::Refused("The leased agent's kernel answered with something unexpected".to_owned())
}

impl RemoteSource for LeasedWorkspace<'_> {
    async fn read(&self, path: &str, offset: u64) -> Result<SourceSlice, LinkError> {
        match self
            .ask(RelayPeerRequest::ReadLeasedAgentFile {
                leased_agent_id: self.binding.leased_agent_id.clone(),
                path: path.to_owned(),
                offset,
            })
            .await?
        {
            RelayPeerResponse::FileSliceRead {
                size_bytes,
                sha256,
                revision,
                content,
            } => Ok(SourceSlice {
                size_bytes,
                sha256,
                revision,
                bytes: content
                    .decode()
                    .ok_or_else(|| LinkError::Refused("A slice arrived damaged".to_owned()))?,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl RemoteWorkspace for LeasedWorkspace<'_> {
    async fn write(&self, path: &str, slice: &WriteSlice<'_>) -> Result<StagedWrite, LinkError> {
        match self
            .ask(RelayPeerRequest::WriteLeasedAgentFile {
                leased_agent_id: self.binding.leased_agent_id.clone(),
                path: path.to_owned(),
                offset: slice.offset,
                total_size: slice.total_size,
                sha256: slice.sha256.to_owned(),
                overwrite: slice.overwrite,
                content: RemoteFileBytes::encode(slice.bytes),
            })
            .await?
        {
            RelayPeerResponse::LeasedAgentFileWritten { received, sha256 } => {
                Ok(StagedWrite { received, sha256 })
            }
            _ => Err(unexpected()),
        }
    }

    async fn discard(&self, path: &str) -> Result<(), LinkError> {
        match self
            .ask(RelayPeerRequest::DiscardLeasedAgentFile {
                leased_agent_id: self.binding.leased_agent_id.clone(),
                path: path.to_owned(),
            })
            .await?
        {
            RelayPeerResponse::LeasedAgentFileDiscarded => Ok(()),
            _ => Err(unexpected()),
        }
    }
}
