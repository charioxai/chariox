// MP-08 / MP-09 / MP-10 / MP-11, local 1004 and peer 443 (files r1): Files on a machine and the
// kernel-owned transfers between the owner's machines.
use crate::local::{
    FileTransferConflict, FileTransferEndpoint, FileTransferJob, FileTransferRef,
    FileTransferState, KernelDirectoryEntry, KernelFileKind, KernelFileStat,
    ListKernelDirectoryRequest, LocalDaemonRequest, LocalDaemonResponse, StartFileTransferRequest,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

fn job() -> FileTransferJob {
    FileTransferJob {
        job_id: "transfer-1".into(),
        state: FileTransferState::Copying,
        source: FileTransferEndpoint {
            kernel: Some("b3".into()),
            path: "/srv/logs".into(),
        },
        destination: "/Users/me/logs".into(),
        delete_source: true,
        bytes_done: 10,
        bytes_total: 40,
        files_done: 1,
        files_total: 3,
        skipped: 0,
        current: Some("/srv/logs/a.log".into()),
        sha256: None,
        error: None,
        started_ms: 5,
        finished_ms: None,
    }
}

#[test]
fn mp11_machine_files_local_1004_and_peer_443_are_pinned() {
    const { assert!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION >= 1004) };
    const { assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 443) };
    let start = StartFileTransferRequest {
        source: FileTransferEndpoint {
            kernel: Some("b3".into()),
            path: "/srv/logs".into(),
        },
        destination_folder: "~/Downloads".into(),
        name: None,
        conflict: FileTransferConflict::KeepBoth,
        delete_source: true,
    };
    let local = serde_json::json!({
        "start": LocalDaemonRequest::StartFileTransfer(start.clone()),
        "start_minimal": serde_json::from_value::<LocalDaemonRequest>(serde_json::json!(
            {"StartFileTransfer": {"source": {"path": "/a"}, "destination_folder": "/b"}})).unwrap(),
        "get": LocalDaemonRequest::GetFileTransfer(FileTransferRef { job_id: "transfer-1".into() }),
        "list": LocalDaemonRequest::ListFileTransfers,
        "cancel": LocalDaemonRequest::CancelFileTransfer(FileTransferRef { job_id: "transfer-1".into() }),
        "page": LocalDaemonRequest::ListKernelDirectory(ListKernelDirectoryRequest { path: "/srv".into(), offset: 2000 }),
        "job": LocalDaemonResponse::FileTransfer { job: job() },
        "jobs": LocalDaemonResponse::FileTransfers { jobs: vec![job()] },
        "entry": KernelDirectoryEntry {
            name: "a.log".into(), kind: KernelFileKind::File, size: 3, modified_ms: Some(7),
            revision: "2049:17:3:5.0:5.0".into(), mode: 0o644,
        },
    });
    assert_eq!(
        local,
        serde_json::json!({
            "start": {"StartFileTransfer": {"source": {"kernel": "b3", "path": "/srv/logs"},
                "destination_folder": "~/Downloads", "conflict": "keep_both", "move": true}},
            "start_minimal": {"StartFileTransfer": {"source": {"path": "/a"},
                "destination_folder": "/b", "conflict": "fail", "move": false}},
            "get": {"GetFileTransfer": {"job_id": "transfer-1"}},
            "list": "ListFileTransfers",
            "cancel": {"CancelFileTransfer": {"job_id": "transfer-1"}},
            "page": {"ListKernelDirectory": {"path": "/srv", "offset": 2000}},
            "job": {"FileTransfer": {"job": {"job_id": "transfer-1", "state": "copying",
                "source": {"kernel": "b3", "path": "/srv/logs"}, "destination": "/Users/me/logs",
                "delete_source": true, "bytes_done": 10, "bytes_total": 40, "files_done": 1,
                "files_total": 3, "skipped": 0, "current": "/srv/logs/a.log", "started_ms": 5}}},
            "jobs": {"FileTransfers": {"jobs": [{"job_id": "transfer-1", "state": "copying",
                "source": {"kernel": "b3", "path": "/srv/logs"}, "destination": "/Users/me/logs",
                "delete_source": true, "bytes_done": 10, "bytes_total": 40, "files_done": 1,
                "files_total": 3, "skipped": 0, "current": "/srv/logs/a.log", "started_ms": 5}]}},
            "entry": {"name": "a.log", "kind": "file", "size": 3, "modified_ms": 7,
                "revision": "2049:17:3:5.0:5.0", "mode": 420},
        })
    );

    let stat = KernelFileStat {
        path: "/srv/a.log".into(),
        exists: true,
        kind: Some(KernelFileKind::File),
        size: Some(3),
        modified_ms: Some(5),
        revision: Some("2049:17:3:5.0:5.0".into()),
        mode: Some(0o644),
        sha256: None,
        partial_size: None,
    };
    let requests = [
        RelayPeerRequest::StatMachinePath {
            path: "/srv/a.log".into(),
        },
        RelayPeerRequest::ListMachineDirectory {
            path: "/srv".into(),
            offset: 0,
        },
        RelayPeerRequest::ReadMachineFile {
            path: "/srv/a.log".into(),
            offset: 4_194_304,
        },
        RelayPeerRequest::DeleteMachinePath {
            path: "/srv/a.log".into(),
            expect_revision: Some("r1".into()),
        },
        RelayPeerRequest::DeleteMachinePath {
            path: "/srv".into(),
            expect_revision: None,
        },
        RelayPeerRequest::StartFileTransfer { request: start },
        RelayPeerRequest::GetFileTransfer {
            job_id: "transfer-1".into(),
        },
        RelayPeerRequest::CancelFileTransfer {
            job_id: "transfer-1".into(),
        },
    ];
    let responses = [
        RelayPeerResponse::MachinePathStat { stat },
        RelayPeerResponse::MachineDirectory {
            entries: vec![],
            truncated: true,
        },
        RelayPeerResponse::MachinePathDeleted,
        RelayPeerResponse::FileTransfer { job: job() },
    ];
    assert_eq!(
        requests
            .iter()
            .map(|request| serde_json::to_value(request).unwrap())
            .collect::<Vec<_>>(),
        [
            serde_json::json!({"kind": "stat_machine_path", "path": "/srv/a.log"}),
            serde_json::json!({"kind": "list_machine_directory", "path": "/srv", "offset": 0}),
            serde_json::json!({"kind": "read_machine_file", "path": "/srv/a.log", "offset": 4_194_304}),
            serde_json::json!({"kind": "delete_machine_path", "path": "/srv/a.log", "expect_revision": "r1"}),
            serde_json::json!({"kind": "delete_machine_path", "path": "/srv", "expect_revision": null}),
            serde_json::json!({"kind": "start_file_transfer", "request": {
                "source": {"kernel": "b3", "path": "/srv/logs"}, "destination_folder": "~/Downloads",
                "conflict": "keep_both", "move": true}}),
            serde_json::json!({"kind": "get_file_transfer", "job_id": "transfer-1"}),
            serde_json::json!({"kind": "cancel_file_transfer", "job_id": "transfer-1"}),
        ]
    );
    assert_eq!(
        responses
            .iter()
            .map(|response| serde_json::to_value(response).unwrap())
            .collect::<Vec<_>>(),
        [
            serde_json::json!({"kind": "machine_path_stat", "stat": {"path": "/srv/a.log",
                "exists": true, "kind": "file", "size": 3, "modified_ms": 5,
                "revision": "2049:17:3:5.0:5.0", "mode": 420, "sha256": null, "partial_size": null}}),
            serde_json::json!({"kind": "machine_directory", "entries": [], "truncated": true}),
            serde_json::json!({"kind": "machine_path_deleted"}),
            serde_json::json!({"kind": "file_transfer", "job": {"job_id": "transfer-1",
                "state": "copying", "source": {"kernel": "b3", "path": "/srv/logs"},
                "destination": "/Users/me/logs", "delete_source": true, "bytes_done": 10,
                "bytes_total": 40, "files_done": 1, "files_total": 3, "skipped": 0,
                "current": "/srv/logs/a.log", "started_ms": 5}}),
        ]
    );
    for request in requests {
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(value).unwrap(),
            request
        );
    }
    for response in responses {
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(value).unwrap(),
            response
        );
    }
}
