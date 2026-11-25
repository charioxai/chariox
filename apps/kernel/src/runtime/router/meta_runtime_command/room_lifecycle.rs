//! `room create` (also `session create|new`) and `room delete`: an agent makes Rooms of its own
//! and removes only those. Who may do either is the kernel's call, in `room_agent_admission`.
use super::result::meta_command_error;
use super::*;
use crate::session::{CreateSessionRequest, SessionAgentDefaults, SessionProjectSelection};

const CREATE_USAGE: &str = "usage: room create [--name <name>] [--project <project-id>] [--provider <provider> [--model <model>] [--effort <effort>] [--account-profile <profile>]]";
const DELETE_USAGE: &str = "usage: room delete <room-id>";

/// The request for a Room command; `home` is the Room the caller runs in, whose directory the new Room shares.
pub(super) fn meta_room_request(
    home: &crate::session::RuntimeSession,
    tokens: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    match tokens {
        [first, second, args @ ..]
            if matches!(
                (first.as_str(), second.as_str()),
                ("room", "create") | ("session", "create" | "new")
            ) =>
        {
            create_request(home, args)
        }
        [first, second, room] if first == "room" && second == "delete" => {
            Ok(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: room.clone(),
                workspace_id: None,
            }))
        }
        [first, second, ..] if first == "room" && second == "delete" => {
            Err(meta_command_error(DELETE_USAGE))
        }
        _ => Err(meta_command_error(CREATE_USAGE)),
    }
}

fn create_request(
    home: &crate::session::RuntimeSession,
    args: &[String],
) -> Result<LocalDaemonRequest, DaemonError> {
    let mut request = CreateSessionRequest::new(home.workspace_id(), home.worktree_id());
    let (mut provider, mut model, mut effort, mut account_profile) = (None, None, None, None);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| meta_command_error(format!("`{flag}` needs a value; {CREATE_USAGE}")))?
            .clone();
        match flag.as_str() {
            "--name" => request.alias = Some(value),
            "--project" => {
                request.project_selection = SessionProjectSelection::Existing { project_id: value }
            }
            "--provider" => provider = Some(value),
            "--model" => model = Some(value),
            "--effort" => effort = Some(value),
            "--account-profile" => account_profile = Some(value),
            other => {
                return Err(meta_command_error(format!(
                    "unknown option `{other}`; {CREATE_USAGE}"
                )))
            }
        }
    }
    match provider {
        Some(provider) => {
            request.agent_defaults = Some(SessionAgentDefaults {
                provider,
                model,
                effort,
                account_profile,
                execution_mode: None,
                permission_level: None,
            });
        }
        None if model.is_some() || effort.is_some() || account_profile.is_some() => {
            return Err(meta_command_error(format!(
                "`--model`, `--effort` and `--account-profile` go with `--provider`; {CREATE_USAGE}"
            )))
        }
        None => {}
    }
    Ok(LocalDaemonRequest::CreateSession(request))
}
