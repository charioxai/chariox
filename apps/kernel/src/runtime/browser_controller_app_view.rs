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
