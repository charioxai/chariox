//! App view operations on the slice browser controller. The home kernel picks
//! the installation and its verified UI files; the controller only serves them
//! on the App origin and relays `window.chariox.call` requests back.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserAppViewAsset {
    pub(crate) path: String,
    pub(crate) content_type: String,
    pub(crate) body_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserAppViewError {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BrowserAppViewRequest {
    Open {
        origin_label: String,
        installation_id: String,
        entry: String,
        assets: Vec<BrowserAppViewAsset>,
    },
    Calls,
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
}

impl BrowserAppViewRequest {
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Open { .. } => "browser.app.open",
            Self::Calls => "browser.app.calls",
            Self::Reload { .. } => "browser.app.reload",
            Self::Respond { .. } => "browser.app.respond",
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewOpened {
    pub(crate) target_id: String,
    pub(crate) origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewCall {
    pub(crate) installation_id: String,
    pub(crate) target_id: String,
    pub(crate) call_id: String,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewCalls {
    pub(crate) calls: Vec<BrowserAppViewCall>,
    /// App targets the controller still serves; any other binding is closed.
    /// Absent (an older controller) means "unknown": nothing is pruned.
    #[serde(default)]
    pub(crate) open_targets: Option<Vec<String>>,
    /// Reserved conversation panels in page CSS pixels. Absent from an older
    /// controller, which neither reports panels nor marks App Tabs.
    #[serde(default)]
    pub(crate) panels: Option<Vec<BrowserAppViewPanel>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct BrowserAppViewPanel {
    pub(crate) target_id: String,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
