//! `room list` and `slice list`: what `--room` and `--slice` accept for the calling agent.
use super::*;

impl CommandRouter {
    /// The read-only lists of the room surface; `None` for any other command. Outside the room
    /// surface `slice list` stays the Meta-mode command.
    pub(super) fn reach_list_result(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        tokens: &[String],
    ) -> Option<RuntimeToolResult> {
        if !self.runtime_state.room_agent_tools_enabled() {
            return None;
        }
        let response = match tokens {
            [first, second] if first == "room" && second == "list" => {
                serde_json::json!({"rooms": self.runtime_state.reachable_rooms(session, caller)})
            }
            [first, second, room]
                if first == "room" && matches!(second.as_str(), "agents" | "cards") =>
            {
                match self
                    .runtime_state
                    .owner_room_state(caller.id(), room, second == "cards")
                {
                    Ok(response) => response,
                    Err(error) => {
                        return Some(super::result::meta_command_failure_result(command, error))
                    }
                }
            }
            [first, second] if first == "slice" && matches!(second.as_str(), "list" | "ls") => {
                let slices = self
                    .runtime_state
                    .slices_usable_by_room(session.id(), caller.owner_user_id())
                    .into_iter()
                    .map(|slice| {
                        serde_json::json!({
                            "id": slice.id,
                            "name": slice.name,
                            "status": slice.status,
                            "backend": slice.backend,
                            "os": slice.os,
                            "display_mode": slice.display_mode,
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({"slices": slices})
            }
            _ => return None,
        };
        Some(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"command": command, "response": response}),
        })
    }
}
