//! MP-08 / MP-10 / MP-11: D1's allocated local shapes; peer protocol is unchanged.
use crate::local::*;
use chariox_app_runtime::installation::channel::{AcquisitionSource, InstallationChannel};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn app_dev_register_protocol_644_snapshot_and_legacy_requests() {
    let old = json!({"BeginAppInstall": {
        "session_id":"session", "request_id":"retry",
        "upload_handle":format!("upload_{}", "a".repeat(64)),
        "expected_package_digest":format!("sha256:{}", "b".repeat(64))
    }});
    let LocalDaemonRequest::BeginAppInstall(old_install) =
        serde_json::from_value(old.clone()).unwrap()
    else {
        panic!("install request")
    };
    assert_eq!(old_install.source, AcquisitionSource::LocalFile);
    let mut old_update = old["BeginAppInstall"].clone();
    old_update["installation_id"] = json!("dev");
    old_update["expected_generation"] = json!("1");
    let LocalDaemonRequest::BeginAppUpdate(old_update) =
        serde_json::from_value(json!({"BeginAppUpdate":old_update})).unwrap()
    else {
        panic!("update request")
    };
    assert_eq!(old_update.source, AcquisitionSource::LocalFile);
    let mut requests = Vec::new();
    for source in [AcquisitionSource::DevPush, AcquisitionSource::Register] {
        requests.push(LocalDaemonRequest::BeginAppInstall(
            BeginAppInstallRequest {
                source,
                ..old_install.clone()
            },
        ));
        requests.push(LocalDaemonRequest::BeginAppUpdate(BeginAppUpdateRequest {
            source,
            ..old_update.clone()
        }));
    }
    let response = LocalDaemonResponse::AppInstallation {
        installation: AppInstallationSummary {
            installation_id: "dev".into(),
            app_id: "com.example.dev".into(),
            channel: InstallationChannel::Dev,
            dev_room_id: Some("room".into()),
            generation: "1".into(),
            active_release: None,
            pending_generation: Some("2".into()),
            admission_paused: false,
            data_kept: true,
        },
    };
    for request in &requests {
        assert_eq!(
            *request,
            serde_json::from_value::<LocalDaemonRequest>(serde_json::to_value(request).unwrap())
                .unwrap()
        );
    }
    let snapshot = json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION, "requests":requests,"response":response});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("app-dev-register-644.json")).unwrap();
    assert_eq!(
        snapshot, expected,
        "D1's serialized shapes require the allocated local version"
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "bea055a6b01732e149feb5b0629889c307bb674e507d1d9537575c20a1ded5b9"
    );
}
