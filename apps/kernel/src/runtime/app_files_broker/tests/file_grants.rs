//! `files.import` of a user-selected grant through the actual worker peer,
//! durable writer and private data root.
use super::*;
use crate::durable_state::app_file_grants::{FileGrantCommand, FilePick, GrantedFile, PickState};
use crate::runtime::app_file_grant_broker::AppFileGrantBroker;
use chariox_app_runtime::worker_peer::{Broker, BrokerFuture};

struct GrantDelegate(AppFileGrantBroker);
impl Broker for GrantDelegate {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let service = self.0.clone();
        Box::pin(async move { service.dispatch(request).await })
    }
}

fn broker(fixture: &Fixture, user_selected: bool) -> Arc<dyn Broker> {
    Arc::new(GrantDelegate(AppFileGrantBroker::fixture(
        fixture.store.clone(),
        "alice".into(),
        fixture.catalog.clone(),
        fixture.admission.clone(),
        fixture.data(),
        user_selected,
    )))
}

#[test]
fn a_granted_file_imports_once_into_private_data_and_only_with_the_capability() {
    let fixture = Fixture::new(Mode::Ready);
    fixture.runtime.block_on(async {
        let mut denied = TestPeer::start_with(broker(&fixture, false));
        denied
            .send("denied", "host.pick_file", serde_json::json!({}))
            .await;
        assert_eq!(
            denied.response().await.1.unwrap_err().code,
            "CAPABILITY_REQUIRED"
        );
        denied.close().await;

        let mut peer = TestPeer::start_with(broker(&fixture, true));
        peer.send(
            "pick",
            "host.pick_file",
            serde_json::json!({"accept": [".md"], "multiple": false}),
        )
        .await;
        let pick = peer.response().await.1.unwrap();
        assert_eq!(pick["state"], "pending");
        let operation = pick["operationId"].as_str().unwrap().to_owned();
        for params in [
            serde_json::json!({"accept": ["md"]}),
            serde_json::json!({"accept": [".md"], "path": "/etc"}),
        ] {
            peer.send("bad", "host.pick_file", params).await;
            assert_eq!(
                peer.response().await.1.unwrap_err().code,
                "INVALID_ARGUMENT"
            );
        }

        // The owner answers in the trusted prompt.
        let store = fixture.store.clone();
        let answered = operation.clone();
        let granted = tokio::task::spawn_blocking(move || {
            store.app_file_grant(FileGrantCommand::Grant {
                owner: "alice".into(),
                operation_id: answered,
                files: vec![GrantedFile {
                    name: "notes.md".into(),
                    contents: b"# Notes".to_vec(),
                }],
                now_ms: crate::session::unix_epoch_ms(),
            })
        })
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(granted.state, PickState::Granted);
        peer.send(
            "status",
            "host.pick_file_status",
            serde_json::json!({"operationId": operation}),
        )
        .await;
        let status = peer.response().await.1.unwrap();
        assert_eq!(status["state"], "granted");
        assert_eq!(status["grantIds"], serde_json::json!(granted.grants));

        let import =
            serde_json::json!({"grantId": granted.grants[0], "destination": "fixture-file"});
        peer.send("import", "files.import", import.clone()).await;
        assert_eq!(
            peer.response().await.1.unwrap(),
            serde_json::json!({"bytesWritten": 7, "name": "notes.md"})
        );
        peer.send("again", "files.import", import).await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "NOT_FOUND");
        peer.close().await;
    });
    assert_eq!(
        fixture.observed.private_file().unwrap(),
        Some(b"# Notes".to_vec())
    );
}

#[test]
fn another_installations_pick_is_not_visible() {
    let fixture = Fixture::new(Mode::Ready);
    let store = fixture.store.clone();
    store
        .app_file_grant(FileGrantCommand::Create(FilePick {
            operation_id: "foreign".into(),
            owner: "alice".into(),
            installation: "someone-else".into(),
            generation: fixture.catalog.generation(),
            accept: Vec::new(),
            multiple: false,
            state: PickState::Pending,
            expires_ms: u64::MAX / 2,
            grants: Vec::new(),
        }))
        .unwrap();
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start_with(broker(&fixture, true));
        peer.send(
            "status",
            "host.pick_file_status",
            serde_json::json!({"operationId": "foreign"}),
        )
        .await;
        assert_eq!(peer.response().await.1.unwrap_err().code, "NOT_FOUND");
        peer.close().await;
    });
}

#[test]
fn an_export_offers_a_copy_of_one_private_file_for_the_owner_to_save_once() {
    let fixture = Fixture::new(Mode::Ready);
    fixture.runtime.block_on(async {
        let mut peer = TestPeer::start(fixture.service());
        peer.replace("write", b"# Plan").await;
        peer.response().await.1.unwrap();
        peer.close().await;

        let mut peer = TestPeer::start_with(broker(&fixture, true));
        for path in ["../fixture-file", "missing"] {
            peer.send("bad", "files.export", serde_json::json!({"path": path}))
                .await;
            assert_eq!(
                peer.response().await.1.unwrap_err().code,
                "INVALID_ARGUMENT"
            );
        }
        peer.send(
            "export",
            "files.export",
            serde_json::json!({"path": "fixture-file"}),
        )
        .await;
        let operation = peer.response().await.1.unwrap()["operationId"]
            .as_str()
            .unwrap()
            .to_owned();
        peer.close().await;

        let store = fixture.store.clone();
        let saved = tokio::task::spawn_blocking(move || {
            store.app_file_export(
                crate::durable_state::app_file_exports::FileExportCommand::Save {
                    owner: "alice".into(),
                    operation_id: operation,
                    now_ms: crate::session::unix_epoch_ms(),
                },
            )
        })
        .await
        .unwrap();
        let Ok(crate::durable_state::app_file_exports::FileExportReply::Saved { name, contents }) =
            saved
        else {
            panic!("the owner saves the offered copy");
        };
        assert_eq!(
            (name.as_str(), contents.as_slice()),
            ("fixture-file", b"# Plan".as_slice())
        );
    });
}
