//! Agent tools for the files of the owner's machines: list a folder, read a page, copy or move
//! a file or folder from one machine to another. A copy is a kernel transfer job like the one a
//! client starts; the bytes go from machine to machine and the model sees only the outcome.
use super::*;
use crate::local::{
    FileTransferConflict, FileTransferEndpoint, FileTransferJob, KernelDirectoryEntry,
    StartFileTransferRequest,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const FILES_LIST: &str = "chariox.files.list";
pub(crate) const FILES_READ: &str = "chariox.files.read";
pub(crate) const FILES_COPY: &str = "chariox.files.copy";
pub(crate) const FILES_TRANSFER: &str = "chariox.files.transfer";

const SLICE_TIMEOUT: Duration = Duration::from_secs(90);
const READ_DEFAULT_BYTES: u64 = 16 * 1024;
const READ_MAX_BYTES: u64 = 64 * 1024;
const WAIT_DEFAULT_SECONDS: u64 = 300;
const WAIT_MAX_SECONDS: u64 = 900;
const POLL: Duration = Duration::from_millis(400);

pub(crate) fn is_files_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.replace('_', ".").as_str(),
        "chariox.files.list"
            | "chariox.files.read"
            | "chariox.files.copy"
            | "chariox.files.transfer"
    )
}

