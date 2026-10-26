//! Private supervisor/worker launch ABI. No client-supplied paths or code.
use crate::release_store::aot_key::AotCacheKey;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmLaunch {
    pub generation: String,
    pub installation_id: String,
    pub release_digest: String,
    pub manifest: chariox_app_package::v2::ManifestV2,
    pub cache_key: AotCacheKey,
}
