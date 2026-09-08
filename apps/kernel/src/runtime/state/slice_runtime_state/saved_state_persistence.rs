use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::slice::{SliceRecord, SliceSavedStateRecord, SliceStore};

pub(super) fn publish(
    slices: &SliceStore,
    durable: &DurableKernelStateStore,
    slice_ref: &str,
    state: SliceSavedStateRecord,
) -> Result<SliceRecord, DaemonError> {
    slices.upsert_saved_state_after_commit(
        slice_ref,
        state,
        crate::session::unix_epoch_ms(),
        |state, slice| {
            // The existing writer acknowledges only after both events commit in
            // one SQLite transaction. Volatile state is then published infallibly.
            durable.append_saved_slice_state(state, slice)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_state_and_reference_commit_atomically_before_volatile_publication() {
        let root = std::env::temp_dir().join(format!(
            "chariox-slice-publish-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let path = root.join("state.sqlite");
        let durable = DurableKernelStateStore::open(path.clone()).unwrap();
        let observer = rusqlite::Connection::open(path).unwrap();
        let slices = SliceStore::default();
        let slice = slices
            .create(
                "kernel",
                "machine",
                crate::slice::CreateSliceInput {
                    name: "browser".into(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".into(),
                    display_mode: crate::slice::SliceDisplayMode::Headed,
                    workspace_id: None,
                    worktree_id: None,
                    workspace_mount: None,
                    development: None,
                    worker_kernel_ref: None,
                    display_url: None,
                    provider_auth: vec![],
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .unwrap();
        let state = |id: &str| SliceSavedStateRecord {
            id: id.into(),
            slice_name: slice.name.clone(),
            source_slice_id: slice.id.clone(),
            backend: slice.backend.clone(),
            os: slice.os.clone(),
            image_ref: format!("retained-{id}"),
            home_archive_path: format!("/private/{id}/home.tar.zst"),
            manifest_path: format!("/private/{id}/manifest.json"),
            created_at_ms: 1,
            updated_at_ms: 1,
            size_bytes: Some(1),
            last_operation: Some("state.save".into()),
            last_operation_status: Some(crate::slice::SliceOperationStatus::Completed),
            last_error: None,
        };
        publish(&slices, &durable, &slice.id, state("previous")).unwrap();
        observer.execute_batch("CREATE TRIGGER fail_slice_checkpoint BEFORE INSERT ON durable_state_events WHEN NEW.kind='slice.state.saved' BEGIN SELECT RAISE(FAIL, 'injected checkpoint failure'); END;").unwrap();
        assert!(publish(&slices, &durable, &slice.id, state("uncommitted")).is_err());
        assert!(slices.saved_state("uncommitted").is_err());
        assert_eq!(
            slices
                .resolve(&slice.id)
                .unwrap()
                .saved_state_ref
                .as_deref(),
            Some("previous")
        );
        observer
            .execute_batch("DROP TRIGGER fail_slice_checkpoint;")
            .unwrap();
        observer.execute_batch("CREATE TRIGGER fail_slice_reference BEFORE INSERT ON durable_state_events WHEN NEW.kind='slice.updated' BEGIN SELECT RAISE(FAIL, 'injected reference failure'); END;").unwrap();
        assert!(publish(&slices, &durable, &slice.id, state("checkpoint")).is_err());
        // Failure of the second insert rolls back both events. Neither a
        // separate SQLite reader nor the volatile store sees the new state.
        let count: i64 = observer.query_row("SELECT count(*) FROM durable_state_events WHERE kind='slice.state.saved' AND subject_id='checkpoint'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
        assert!(slices.saved_state("checkpoint").is_err());
        assert_eq!(
            slices
                .resolve(&slice.id)
                .unwrap()
                .saved_state_ref
                .as_deref(),
            Some("previous")
        );
        let updates = durable.load_events_by_kind("slice.updated").unwrap();
        assert_eq!(
            updates.last().unwrap().payload["slice"]["saved_state_ref"],
            "previous"
        );
        assert_eq!(
            slices.saved_state("previous").unwrap().image_ref,
            "retained-previous"
        );
        observer
            .execute_batch("DROP TRIGGER fail_slice_reference;")
            .unwrap();
        let mut checkpoint = state("checkpoint");
        checkpoint.last_operation = Some("chromium.sandbox.migrate".into());
        checkpoint.last_operation_status = Some(crate::slice::SliceOperationStatus::InProgress);
        publish(&slices, &durable, &slice.id, checkpoint.clone()).unwrap();
        let updates = durable.load_events_by_kind("slice.updated").unwrap();
        assert_eq!(
            updates.last().unwrap().payload["slice"]["saved_state_ref"],
            "checkpoint"
        );
        observer.execute_batch("CREATE TRIGGER fail_completion_reference BEFORE INSERT ON durable_state_events WHEN NEW.kind='slice.updated' BEGIN SELECT RAISE(FAIL, 'injected completion reference failure'); END;").unwrap();
        checkpoint.last_operation_status = Some(crate::slice::SliceOperationStatus::Completed);
        assert!(publish(&slices, &durable, &slice.id, checkpoint.clone()).is_err());
        let events = durable.load_events_by_kind("slice.state.saved").unwrap();
        let durable_checkpoint: SliceSavedStateRecord =
            serde_json::from_value(events.last().unwrap().payload["state"].clone()).unwrap();
        assert_eq!(
            durable_checkpoint.last_operation_status,
            Some(crate::slice::SliceOperationStatus::InProgress)
        );
        assert_eq!(
            slices
                .saved_state("checkpoint")
                .unwrap()
                .last_operation_status,
            Some(crate::slice::SliceOperationStatus::InProgress)
        );
        observer
            .execute_batch("DROP TRIGGER fail_completion_reference;")
            .unwrap();
        publish(&slices, &durable, &slice.id, checkpoint).unwrap();
        assert_eq!(
            slices
                .saved_state("checkpoint")
                .unwrap()
                .last_operation_status,
            Some(crate::slice::SliceOperationStatus::Completed)
        );
    }
}
