//! App view operations on the slice browser controller. The home kernel picks
//! the installation and its verified UI files; the controller only serves them
//! on the App origin and relays `window.chariox.call` requests back.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) use crate::local::{
    AppFrontendAsset as BrowserAppViewAsset, AppViewChannelError as BrowserAppViewError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BrowserAppViewRequest {
    Open {
        origin_label: String,
        installation_id: String,
        /// User-domain views are distinct ephemeral targets; Room opens omit it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_id: Option<String>,
        entry: String,
        assets: Vec<BrowserAppViewAsset>,
        /// The page's CSS size beside its agent panel; absent: the default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page: Option<AppViewPage>,
    },
    /// The Tab's page size changed (its panel moved, was minimized or hidden).
    Layout {
        target_id: String,
        page: AppViewPage,
    },
    Calls,
    /// End only the exact kernel-bound App target.
    Close {
        target_id: String,
    },
    /// Serve the current generation's assets to an open view and reload it.
    Reload {
        target_id: String,
        entry: String,
        assets: Vec<BrowserAppViewAsset>,
    },
    Respond {
        target_id: String,
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<BrowserAppViewError>,
    },
    /// Apps v2 A2: one call into the page's agent bridge, run as `agent`.
    /// The answer is a `ViewAgentAnswer`.
    Agent {
        target_id: String,
        agent: String,
        call: ViewAgentCallKind,
        /// `invoke` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// `invoke` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        /// `changes` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<u64>,
        /// `changes` only: how long to wait for the first change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait_ms: Option<u64>,
        deadline_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewAgentCallKind {
    Registrations,
    Invoke,
    State,
    Changes,
}

/// What the page's agent bridge answered: a value, or an error with a code the
/// App (or the bridge) chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ViewAgentAnswer {
    Value(Value),
    Error(BrowserAppViewError),
}

impl ViewAgentAnswer {
    pub(crate) fn from_controller(reply: Value) -> Option<Self> {
        match reply.get("ok")?.as_bool()? {
            true => Some(Self::Value(reply.get("value")?.clone())),
            false => Some(Self::Error(BrowserAppViewError {
                code: reply.get("code")?.as_str()?.to_owned(),
                message: reply.get("message")?.as_str()?.to_owned(),
            })),
        }
    }
}

impl BrowserAppViewRequest {
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Open { .. } => "browser.app.open",
            Self::Calls => "browser.app.calls",
            Self::Close { .. } => "browser.app.close",
            Self::Reload { .. } => "browser.app.reload",
            Self::Layout { .. } => "browser.app.layout",
            Self::Respond { .. } => "browser.app.respond",
            Self::Agent { .. } => "browser.app.agent",
        }
    }

    /// Controller params: the request without its `op` tag.
    pub(crate) fn params(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.remove("op");
        }
        value
    }
}

/// An App page's CSS size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppViewPage {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewOpened {
    pub(crate) target_id: String,
    pub(crate) origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewCall {
    pub(crate) installation_id: String,
    pub(crate) target_id: String,
    /// The Tab's top-level document (CDP loader) that made the call. Absent
    /// from an older controller: the call then ends only with its Tab.
    #[serde(default)]
    pub(crate) document_id: Option<String>,
    pub(crate) call_id: String,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Value,
    /// MP-08/MP-11: the agent whose input drove a user-domain App Tab's call;
    /// absent when the owner drives it. Room views keep the Room rule.
    #[serde(default)]
    pub(crate) actor: Option<BrowserAppViewActor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewActor {
    pub(crate) kind: String,
    pub(crate) id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewCalls {
    pub(crate) calls: Vec<BrowserAppViewCall>,
    /// App targets the controller still serves; any other binding is closed.
    /// Absent (an older controller) means "unknown": nothing is pruned.
    #[serde(default)]
    pub(crate) open_targets: Option<Vec<String>>,
    /// Each open App target's current document; a call made by another
    /// document is cancelled. Absent from an older controller.
    #[serde(default)]
    pub(crate) documents: Option<std::collections::HashMap<String, String>>,
    /// The controller lays App pages out beside the trusted conversation
    /// panel. An older controller does not, so its App Tabs get no panel.
    #[serde(default)]
    pub(crate) app_panels: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_call_names_only_the_tab_the_agent_and_the_call() {
        let request = BrowserAppViewRequest::Agent {
            target_id: "target-1".into(),
            agent: "agent-7".into(),
            call: ViewAgentCallKind::Invoke,
            name: Some("add_todo".into()),
            input: Some(serde_json::json!({"text": "milk"})),
            since: None,
            wait_ms: None,
            deadline_ms: 20_000,
        };
        assert_eq!(request.method(), "browser.app.agent");
        assert_eq!(
            request.params(),
            serde_json::json!({"target_id": "target-1", "agent": "agent-7", "call": "invoke",
                "name": "add_todo", "input": {"text": "milk"}, "deadline_ms": 20000})
        );
        let wire = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<BrowserAppViewRequest>(&wire).unwrap(),
            request
        );
    }

    #[test]
    fn a_bridge_answer_is_a_value_or_a_coded_error_and_nothing_else() {
        assert_eq!(
            ViewAgentAnswer::from_controller(
                serde_json::json!({"ok": true, "value": {"count": 1}})
            ),
            Some(ViewAgentAnswer::Value(serde_json::json!({"count": 1})))
        );
        assert_eq!(
            ViewAgentAnswer::from_controller(
                serde_json::json!({"ok": false, "code": "APP_REFUSED", "message": "no"})
            ),
            Some(ViewAgentAnswer::Error(BrowserAppViewError {
                code: "APP_REFUSED".into(),
                message: "no".into()
            }))
        );
        for malformed in [
            serde_json::json!({"value": 1}),
            serde_json::json!({"ok": "yes"}),
            serde_json::json!({"ok": true}),
            serde_json::json!({"ok": false, "code": "x"}),
        ] {
            assert_eq!(ViewAgentAnswer::from_controller(malformed), None);
        }
    }
}
