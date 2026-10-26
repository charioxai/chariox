//! The identity of a locally derived AOT cache entry
//! (docs/APPS_V2_CONTRACTS.md §6). The kernel compiles trusted component bytes itself and never loads an
//! uploaded `.cwasm`; this key says when an entry may be deserialized again.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Everything native code depends on. Any change makes old entries unusable and
/// forces a recompile from the retained verified component, offline if needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AotCacheKey {
    pub component_digest: String,
    pub wasmtime_version: String,
    pub worker_build: String,
    pub engine_config_digest: String,
    pub target_triple: String,
    pub cpu_feature_policy: String,
}

impl AotCacheKey {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json_canonicalizer::to_vec(self).expect("a struct of strings serializes")
    }

    /// The cache entry's name.
    pub fn id(&self) -> String {
        format!("sha256:{:x}", Sha256::digest(self.canonical_bytes()))
    }
}

/// Recorded beside an entry. The source release is provenance only: identical
/// components share an entry across releases, and an entry conveys no
/// publisher or review trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AotProvenance {
    pub key: AotCacheKey,
    pub source_release_digest: String,
    /// Over the serialized artifact, checked before it is deserialized.
    pub artifact_digest: String,
}

impl AotProvenance {
    /// The entry may be used only by the exact build that would produce it.
    pub fn usable_by(&self, running: &AotCacheKey) -> bool {
        &self.key == running
    }
}
