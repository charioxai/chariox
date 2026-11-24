use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::AgentInstance;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{LocalDaemonResponse, OwnerViewRequest, OwnerViewRows, OwnerViewWatcher};
use crate::runtime::command::KernelCommand;
use crate::session::{unix_epoch_ms, RuntimeSession, DEFAULT_LOCAL_USER_ID};
use crate::transport::runtime_tools::RuntimeToolResult;

use super::{KernelRuntimeState, RuntimeChangeSignal};

#[derive(Default)]
pub(super) struct OwnerView {
    state: Mutex<State>,
    pub(crate) changes: RuntimeChangeSignal,
}

#[derive(Default)]
struct State {
    owner: OwnerAccount,
    watchers: BTreeMap<String, OwnerViewWatcher>,
    generation: String,
    tab_id: Option<String>,
    presentation: Option<(u64, OwnerViewRows)>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct OwnerAccount {
    realm_id: String,
    user_id: String,
}

impl State {
    fn for_owner(&mut self, owner: &OwnerAccount) -> &mut Self {
        if &self.owner != owner {
            *self = Self {
                owner: owner.clone(),
                ..Self::default()
            };
        }
        self
    }

    fn watchers_changed(&mut self) {
        // Require a new presentation after the browser has rendered the current
        // watching indicator. Old publications cannot cross a membership change.
        self.generation = format!("{:032x}", rand::random::<u128>());
        self.presentation = None;
        if self.watchers.is_empty() {
            self.tab_id = None;
        }
    }

