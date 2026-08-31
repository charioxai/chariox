use serde::{Deserialize, Serialize};

use crate::runtime::browser_controller_process::{
    BrowserControllerProcessSnapshot, BrowserControllerReconciliation,
};
use crate::session::CanonicalViewport;

/// Physical controller operations only. The home retains Room/tab authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerCommand {
    Acquire,
    Reconcile {
        viewport: CanonicalViewport,
    },
    Snapshot {
        target_id: String,
        document_id: String,
    },
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RoomBrowserControllerResult {
    Snapshot {
        snapshot: Option<
            crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot,
        >,
    },
    Process {
        snapshot: Option<BrowserControllerProcessSnapshot>,
    },
    Reconciled {
        reconciliation: Option<BrowserControllerReconciliation>,
    },
}
