//! MP-08/MP-09/MP-10/MP-11: tool-only upload arguments; no terminal protocol change.
use super::kernel_browser_runtime::host_error;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    command: UploadCommand,
    document_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum UploadCommand {
    Input {
        tab_id: String,
        generation: u64,
        input: UploadInput,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum UploadInput {
    Upload {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        paths: Option<Vec<String>>,
    },
}

pub(super) fn params(arguments: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
    let request: Arguments = serde_json::from_value(arguments)
        .map_err(|error| host_error(super::kernel_browser_command_error::public_reason(&error)))?;
    let UploadCommand::Input {
        tab_id,
        generation,
        input,
    } = &request.command;
    let UploadInput::Upload {
        target,
        path,
        paths,
    } = input;
    let invalid = || {
        host_error(
            "upload requires an observed document, snapshot target and path or 1 through 20 paths"
                .into(),
        )
    };
    if request.document_id.is_empty()
        || request.document_id.len() > 256
        || tab_id.is_empty()
        || tab_id.len() > 256
        || *generation == 0
        || !target.strip_prefix("backend:").is_some_and(|id| {
            id.parse::<u64>().is_ok_and(|value| value > 0)
                && id.len() <= 16
                && id.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(invalid());
    }
    let bounded =
        |value: &String| !value.is_empty() && value.len() <= 4096 && !value.contains('\0');
    match (path, paths) {
        (Some(path), None) if bounded(path) => {}
        (None, Some(paths))
            if !paths.is_empty() && paths.len() <= 20 && paths.iter().all(bounded) => {}
        _ => return Err(invalid()),
    }
    let mut params = serde_json::to_value(&request.command).map_err(|_| invalid())?;
    params["document_id"] = request.document_id.into();
    params["focused_agent"] = true.into();
    Ok(params)
}
