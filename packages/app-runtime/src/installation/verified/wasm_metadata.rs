//! Preserve v1 digest compatibility while binding v2 UI effects into the same
//! installation approval and catalog authority, never a parallel approval.
use super::{json_digest, Result};
use chariox_app_package::{ReleaseManifest, VerifiedApp};
use serde_json::json;

pub(super) fn digests(
    package: &impl VerifiedApp,
    capabilities: String,
    catalog: String,
) -> Result<(String, String)> {
    if matches!(package.release_manifest(), ReleaseManifest::Node(_)) {
        return Ok((capabilities, catalog));
    }
    let actions = package.ui_actions();
    Ok((
        json_digest(
            &json!({"schema":"chariox.installation-capabilities.wasm.v1", "backend":capabilities,"uiActions":actions}),
        )?,
        json_digest(
            &json!({"schema":"chariox.installation-catalog.wasm.v1","backend":catalog,"uiActions":actions}),
        )?,
    ))
}