pub(crate) fn files_tool_specs() -> Vec<RuntimeToolSpec> {
    let machine = serde_json::json!({"type":"string","description":"Another machine of yours (kernel id, kernel name or machine name); omit for this machine"});
    vec![
        RuntimeToolSpec {
            name: FILES_LIST.into(),
            description: "List a folder on this machine or another machine of yours: name, kind, size, modified time and revision of each entry, in name order. Paths are absolute or start with ~/. The kernel's own state and Vault are never listed.".into(),
            input_schema: serde_json::json!({"type":"object","required":["path"],"properties":{
                "machine": machine,
                "path":{"type":"string"},
                "offset":{"type":"integer","minimum":0,"description":"Skip this many entries; continue a truncated listing at offset + entries returned"}},
                "additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: FILES_READ.into(),
            description: "Read a page of a file on this machine or another machine of yours: at most 64 KiB from `offset`, as text, or base64 when the bytes are not text. To move a file use chariox.files.copy, which keeps the bytes out of your context. The kernel's own state and Vault are never read.".into(),
            input_schema: serde_json::json!({"type":"object","required":["path"],"properties":{
                "machine": machine,
                "path":{"type":"string"},
                "offset":{"type":"integer","minimum":0,"default":0},
                "length":{"type":"integer","minimum":1,"maximum":READ_MAX_BYTES,"default":READ_DEFAULT_BYTES}},
                "additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: FILES_COPY.into(),
            description: "Copy or move a file or folder from one machine of yours to a folder on another (or the same). The destination machine pulls the bytes from the source over the relay, verified by SHA-256, staged and published only when whole; the content never enters your context. A folder is copied whole or not at all. A move deletes the source only after the destination holds a verified copy. `conflict` says what to do when the name is taken: fail (default), replace, keep_both or skip. The kernel's own state and Vault are never copied from or to. Waits up to `wait_seconds` for the end; a longer transfer keeps running and chariox.files.transfer reports on it.".into(),
            input_schema: serde_json::json!({"type":"object","required":["source","destination"],"properties":{
                "source":{"type":"object","required":["path"],"properties":{"machine":machine,"path":{"type":"string"}},"additionalProperties":false},
                "destination":{"type":"object","required":["folder"],"properties":{"machine":machine,"folder":{"type":"string","description":"The folder the item lands in"},"name":{"type":"string","description":"The item's name there; the source's own name by default"}},"additionalProperties":false},
                "conflict":{"type":"string","enum":["fail","replace","keep_both","skip"],"default":"fail"},
                "move":{"type":"boolean","default":false},
                "wait_seconds":{"type":"integer","minimum":0,"maximum":WAIT_MAX_SECONDS,"default":WAIT_DEFAULT_SECONDS}},
                "additionalProperties":false}),
        },
        RuntimeToolSpec {
            name: FILES_TRANSFER.into(),
            description: "How a file transfer started with chariox.files.copy stands, or stop it. `machine` is the destination machine of that copy.".into(),
            input_schema: serde_json::json!({"type":"object","required":["job_id"],"properties":{
                "job_id":{"type":"string"},
                "machine": machine,
                "cancel":{"type":"boolean","default":false},
                "wait_seconds":{"type":"integer","minimum":0,"maximum":WAIT_MAX_SECONDS,"default":0}},
                "additionalProperties":false}),
        },
    ]
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    machine: Option<String>,
    path: String,
    #[serde(default)]
    offset: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    #[serde(default)]
    machine: Option<String>,
    path: String,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    length: Option<u64>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceArgs {
    #[serde(default)]
    machine: Option<String>,
    path: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationArgs {
    #[serde(default)]
    machine: Option<String>,
    folder: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyArgs {
    source: SourceArgs,
    destination: DestinationArgs,
    #[serde(default)]
    conflict: FileTransferConflict,
    #[serde(default, rename = "move")]
    delete_source: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferArgs {
    job_id: String,
    #[serde(default)]
    machine: Option<String>,
    #[serde(default)]
    cancel: bool,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

/// A machine an agent named: this kernel, or another kernel of its owner.
enum Target {
    Here,
    There { kernel: String, name: String },
}

fn failure(reason: impl Into<String>) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({ "error": reason.into() }),
    }
}

fn success(payload: serde_json::Value) -> RuntimeToolResult {
    RuntimeToolResult { ok: true, payload }
}

fn bad_arguments(tool: &'static str, error: serde_json::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: tool,
        message: format!("invalid arguments: {error}"),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn files_tool(
        &self,
        token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let agent = self.files_tool_caller(token)?;
        let outcome = match tool_name.replace('_', ".").as_str() {
            FILES_LIST => {
                let args: ListArgs =
                    serde_json::from_value(arguments).map_err(|e| bad_arguments(FILES_LIST, e))?;
                self.files_list(&agent, args).await
            }
            FILES_READ => {
                let args: ReadArgs =
                    serde_json::from_value(arguments).map_err(|e| bad_arguments(FILES_READ, e))?;
                self.files_read(&agent, args).await
            }
            FILES_COPY => {
                let args: CopyArgs =
                    serde_json::from_value(arguments).map_err(|e| bad_arguments(FILES_COPY, e))?;
                self.files_copy(&agent, args).await
            }
            _ => {
                let args: TransferArgs = serde_json::from_value(arguments)
                    .map_err(|e| bad_arguments(FILES_TRANSFER, e))?;
                self.files_transfer(&agent, args).await
            }
        };
        Ok(outcome.unwrap_or_else(failure))
    }

    /// The agent behind a running turn's token.
    fn files_tool_caller(&self, token: &str) -> Result<crate::agent::AgentInstance, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(refusal_error("Files tools need one running agent"));
        };
        if run.state() == crate::provider::ProviderRunState::Ended {
            return Err(refusal_error("Your provider run has ended"));
        }
        let agent_id = run
            .agent_instance_id()
            .ok_or_else(|| refusal_error("Files tools need an agent's own turn"))?;
        self.owned.agent_store.get_agent(agent_id)
    }

    /// The machine `reference` names, which may only be another kernel of the agent's owner.
    async fn files_machine(
        &self,
        agent: &crate::agent::AgentInstance,
        reference: Option<&str>,
    ) -> Result<Target, String> {
        let Some(reference) = reference.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(Target::Here);
        };
        let kernel = self
            .resolve_peer_kernel(reference)
            .await
            .map_err(super::machine_files_runtime::error_text)?;
        let Some(kernel) = kernel else {
            return Ok(Target::Here);
        };
        self.peer_owner_of_agent(agent, &kernel)
            .map_err(super::machine_files_runtime::error_text)?;
        Ok(Target::There {
            kernel,
            name: reference.to_owned(),
        })
    }

    async fn ask_machine(
        &self,
        agent: &crate::agent::AgentInstance,
        kernel: &str,
        name: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, String> {
        let owner = self
            .peer_owner_of_agent(agent, kernel)
            .map_err(super::machine_files_runtime::error_text)?;
        match self.peer_send(&owner, kernel, request, SLICE_TIMEOUT).await {
            Ok(RelayPeerResponse::FileRefused { reason }) => Err(reason),
            Ok(response) => Ok(response),
            Err(error) => Err(format!(
                "the machine `{name}` did not answer ({}); try again once it is reachable",
                super::machine_files_runtime::error_text(error)
            )),
        }
    }

    async fn files_list(
        &self,
        agent: &crate::agent::AgentInstance,
        args: ListArgs,
    ) -> Result<RuntimeToolResult, String> {
        let (entries, truncated) = match self.files_machine(agent, args.machine.as_deref()).await? {
            Target::Here => {
                let guard = self.machine_state_guard();
                let (path, offset) = (args.path.clone(), args.offset);
                tokio::task::spawn_blocking(move || {
                    crate::machine_files::list_path(&guard, &path, offset)
                })
                .await
                .map_err(|_| "the listing stopped unexpectedly".to_owned())??
            }
            Target::There { kernel, name } => match self
                .ask_machine(
                    agent,
                    &kernel,
                    &name,
                    RelayPeerRequest::ListMachineDirectory {
                        path: args.path.clone(),
                        offset: args.offset,
                    },
                )
                .await?
            {
                RelayPeerResponse::MachineDirectory { entries, truncated } => (entries, truncated),
                _ => return Err("the machine answered with something unexpected".into()),
            },
        };
        Ok(success(serde_json::json!({
            "path": args.path,
            "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
            "truncated": truncated,
        })))
    }

    async fn files_read(
        &self,
        agent: &crate::agent::AgentInstance,
        args: ReadArgs,
    ) -> Result<RuntimeToolResult, String> {
        let length = args
            .length
            .unwrap_or(READ_DEFAULT_BYTES)
            .clamp(1, READ_MAX_BYTES);
        let (bytes, size) = match self.files_machine(agent, args.machine.as_deref()).await? {
            Target::Here => {
                let (machine, path, offset) = (self.this_machine(), args.path.clone(), args.offset);
                let slice = tokio::task::spawn_blocking(move || {
                    crate::agent_file_copy::read_slice(&machine, &path, offset)
                })
                .await
                .map_err(|_| "the read stopped unexpectedly".to_owned())??;
                (slice.bytes.to_vec(), slice.size_bytes)
            }
            Target::There { kernel, name } => match self
                .ask_machine(
                    agent,
                    &kernel,
                    &name,
                    RelayPeerRequest::ReadMachineFile {
                        path: args.path.clone(),
                        offset: args.offset,
                    },
                )
                .await?
            {
                RelayPeerResponse::FileSliceRead {
                    size_bytes,
                    content,
                    ..
                } => (
                    content.decode().ok_or("a page arrived damaged")?.to_vec(),
                    size_bytes,
                ),
                _ => return Err("the machine answered with something unexpected".into()),
            },
        };
        let page = &bytes[..bytes.len().min(length as usize)];
        let eof = args.offset.saturating_add(page.len() as u64) >= size;
        let mut payload = serde_json::json!({"offset": args.offset, "size": size, "eof": eof});
        match std::str::from_utf8(page) {
            Ok(text) => payload["text"] = text.into(),
            Err(_) => {
                use base64::Engine;
                payload["base64"] = base64::engine::general_purpose::STANDARD
                    .encode(page)
                    .into();
            }
        }
        Ok(success(payload))
    }

    async fn files_copy(
        &self,
        agent: &crate::agent::AgentInstance,
        args: CopyArgs,
    ) -> Result<RuntimeToolResult, String> {
        let source = self
            .files_machine(agent, args.source.machine.as_deref())
            .await?;
        let destination = self
            .files_machine(agent, args.destination.machine.as_deref())
            .await?;
        let source_kernel = match &source {
            Target::Here => self.owned.config_projection.snapshot().daemon_id,
            Target::There { kernel, .. } => kernel.clone(),
        };
        let request = StartFileTransferRequest {
            source: FileTransferEndpoint {
                kernel: Some(source_kernel),
                path: args.source.path,
            },
            destination_folder: args.destination.folder,
            name: args.destination.name,
            conflict: args.conflict,
            delete_source: args.delete_source,
        };
        let job = match &destination {
            Target::Here => self
                .start_file_transfer(request)
                .await
                .map_err(super::machine_files_runtime::error_text)?,
            Target::There { kernel, name } => {
                match self
                    .ask_machine(
                        agent,
                        kernel,
                        name,
                        RelayPeerRequest::StartFileTransfer { request },
                    )
                    .await?
                {
                    RelayPeerResponse::FileTransfer { job } => job,
                    _ => return Err("the machine answered with something unexpected".into()),
                }
            }
        };
        let wait = args
            .wait_seconds
            .unwrap_or(WAIT_DEFAULT_SECONDS)
            .min(WAIT_MAX_SECONDS);
        let job = self.await_transfer(agent, &destination, job, wait).await?;
        Ok(transfer_result(&destination, &job))
    }

    async fn files_transfer(
        &self,
        agent: &crate::agent::AgentInstance,
        args: TransferArgs,
    ) -> Result<RuntimeToolResult, String> {
        let machine = self.files_machine(agent, args.machine.as_deref()).await?;
        let job_id = args.job_id;
        let job = match (&machine, args.cancel) {
            (Target::Here, false) => {
                self.get_file_transfer(&crate::local::FileTransferRef { job_id })
            }
            (Target::Here, true) => {
                self.cancel_file_transfer(&crate::local::FileTransferRef { job_id })
            }
            (Target::There { kernel, name }, cancel) => {
                let request = if cancel {
                    RelayPeerRequest::CancelFileTransfer { job_id }
                } else {
                    RelayPeerRequest::GetFileTransfer { job_id }
                };
                return match self.ask_machine(agent, kernel, name, request).await? {
                    RelayPeerResponse::FileTransfer { job } => {
                        let wait = args.wait_seconds.unwrap_or(0).min(WAIT_MAX_SECONDS);
                        let job = self.await_transfer(agent, &machine, job, wait).await?;
                        Ok(transfer_result(&machine, &job))
                    }
                    _ => Err("the machine answered with something unexpected".into()),
                };
            }
        }
        .map_err(super::machine_files_runtime::error_text)?;
        let wait = args.wait_seconds.unwrap_or(0).min(WAIT_MAX_SECONDS);
        let job = self.await_transfer(agent, &machine, job, wait).await?;
        Ok(transfer_result(&machine, &job))
    }

    /// The job as it stands once it ends or `wait_seconds` pass.
    async fn await_transfer(
        &self,
        agent: &crate::agent::AgentInstance,
        machine: &Target,
        mut job: FileTransferJob,
        wait_seconds: u64,
    ) -> Result<FileTransferJob, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_seconds);
        while !job.state.is_finished() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
            let job_id = job.job_id.clone();
            job = match machine {
                Target::Here => self
                    .get_file_transfer(&crate::local::FileTransferRef { job_id })
                    .map_err(super::machine_files_runtime::error_text)?,
                Target::There { kernel, name } => match self
                    .ask_machine(
                        agent,
                        kernel,
                        name,
                        RelayPeerRequest::GetFileTransfer { job_id },
                    )
                    .await?
                {
                    RelayPeerResponse::FileTransfer { job } => job,
                    _ => return Err("the machine answered with something unexpected".into()),
                },
            };
        }
        Ok(job)
    }
}

fn refusal_error(message: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: "file_transfer_refused".into(),
        message: message.into(),
    }
}

fn entry_json(entry: &KernelDirectoryEntry) -> serde_json::Value {
    serde_json::json!({
        "name": entry.name,
        "kind": entry.kind,
        "size": entry.size,
        "modified_ms": entry.modified_ms,
        "revision": entry.revision,
    })
}

/// What the model gets of a transfer: how it ended or stands, never any file content.
fn transfer_result(destination: &Target, job: &FileTransferJob) -> RuntimeToolResult {
    let mut payload = serde_json::to_value(job).expect("a job serializes");
    if let Target::There { name, .. } = destination {
        payload["destination_machine"] = name.as_str().into();
    }
    payload
        .as_object_mut()
        .expect("an object")
        .remove("started_ms");
    match job.state {
        crate::local::FileTransferState::Failed | crate::local::FileTransferState::Canceled => {
            RuntimeToolResult { ok: false, payload }
        }
        _ => RuntimeToolResult { ok: true, payload },
    }
}
