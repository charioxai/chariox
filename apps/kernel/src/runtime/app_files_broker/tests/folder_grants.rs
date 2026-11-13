//! Actual SDK peer and signed installation, durable owner grant and writer.
use super::*;
use crate::durable_state::app_folder_grants::FolderCommand;
use chariox_app_runtime::external_folder::{ExternalFolder, FolderAccess};
use serde_json::json;

struct Folder(std::path::PathBuf);
impl Folder {
    fn new() -> Self {
        let path = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-folder-broker-{:032x}",
                rand::random::<u128>()
            ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn grant(&self, fixture: &Fixture, operation: &str, access: FolderAccess) -> String {
        fixture
            .store
            .app_folder_command(FolderCommand::Grant {
                owner: "alice".into(),
                catalog: fixture.catalog.clone(),
                operation_id: operation.into(),
                path: self.0.to_str().unwrap().into(),
                identity: ExternalFolder::select(&self.0).unwrap().identity(),
                access,
                now_ms: crate::session::unix_epoch_ms(),
            })
            .unwrap()["grantId"]
            .as_str()
            .unwrap()
            .into()
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn folder_capability_and_owner_grant_are_required_before_broker_io() {
    let denied = Fixture::new(Mode::Ready);
    denied.runtime.block_on(async {
        let mut peer = TestPeer::start_with(denied.broker());
        peer.send("pick", "host.pick_folder", json!({})).await;
        assert_eq!(
            peer.response().await.1.unwrap_err().code,
            "CAPABILITY_REQUIRED"
        );
        peer.close().await;
    });
    let fixture = Fixture::with_folder(Mode::Ready);
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        peer.send(
            "missing",
            "folders.read",
            json!({"grantId":"unknown","path":"a.md"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "NOT_FOUND");
        peer.send(
            "forged",
            "host.pick_folder",
            json!({"path":"/etc","owner":"alice"}),
        )
        .await;
        assert_eq!(
            peer.response().await.1.unwrap_err().code,
            "INVALID_ARGUMENT"
        );
        peer.close().await;
    });
}

#[test]
fn a_durable_read_only_grant_reads_without_exposing_the_original_path_and_revokes() {
    let fixture = Fixture::with_folder(Mode::Ready);
    let folder = Folder::new();
    std::fs::write(folder.0.join("notes.md"), b"# Notes").unwrap();
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        peer.send("pick", "host.pick_folder", json!({})).await;
        let pick = peer.response().await.1.unwrap();
        let operation = pick["operationId"].as_str().unwrap();
        assert!(fixture
            .store
            .app_folder_command(FolderCommand::Grant {
                owner: "mallory".into(),
                catalog: fixture.catalog.clone(),
                operation_id: operation.into(),
                path: folder.0.to_str().unwrap().into(),
                identity: ExternalFolder::select(&folder.0).unwrap().identity(),
                access: FolderAccess::ReadWrite,
                now_ms: crate::session::unix_epoch_ms()
            })
            .is_err());
        let grant = folder.grant(&fixture, operation, FolderAccess::ReadOnly);
        peer.send(
            "status",
            "host.pick_folder_status",
            json!({"operationId":operation}),
        )
        .await;
        let status = peer.response().await.1.unwrap();
        assert_eq!(status["grantId"], grant);
        assert!(!status.to_string().contains(folder.0.to_str().unwrap()));
        peer.send(
            "read",
            "folders.read",
            json!({"grantId":grant,"path":"notes.md"}),
        )
        .await;
        assert_eq!(
            peer.response().await.1.unwrap(),
            json!({"contentsBase64":"IyBOb3Rlcw=="})
        );
        peer.send(
            "write",
            "folders.write",
            json!({"grantId":grant,"path":"notes.md","contentsBase64":"bmV3"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "READ_ONLY");
        peer.send(
            "escape",
            "folders.read",
            json!({"grantId":grant,"path":"../notes.md"}),
        )
        .await;
        assert_eq!(
            peer.response().await.1.unwrap_err().code,
            "INVALID_ARGUMENT"
        );
        fixture
            .store
            .app_folder_command(FolderCommand::Revoke {
                owner: "alice".into(),
                installation: fixture.catalog.installation_id().into(),
                grant_id: grant.clone(),
            })
            .unwrap();
        peer.send(
            "revoked",
            "folders.read",
            json!({"grantId":grant,"path":"notes.md"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "NOT_FOUND");
        peer.close().await;
    });
    assert_eq!(
        std::fs::read(folder.0.join("notes.md")).unwrap(),
        b"# Notes"
    );
}

#[test]
fn folder_writes_publish_under_quota_and_rate_and_permission_changes_end_the_grant() {
    let fixture = Fixture::with_folder(Mode::Ready);
    let folder = Folder::new();
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        peer.send("pick", "host.pick_folder", json!({})).await;
        let pick = peer.response().await.1.unwrap();
        let grant = folder.grant(
            &fixture,
            pick["operationId"].as_str().unwrap(),
            FolderAccess::ReadWrite,
        );
        peer.send(
            "write",
            "folders.write",
            json!({"grantId":grant,"path":"notes.md","contentsBase64":"IyBOb3Rlcw=="}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap()["bytesWritten"], 7);
        assert_eq!(
            std::fs::read(folder.0.join("notes.md")).unwrap(),
            b"# Notes"
        );
        let connection = rusqlite::Connection::open(fixture.store.path()).unwrap();
        connection
            .execute(
                "UPDATE app_folder_grants SET written_bytes=536870912 WHERE grant_id=?1",
                [&grant],
            )
            .unwrap();
        peer.send(
            "quota",
            "folders.write",
            json!({"grantId":grant,"path":"notes.md","contentsBase64":"bmV3"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "QUOTA_EXCEEDED");
        connection
            .execute(
                "UPDATE app_folder_grants SET calls=60 WHERE grant_id=?1",
                [&grant],
            )
            .unwrap();
        peer.send("rate", "folders.list", json!({"grantId":grant}))
            .await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "RATE_LIMITED");
        connection
            .execute(
                "UPDATE app_installations SET active_json=json_set(active_json,
             '$.release.capabilities_digest','changed') WHERE installation_id=?1",
                [fixture.catalog.installation_id()],
            )
            .unwrap();
        assert!(fixture
            .store
            .list_app_folder_grants("alice", fixture.catalog.installation_id())
            .unwrap()
            .is_empty());
        peer.close().await;
    });
    assert_eq!(
        std::fs::read(folder.0.join("notes.md")).unwrap(),
        b"# Notes"
    );
}

#[test]
fn revoking_after_disk_staging_prevents_publication_and_removes_the_staged_file() {
    let fixture = Fixture::with_folder(Mode::Ready);
    let folder = Folder::new();
    std::fs::write(folder.0.join("notes.md"), b"before").unwrap();
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        peer.send("pick", "host.pick_folder", json!({})).await;
        let pick = peer.response().await.1.unwrap();
        let grant = folder.grant(
            &fixture,
            pick["operationId"].as_str().unwrap(),
            FolderAccess::ReadWrite,
        );
        let budget =
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false);
        let prepared = fixture
            .store
            .stage_app_folder_write(
                "alice".into(),
                fixture.catalog.clone(),
                grant.clone(),
                "notes.md",
                b"must not publish",
                budget,
            )
            .unwrap();
        fixture
            .store
            .app_folder_command(FolderCommand::Revoke {
                owner: "alice".into(),
                installation: fixture.catalog.installation_id().into(),
                grant_id: grant,
            })
            .unwrap();
        assert_eq!(
            fixture
                .store
                .app_folder_command(FolderCommand::Publish { prepared }),
            Err("NOT_FOUND")
        );
        assert_eq!(std::fs::read(folder.0.join("notes.md")).unwrap(), b"before");
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);
        peer.close().await;
    });
}

