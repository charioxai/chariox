use serde::{Deserialize, Serialize};

pub const KERNEL_RESOURCE_TELEMETRY_SCHEMA: &str = "chariox.kernel.resource_telemetry.v3";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetKernelResourceTelemetryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelResourceTelemetrySnapshot {
    pub schema: String,
    pub captured_at: String,
    pub captured_at_monotonic_ms: u64,
    pub telemetry: KernelResourceTelemetryMetadata,
    pub release: KernelResourceTelemetryRelease,
    pub cpu_percent: u32,
    pub cpu_sample_window_ms: u64,
    pub memory: KernelResourceTelemetryMemory,
    pub disk: KernelResourceTelemetryDisk,
    pub process: KernelResourceTelemetryProcess,
    pub logs: KernelResourceTelemetryLogs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum KernelResourceTelemetryRelease {
    #[serde(rename = "verified")]
    Verified {
        #[serde(rename = "runtimeReleaseDigest")]
        runtime_release_digest: String,
        #[serde(rename = "sourceCommit")]
        source_commit: String,
        #[serde(rename = "sourceTree")]
        source_tree: String,
        target: String,
        #[serde(rename = "activeReleasePath")]
        active_release_path: String,
        #[serde(rename = "manifestSignatureVerified")]
        manifest_signature_verified: bool,
        #[serde(rename = "manifestDigestVerified")]
        manifest_digest_verified: bool,
        #[serde(rename = "kernelArtifactVerified")]
        kernel_artifact_verified: bool,
        #[serde(rename = "bootstrapReceiptVerified")]
        bootstrap_receipt_verified: bool,
    },
    #[serde(rename = "unavailable")]
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelResourceTelemetryMetadata {
    pub scope: String,
    pub authoritative: bool,
    pub target_id: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelResourceTelemetryMemory {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelResourceTelemetryDisk {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelResourceTelemetryProcess {
    pub count: u64,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelResourceTelemetryLogs {
    pub bytes: u64,
}
