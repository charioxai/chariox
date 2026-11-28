use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::AgentInstance;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonResponse, OwnerViewBrowser, OwnerViewRequest, OwnerViewRows, OwnerViewSnapshot,
    OwnerViewWatcher,
};
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
    browser: Option<(u64, OwnerViewBrowser)>,
    /// Counts the agents' snapshot requests; the owner tab answers the newest one.
    snapshot_wanted: u64,
    snapshot: Option<(u64, OwnerViewSnapshot)>,
}

/// A snapshot is one bounded JPEG (512 KiB once decoded).
const SNAPSHOT_MAX_BASE64: usize = 512 * 1024 * 4 / 3 + 4;
const BROWSER_MAX_PANES: usize = 32;

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
        self.browser = None;
        self.snapshot = None;
        if self.watchers.is_empty() {
            self.tab_id = None;
        }
    }

    fn status(&self) -> LocalDaemonResponse {
        LocalDaemonResponse::OwnerViewStatus {
            generation: self.generation.clone(),
            tab_id: self.tab_id.clone(),
            watchers: self.watchers.values().cloned().collect(),
            snapshot_wanted: self.snapshot_wanted,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum ViewerRequest {
    Watch {},
    Read {
        #[serde(default)]
        snapshot: bool,
    },
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
                browser,
                snapshot,
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
                if !browser.as_ref().is_none_or(browser_valid)
                    || !snapshot
                        .as_ref()
                        .is_none_or(|snapshot| snapshot_valid(snapshot, state.snapshot_wanted))
                {
                    return Err(view_error(
                        "owner_view_invalid_publication",
                        "Open the owner Web view again.",
                    ));
                }
                let at = unix_epoch_ms();
                state.presentation = Some((at, rows));
                if let Some(browser) = browser {
                    state.browser = Some((at, browser));
                }
                if let Some(snapshot) = snapshot {
                    state.snapshot = Some((at, snapshot));
                }
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
            ViewerRequest::Read { snapshot } => {
                if !state.watchers.contains_key(id) {
                    return Ok(refused(
                        "Start watching the owner Web view before reading it.",
                    ));
                }
                if snapshot {
                    // The owner tab sees the new count on its status feed and sends one picture.
                    state.snapshot_wanted += 1;
                    self.changes.record_change();
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
        let Some((at, rows)) = &state.presentation else {
            return Ok(RuntimeToolResult {
                ok: true,
                payload: json!({ "status": "waiting_for_owner_tab" }),
            });
        };
        let mut payload = json!({ "status": "watching", "published_at_ms": at, "rows": rows });
        if let Some((at, browser)) = &state.browser {
            payload["browser"] = json!(browser);
            payload["browser"]["measured_at_ms"] = json!(at);
        }
        if let ViewerRequest::Read { snapshot: true } = request {
            payload["snapshot"] = match &state.snapshot {
                Some((at, snapshot)) => json!({
                    "status": "latest",
                    "tab_id": snapshot.tab_id,
                    "source": snapshot.source,
                    "captured_at_ms": at,
                    "note": "The owner tab is sending a newer picture. Read with snapshot again for it.",
                }),
                None => json!({
                    "status": "requested",
                    "note": "The owner tab is sending a picture. Read with snapshot again in a second.",
                }),
            };
            if let Some((_, snapshot)) = &state.snapshot {
                payload["mime_type"] = json!("image/jpeg");
                payload["image_base64"] = json!(snapshot.jpeg_base64);
            }
        }
        Ok(RuntimeToolResult { ok: true, payload })
    }
}

fn browser_valid(browser: &OwnerViewBrowser) -> bool {
    let rate = |value: f64| value.is_finite() && (0.0..=1000.0).contains(&value);
    let dpr = browser.viewer.device_pixel_ratio;
    dpr.is_finite()
        && dpr > 0.0
        && dpr <= 16.0
        && browser.panes.len() <= BROWSER_MAX_PANES
        && browser.panes.iter().all(|pane| {
            short_label(&pane.tab_id)
                && short_label(&pane.mode)
                && [
                    pane.received_fps,
                    pane.painted_fps,
                    pane.region_painted_fps,
                    pane.dropped_fps,
                    pane.still_updates_fps,
                ]
                .into_iter()
                .all(rate)
                && pane.input_to_paint_ms.as_ref().is_none_or(|latency| {
                    [latency.p50, latency.max]
                        .into_iter()
                        .all(|value| value.is_finite() && (0.0..=600_000.0).contains(&value))
                })
        })
}

fn snapshot_valid(snapshot: &OwnerViewSnapshot, wanted: u64) -> bool {
    use base64::Engine as _;
    snapshot.request > 0
        && snapshot.request <= wanted
        && short_label(&snapshot.tab_id)
        && snapshot.jpeg_base64.len() <= SNAPSHOT_MAX_BASE64
        && base64::engine::general_purpose::STANDARD
            .decode(&snapshot.jpeg_base64)
            .is_ok_and(|bytes| bytes.starts_with(&[0xff, 0xd8, 0xff]))
}

fn short_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
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
                browser: None,
                snapshot: None,
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
        owner_view_carries_viewer_stats_and_one_snapshot_per_request(
            &view, &store, &owner, &tab_id,
        );
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

    fn owner_view_carries_viewer_stats_and_one_snapshot_per_request(
        view: &OwnerView,
        store: &DurableKernelStateStore,
        owner: &OwnerAccount,
        tab_id: &str,
    ) {
        use crate::local::{
            OwnerViewLatency, OwnerViewPane, OwnerViewPaneTotals, OwnerViewPath,
            OwnerViewSnapshotSource, OwnerViewViewer, OwnerViewViewerKind,
        };
        use base64::Engine as _;
        let read = |arguments: Value| {
            view.viewer_request(store, owner, "agent", "agent", arguments)
                .unwrap()
        };
        let status = || match view
            .terminal_request(store, owner, OwnerViewRequest::Status {})
            .unwrap()
        {
            LocalDaemonResponse::OwnerViewStatus {
                generation,
                snapshot_wanted,
                ..
            } => (generation, snapshot_wanted),
            _ => panic!("expected status"),
        };
        let browser = OwnerViewBrowser {
            viewer: OwnerViewViewer {
                kind: OwnerViewViewerKind::Headless,
                device_pixel_ratio: 2.0,
            },
            path: OwnerViewPath::Relay,
            window_ms: 1000,
            panes: vec![OwnerViewPane {
                tab_id: "tab-1".into(),
                mode: "video".into(),
                received_fps: 59.5,
                painted_fps: 58.0,
                region_painted_fps: 0.0,
                dropped_fps: 1.5,
                still_updates_fps: 0.0,
                input_to_paint_ms: Some(OwnerViewLatency {
                    p50: 31.0,
                    max: 48.0,
                    count: 3,
                }),
                totals: OwnerViewPaneTotals {
                    received: 600,
                    painted: 580,
                    dropped: 20,
                    still_updates: 0,
                },
            }],
        };
        let jpeg = base64::engine::general_purpose::STANDARD.encode([0xff, 0xd8, 0xff, 0xe0, 1, 2]);
        let snapshot = |request, jpeg_base64: &str| OwnerViewSnapshot {
            request,
            tab_id: "tab-1".into(),
            source: OwnerViewSnapshotSource::ViewerCanvas,
            jpeg_base64: jpeg_base64.into(),
        };
        let publish = |browser: Option<OwnerViewBrowser>, snapshot: Option<OwnerViewSnapshot>| {
            view.terminal_request(
                store,
                owner,
                OwnerViewRequest::Publish {
                    generation: status().0,
                    tab_id: tab_id.into(),
                    rows: OwnerViewRows {
                        navigator: vec!["Room".into()],
                        settings: vec![],
                    },
                    browser,
                    snapshot,
                },
            )
        };

        publish(Some(browser.clone()), None).unwrap();
        let payload = read(json!({"operation": "read"})).payload;
        assert_eq!(payload["browser"]["viewer"]["kind"], "headless");
        assert_eq!(payload["browser"]["path"], "relay");
        assert_eq!(payload["browser"]["panes"][0]["painted_fps"], 58.0);
        assert!(payload["browser"]["measured_at_ms"].is_u64());
        assert!(payload.get("snapshot").is_none() && payload.get("image_base64").is_none());

        // A picture only answers a request, and only a JPEG of bounded size.
        assert_eq!(status().1, 0);
        assert!(publish(None, Some(snapshot(1, &jpeg))).is_err());
        let asked = read(json!({"operation": "read", "snapshot": true})).payload;
        assert_eq!(asked["snapshot"]["status"], "requested");
        assert!(asked.get("image_base64").is_none());
        assert_eq!(status().1, 1);
        let png = base64::engine::general_purpose::STANDARD.encode([0x89, b'P', b'N', b'G']);
        assert!(publish(None, Some(snapshot(1, &png))).is_err());
        let huge = "A".repeat(SNAPSHOT_MAX_BASE64 + 4);
        assert!(publish(None, Some(snapshot(1, &huge))).is_err());
        let mut bad = browser.clone();
        bad.panes[0].painted_fps = f64::NAN;
        assert!(publish(Some(bad), None).is_err());
        publish(None, Some(snapshot(1, &jpeg))).unwrap();
        let shown = read(json!({"operation": "read", "snapshot": true})).payload;
        assert_eq!(shown["snapshot"]["status"], "latest");
        assert_eq!(shown["snapshot"]["source"], "viewer_canvas");
        assert_eq!(shown["mime_type"], "image/jpeg");
        assert_eq!(shown["image_base64"], jpeg.as_str());
        // A publication without stats keeps the last stats.
        assert_eq!(shown["browser"]["panes"][0]["tab_id"], "tab-1");

        // The read vocabulary stays observation only.
        for frame in [
            json!({"operation": "read", "snapshot": "yes"}),
            json!({"operation": "read", "snapshot": true, "click": [10, 10]}),
            json!({"operation": "type", "text": "hello"}),
            json!({"operation": "navigate", "url": "https://example.com"}),
            json!({"operation": "key", "key": "Enter"}),
        ] {
            assert!(!read(frame).ok);
        }
    }
}
