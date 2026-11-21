//! Operations scoped to the allocation this agent acquired.
use super::*;
use crate::runtime::kernel_gateway::Origin;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use serde::Deserialize;
use serde_json::{json, Value};

pub(in crate::runtime::state) const EXEC: &str = "chariox.machines.exec";
pub(in crate::runtime::state) const COPY: &str = "chariox.machines.copy";

pub(super) fn specs() -> [RuntimeToolSpec; 2] {
    [
        RuntimeToolSpec {
            name: EXEC.into(),
            description: "Run a short command on a ready machine you acquired. The command runs directly on the acquired machine without uploading your Room workspace. Use the allocation_ref from acquire and optional absolute directory on that machine.".into(),
            input_schema: json!({
                "type":"object","additionalProperties":false,"required":["allocation_ref","command"],
                "properties":{"allocation_ref":{"type":"string"},"command":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},"directory":{"type":"string"}},
            }),
        },
        RuntimeToolSpec {
            name: COPY.into(),
            description: "Copy a file between this kernel and a ready machine you acquired, with a verified checksum. Upload reads path here and writes into folder there. Download reads path there and writes into folder here. Use the allocation_ref from acquire.".into(),
            input_schema: json!({
                "type":"object","additionalProperties":false,"required":["allocation_ref","direction","path","folder"],
                "properties":{"allocation_ref":{"type":"string"},"direction":{"enum":["upload","download"]},"path":{"type":"string"},"folder":{"type":"string"},"name":{"type":"string"}},
            }),
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecArgs {
    allocation_ref: String,
    command: Vec<String>,
    directory: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::runtime::state) enum CopyDirection {
    Upload,
    Download,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime::state) struct AllocationCopy {
    allocation_ref: String,
    pub(in crate::runtime::state) direction: CopyDirection,
    pub(in crate::runtime::state) path: String,
    pub(in crate::runtime::state) folder: String,
    pub(in crate::runtime::state) name: Option<String>,
}

enum Operation {
    Exec(ExecArgs),
    Copy(AllocationCopy),
}

impl Operation {
    fn allocation_ref(&self) -> &str {
        match self {
            Self::Exec(args) => &args.allocation_ref,
            Self::Copy(args) => &args.allocation_ref,
        }
    }
}

impl KernelRuntimeState {
    pub(super) async fn allocation_access_tool(
        &self,
        origin: &Origin,
        agent: &str,
        tool: &str,
        args: Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let error = |error: serde_json::Error| {
            acquisition_error(
                "invalid_request",
                &format!("Check the arguments for {tool}: {error}"),
            )
        };
        let operation = match tool {
            EXEC => Operation::Exec(serde_json::from_value(args).map_err(error)?),
            COPY => Operation::Copy(serde_json::from_value(args).map_err(error)?),
            _ => {
                return Err(acquisition_error(
                    "invalid_request",
                    "Choose machines.exec or machines.copy",
                ))
            }
        };
        if let Operation::Exec(args) = &operation {
            if args
                .command
                .first()
                .is_none_or(|program| program.trim().is_empty())
                || args.command.len() > 64
                || args.command.iter().any(|arg| arg.contains('\0'))
                || args
                    .directory
                    .as_deref()
                    .is_some_and(|dir| !std::path::Path::new(dir).is_absolute())
            {
                return Err(acquisition_error(
                    "invalid_request",
                    "Give the executable and arguments as a nonempty command array and use an absolute directory",
                ));
            }
        }
        let allocation_ref = operation.allocation_ref();
        if !self
            .owned
            .durable_state_store
            .allocation_belongs_to_agent(allocation_ref, agent)?
        {
            return Err(acquisition_error(
                "machine_not_held",
                "Use the allocation_ref from a machine you acquired. This allocation is unavailable to your agent",
            ));
        }
        let command_id = format!("allocation-{:032x}", rand::random::<u128>());
        let machine = self
            .job_held_machine(allocation_ref, origin, &command_id)
            .await?;
        if !machine.target_verified {
            return Err(acquisition_error(
                "machine_unreachable",
                "The allocation's enrolled kernel could not be verified. Check machines.status and retry once access is available",
            ));
        }
        match operation {
            Operation::Exec(args) => {
                let payload = super::super::jobs::held_exec::run(
                    machine.grant,
                    &command_id,
                    args.command,
                    args.directory.as_deref(),
                )
                .await?;
                Ok(RuntimeToolResult { ok: true, payload })
            }
            Operation::Copy(args) => {
                self.copy_acquired_file(agent, machine.kernel_id, args)
                    .await
            }
        }
    }
}
