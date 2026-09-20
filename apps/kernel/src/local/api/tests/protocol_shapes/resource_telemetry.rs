use super::*;
use crate::local::{
    GetKernelResourceTelemetryRequest, KernelResourceTelemetryDisk, KernelResourceTelemetryLogs,
    KernelResourceTelemetryMemory, KernelResourceTelemetryMetadata, KernelResourceTelemetryProcess,
    KernelResourceTelemetryRelease, KernelResourceTelemetrySnapshot,
};

#[test]
fn kernel_resource_telemetry_request_and_response_shape_is_versioned() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 336);

    let request = LocalDaemonRequest::GetKernelResourceTelemetry(GetKernelResourceTelemetryRequest);
    let response = LocalDaemonResponse::KernelResourceTelemetry {
        snapshot: KernelResourceTelemetrySnapshot {
            schema: "chariox.kernel.resource_telemetry.v3".to_string(),
            captured_at: "2026-09-20T00:00:00.000Z".to_string(),
            captured_at_monotonic_ms: 42,
            telemetry: KernelResourceTelemetryMetadata {
                scope: "managed-target".to_string(),
                authoritative: true,
                target_id: "machine-1".to_string(),
                source: "kernel".to_string(),
            },
            release: KernelResourceTelemetryRelease::Verified {
                runtime_release_digest: format!("sha256:{}", "d".repeat(64)),
                source_commit: "a".repeat(40),
                source_tree: "b".repeat(40),
                target: "x86_64-unknown-linux-gnu".to_string(),
                active_release_path: "/usr/lib/chariox/releases/release-1".to_string(),
                manifest_signature_verified: true,
                manifest_digest_verified: true,
                kernel_artifact_verified: true,
                bootstrap_receipt_verified: true,
            },
            cpu_percent: 37,
            cpu_sample_window_ms: 100,
            memory: KernelResourceTelemetryMemory {
                used_bytes: 4_000,
                total_bytes: 8_000,
                available_bytes: 4_000,
            },
            disk: KernelResourceTelemetryDisk {
                used_bytes: 4_000,
                total_bytes: 16_000,
                available_bytes: 12_000,
            },
            process: KernelResourceTelemetryProcess {
                count: 3,
                rss_bytes: 100,
            },
            logs: KernelResourceTelemetryLogs { bytes: 256 },
        },
    };

    let request_wire = serde_json::json!({ "GetKernelResourceTelemetry": null });
    let response_wire = serde_json::json!({
        "KernelResourceTelemetry": {
            "snapshot": {
                "schema": "chariox.kernel.resource_telemetry.v3",
                "capturedAt": "2026-09-20T00:00:00.000Z",
                "capturedAtMonotonicMs": 42,
                "telemetry": {
                    "scope": "managed-target",
                    "authoritative": true,
                    "targetId": "machine-1",
                    "source": "kernel"
                },
                "release": {
                    "status": "verified",
                    "runtimeReleaseDigest": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "sourceCommit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "sourceTree": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "target": "x86_64-unknown-linux-gnu",
                    "activeReleasePath": "/usr/lib/chariox/releases/release-1",
                    "manifestSignatureVerified": true,
                    "manifestDigestVerified": true,
                    "kernelArtifactVerified": true,
                    "bootstrapReceiptVerified": true
                },
                "cpuPercent": 37,
                "cpuSampleWindowMs": 100,
                "memory": { "usedBytes": 4000, "totalBytes": 8000, "availableBytes": 4000 },
                "disk": { "usedBytes": 4000, "totalBytes": 16000, "availableBytes": 12000 },
                "process": { "count": 3, "rssBytes": 100 },
                "logs": { "bytes": 256 }
            }
        }
    });
    assert_eq!(serde_json::to_value(&request).unwrap(), request_wire);
    assert_eq!(serde_json::to_value(&response).unwrap(), response_wire);
    assert_eq!(
        serde_json::from_value::<LocalDaemonRequest>(request_wire.clone()).unwrap(),
        request
    );
    assert_eq!(
        serde_json::from_value::<LocalDaemonResponse>(response_wire.clone()).unwrap(),
        response
    );

    let snapshot = serde_json::json!([request_wire, response_wire]);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    let hash = Sha256::digest(serialized.as_bytes());
    assert_eq!(
        format!("{hash:x}"),
        "49d4d2b207a08c246264611efcd94abe9caa40f6aad2977c8028dbbfb1b5b3e6"
    );
}
