//! Trusted snapshot fields carry protocol identities; page strings carry observations.
use super::*;
use crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot;

impl RoomSecretObservations {
    pub(in crate::runtime::state) fn scrub_snapshot(
        &self,
        room: &str,
        mut snapshot: BrowserControllerStructuredSnapshot,
    ) -> Result<BrowserControllerStructuredSnapshot, DaemonError> {
        // One admission and value lock for the entire snapshot. Attribute maps
        // are page-owned, so their keys receive the ordinary scrub as well.
        let accessibility: Vec<_> = snapshot
            .accessibility_nodes
            .iter()
            .map(|node| {
                (
                    node.name.clone(),
                    node.description.clone(),
                    node.value.clone(),
                )
            })
            .collect();
        let dom: Vec<_> = snapshot
            .dom_nodes
            .iter()
            .map(|node| {
                (
                    node.node_name.clone(),
                    node.text.clone(),
                    node.attributes.clone(),
                )
            })
            .collect();
        let urls: Vec<_> = snapshot
            .dom_documents
            .iter()
            .map(|document| document.url.clone())
            .collect();
        let (accessibility, dom, urls) = self.scrub(room, (accessibility, dom, urls))?;
        for (node, (name, description, value)) in
            snapshot.accessibility_nodes.iter_mut().zip(accessibility)
        {
            node.name = name;
            node.description = description;
            node.value = value;
        }
        for (node, (node_name, text, attributes)) in snapshot.dom_nodes.iter_mut().zip(dom) {
            node.node_name = node_name;
            node.text = text;
            node.attributes = attributes;
        }
        for (document, url) in snapshot.dom_documents.iter_mut().zip(urls) {
            document.url = url;
        }
        Ok(snapshot)
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct KernelSnapshotResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
    snapshot: BrowserControllerStructuredSnapshot,
}

impl RoomSecretObservations {
    pub(in crate::runtime::state) fn scrub_kernel_snapshot_result(
        &self,
        room: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let mut result: KernelSnapshotResult =
            serde_json::from_value(input).map_err(|_| DaemonError::LocalTransport {
                operation: "browser.snapshot",
                message:
                    "The browser returned an invalid snapshot. Refresh the browser and try again."
                        .into(),
            })?;
        result.snapshot = self.scrub_snapshot(room, result.snapshot)?;
        serde_json::to_value(result).map_err(|_| protection_error())
    }
}