#[test]
fn cancelling_after_staging_cannot_evade_the_folder_write_budget() {
    let fixture = Fixture::with_folder(Mode::Ready);
    let folder = Folder::new();
    std::fs::write(folder.0.join("notes.md"), b"before").unwrap();
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(fixture.broker());
        peer.send("pick", "host.pick_folder", json!({})).await;
        let pick = peer.response().await.1.unwrap();
        let grant = folder.grant(
            &fixture,
            pick["operationId"].as_str().unwrap(),
            FolderAccess::ReadWrite,
        );
        let selected = folder.0.clone();
        // Cancel only once real staged bytes exist, independent of the number
        // of cancellation checks before reaching the filesystem.
        let budget =
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(move || {
                std::fs::read_dir(&selected).unwrap().any(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".chariox-folder-")
                })
            });
        assert_eq!(
            fixture.store.prepare_app_folder_write(
                "alice".into(),
                fixture.catalog.clone(),
                grant,
                "notes.md",
                b"must not publish",
                budget
            ),
            Err("CANCELLED")
        );
        assert_eq!(std::fs::read(folder.0.join("notes.md")).unwrap(), b"before");
        let grants = fixture
            .store
            .list_app_folder_grants("alice", fixture.catalog.installation_id())
            .unwrap();
        assert_eq!(
            grants[0].written_bytes, 16,
            "accepted disk staging must spend write budget even if cancelled before publication"
        );
        assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1);
        peer.close().await;
    });
}

#[test]
fn folder_grant_rechecks_publisher_trust_after_the_owner_was_prompted() {
    use crate::durable_state::app_publishers::AppPublisherMutation;
    use chariox_app_runtime::publisher_trust::TrustDecision;
    let fixture = Fixture::with_folder(Mode::Ready);
    let folder = Folder::new();
    let pick = fixture
        .store
        .app_folder_command(FolderCommand::Pick {
            owner: "alice".into(),
            catalog: fixture.catalog.clone(),
            now_ms: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    fixture
        .store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "folder-revoke".into(),
                    authority_ref: "kernel-fixture".into(),
                },
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .unwrap();
    let result = fixture.store.app_folder_command(FolderCommand::Grant {
        owner: "alice".into(),
        catalog: fixture.catalog.clone(),
        operation_id: pick["operationId"].as_str().unwrap().into(),
        path: folder.0.to_str().unwrap().into(),
        identity: ExternalFolder::select(&folder.0).unwrap().identity(),
        access: FolderAccess::ReadOnly,
        now_ms: crate::session::unix_epoch_ms(),
    });
    assert_eq!(result, Err("APP_STALE"));
    assert!(fixture
        .store
        .list_app_folder_grants("alice", fixture.catalog.installation_id())
        .unwrap()
        .is_empty());
}
