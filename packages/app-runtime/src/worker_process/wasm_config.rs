//! Private supervisor/worker launch ABI. No client-supplied paths or code.
use crate::release_store::aot_key::AotCacheKey;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmMigration {
    pub from: u32,
    pub to: u32,
    pub bytes: usize,
    pub digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmLaunch {
    pub generation: String,
    pub installation_id: String,
    pub release_digest: String,
    pub manifest: chariox_app_package::v2::ManifestV2,
    /// Supervisor-owned installation directory; R6 opens existing data/tmp only.
    #[serde(default)]
    pub installation_root: Option<String>,
    pub cache_key: AotCacheKey,
    pub cache_public_key: [u8; 32],
    /// Concatenated portable migration components arrive on supervisor FD9.
    #[serde(default)]
    pub migrations: Vec<WasmMigration>,
}