    fn status(&self) -> LocalDaemonResponse {
        LocalDaemonResponse::OwnerViewStatus {
            generation: self.generation.clone(),
            tab_id: self.tab_id.clone(),
            watchers: self.watchers.values().cloned().collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum ViewerRequest {
    Watch {},
    Read {},
    Leave {},
}

impl KernelRuntimeState {
    pub(crate) fn owner_view_changes(&self) -> &RuntimeChangeSignal {
        &self.owned.owner_view.changes
    }

    pub(crate) fn owner_view_request(
        &self,
        command: &KernelCommand,
        request: OwnerViewRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        if !command.is_terminal_caller()
            || !crate::runtime::cloud_relay_authorization::cloud_relay_caller_owner(
                &config,
                &command.caller,
            )
        {
            return Err(owner_required());
        }
        let owner = owner_account(&config).ok_or_else(owner_required)?;
        self.owned
            .owner_view
            .terminal_request(&self.owned.durable_state_store, &owner, request)
    }

    pub(super) fn owner_view_runtime_tool(
        &self,
        session: &RuntimeSession,
        agent: &AgentInstance,
        arguments: Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let Some(owner) = owner_account(&self.owned.config_projection.snapshot()) else {
            return Ok(refused("This kernel has no hosted owner Web view."));
        };
        let owned_by = |id: &str| id == owner.user_id || id == DEFAULT_LOCAL_USER_ID;
        if agent.archived()
            || !owned_by(session.owner_user_id())
            || !owned_by(agent.owner_user_id())
        {
            return Ok(refused("Only the owner's agents may watch this Web view."));
        }
        self.owned.owner_view.viewer_request(
            &self.owned.durable_state_store,
            &owner,
            agent.id(),
            agent.alias().unwrap_or(agent.id()),
            arguments,
        )
    }
}

fn owner_account(config: &crate::config::DaemonConfig) -> Option<OwnerAccount> {
    let profile = crate::runtime::cloud_relay_control::active_cloud_profile(config)
        .filter(|profile| !profile.user_id.is_empty())?;
    Some(OwnerAccount {
        realm_id: profile.realm_id.clone(),
        user_id: profile.user_id.clone(),
    })
}

impl OwnerView {
    pub(super) fn detach(&self, agent_id: &str) {
        let mut state = self.state.lock().expect("owner view lock poisoned");
        if state.watchers.remove(agent_id).is_some() {
            state.watchers_changed();
            self.changes.record_change();
        }
    }

    fn terminal_request(
        &self,
        store: &DurableKernelStateStore,
        owner: &OwnerAccount,
        request: OwnerViewRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut guard = self.state.lock().expect("owner view lock poisoned");
        let state = guard.for_owner(owner);
        match request {
            OwnerViewRequest::Status {} => {}
            OwnerViewRequest::Publish {
                generation,
                tab_id,
                rows,
            } => {
                if state.watchers.is_empty() || state.generation != generation {
                    return Err(view_error("owner_view_changed", "The watching agents changed. Read their current status before sharing again."));
                }
                if tab_id.len() != 36
                    || !tab_id
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
                {
                    return Err(view_error(
                        "owner_view_invalid_tab",
                        "Open the owner Web view again.",
                    ));
                }
                match state.tab_id.as_ref() {
                    Some(tab) if tab != &tab_id => {
                        return Err(view_error(
                            "owner_view_other_tab",
                            "This view is shared from another owner tab.",
                        ));
                    }
                    None => {
                        state.tab_id = Some(tab_id);
                        self.changes.record_change();
                    }
                    Some(_) => {}
                }
                state.presentation = Some((unix_epoch_ms(), rows));
            }
            OwnerViewRequest::End { agent_id } => {
                store.revoke_owner_view(&owner.realm_id, &owner.user_id, &agent_id)?;
                if state.watchers.remove(&agent_id).is_some() {
                    state.watchers_changed();
                    self.changes.record_change();
                }
            }
        }
        Ok(state.status())
    }

    fn viewer_request(
        &self,
        store: &DurableKernelStateStore,
        owner: &OwnerAccount,
        id: &str,
        name: &str,
        arguments: Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let Ok(request) = serde_json::from_value::<ViewerRequest>(arguments) else {
            crate::logging::warn_with_fields(
                "daemon.owner_view",
                "rejected unsupported owner view operation",
                json!({ "agent_id": id }),
            );
            return Ok(refused(
                "The owner Web view only supports watch, read and leave.",
            ));
        };
        let mut guard = self.state.lock().expect("owner view lock poisoned");
        let state = guard.for_owner(owner);
        if !matches!(request, ViewerRequest::Leave {})
            && !state.watchers.contains_key(id)
            && store.owner_view_revoked(&owner.realm_id, &owner.user_id, id)?
        {
            return Ok(refused("The owner ended your Web view access."));
        }
        match request {
            ViewerRequest::Watch {} => {
                if !state.watchers.contains_key(id) {
                    state.watchers.insert(
                        id.to_owned(),
                        OwnerViewWatcher {
                            agent_id: id.to_owned(),
                            name: name.to_owned(),
                        },
                    );
                    state.watchers_changed();
                    self.changes.record_change();
                }
            }
            ViewerRequest::Read {} => {
                if !state.watchers.contains_key(id) {
                    return Ok(refused(
                        "Start watching the owner Web view before reading it.",
                    ));
                }
            }
            ViewerRequest::Leave {} => {
                if state.watchers.remove(id).is_some() {
                    state.watchers_changed();
                    self.changes.record_change();
                }
                return Ok(RuntimeToolResult {
                    ok: true,
                    payload: json!({ "status": "detached" }),
                });
            }
        }
        let payload = match &state.presentation {
            Some((at, rows)) => {
                json!({ "status": "watching", "published_at_ms": at, "rows": rows })
            }
            None => json!({ "status": "waiting_for_owner_tab" }),
        };
        Ok(RuntimeToolResult { ok: true, payload })
    }
}

fn refused(message: &str) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: json!({ "error": message }),
    }
}

fn owner_required() -> DaemonError {
    view_error(
        "owner_view_owner_required",
        "Sign in to the owner Web terminal to manage its presentation.",
    )
}

fn view_error(code: &str, message: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_view_requires_presentation_for_the_current_watchers_and_resumes_the_same_tab() {
        let view = OwnerView::default();
        let temp = crate::test_temp_dir::TestTempDir::new("owner-view");
        let store = DurableKernelStateStore::open(temp.join("kernel.db")).unwrap();
        let owner = OwnerAccount {
            realm_id: "hosted".into(),
            user_id: "owner".into(),
        };
        let send = |agent, operation| {
            view.viewer_request(
                &store,
                &owner,
                agent,
                agent,
                json!({ "operation": operation }),
            )
            .unwrap()
        };
        let terminal = |request| view.terminal_request(&store, &owner, request);
        assert!(!send("agent", "read").ok);
        assert_eq!(
            send("agent", "watch").payload["status"],
            "waiting_for_owner_tab"
        );

        let LocalDaemonResponse::OwnerViewStatus {
            generation,
            watchers,
            ..
        } = terminal(OwnerViewRequest::Status {}).unwrap()
        else {
            panic!("expected status")
        };
        assert_eq!(watchers[0].name, "agent");

        let tab_id = "e400ab77-217b-4509-b64b-c1cf40551a90".to_owned();
        let publish = |tab_id, room: &str| {
            terminal(OwnerViewRequest::Publish {
                generation: generation.clone(),
                tab_id,
                rows: OwnerViewRows {
                    navigator: vec![room.into()],
                    settings: vec!["Appearance".into()],
                },
            })
        };
        publish(tab_id.clone(), "Room").unwrap();
        assert_eq!(
            send("agent", "read").payload["rows"]["navigator"][0],
            "Room"
        );
        assert!(publish("e400ab77-217b-4509-b64b-c1cf40551a91".into(), "Other tab").is_err());
        publish(tab_id.clone(), "Renamed room").unwrap();
        assert_eq!(
            send("agent", "read").payload["rows"]["navigator"][0],
            "Renamed room"
        );

        for frame in [
            json!({"operation": "click", "target": "Room"}),
            json!({"operation": "read", "keys": "text"}),
        ] {
            assert!(
                !view
                    .viewer_request(&store, &owner, "agent", "agent", frame)
                    .unwrap()
                    .ok
            );
        }
        terminal(OwnerViewRequest::End {
            agent_id: "agent".into(),
        })
        .unwrap();
        assert!(publish(tab_id, "No watchers").is_err());
        assert!(!send("agent", "read").ok);
        assert!(!send("agent", "watch").ok);
        let reopened = DurableKernelStateStore::open(temp.join("kernel.db")).unwrap();
        assert!(
            !OwnerView::default()
                .viewer_request(
                    &reopened,
                    &owner,
                    "agent",
                    "agent",
                    json!({"operation": "watch"})
                )
                .unwrap()
                .ok
        );
    }
}
