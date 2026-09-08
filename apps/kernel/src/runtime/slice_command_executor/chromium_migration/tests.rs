use super::*;

fn container(id: &str, running: bool, migration: Option<&str>) -> docker::Container {
    serde_json::from_value(serde_json::json!({
        "id": id, "running": running, "policy": "policy-digest", "migration": migration, "mounts": []
    })).unwrap()
}

#[test]
fn restart_layouts_require_owned_ids_without_assuming_the_original_still_exists() {
    let original = container("source", false, None);
    let candidate = container("candidate", true, Some("checkpoint"));
    for (canonical, rollback) in [
        (Some(&original), None),
        (None, Some(&original)),
        (Some(&candidate), Some(&original)),
        (Some(&candidate), None),
        (None, None),
    ] {
        validate_layout_identity("source", "checkpoint", canonical, rollback).unwrap();
    }
}

#[test]
fn unrelated_or_restarted_containers_cannot_enter_replacement_or_rollback() {
    let original = container("source", false, None);
    let unrelated = container("unrelated", false, None);
    let restarted = container("source", true, None);
    let other_migration = container("candidate", false, Some("different-checkpoint"));
    for (canonical, rollback) in [
        (Some(&unrelated), Some(&original)),
        (Some(&other_migration), Some(&original)),
        (None, Some(&unrelated)),
        (None, Some(&restarted)),
        (Some(&restarted), None),
        (Some(&original), Some(&original)),
    ] {
        assert!(validate_layout_identity("source", "checkpoint", canonical, rollback).is_err());
    }
}

fn checkpoint() -> SliceSavedStateRecord {
    SliceSavedStateRecord {
        id: "owned-checkpoint".into(),
        slice_name: "browser".into(),
        source_slice_id: "slice-1".into(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".into(),
        image_ref: "retained-image".into(),
        home_archive_path: "/private/checkpoint/home.tar.zst".into(),
        manifest_path: "/private/checkpoint/manifest.json".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
        size_bytes: Some(1),
        last_operation: Some(docker::MIGRATION_OPERATION.into()),
        last_operation_status: Some(SliceOperationStatus::InProgress),
        last_error: None,
    }
}

#[derive(Default)]
struct Fake {
    calls: Vec<&'static str>,
    fail: Vec<&'static str>,
}

impl Fake {
    fn step(&mut self, name: &'static str) -> Result<(), DaemonError> {
        self.calls.push(name);
        if self.fail.contains(&name) {
            Err(docker::error(format!("injected {name} failure")))
        } else {
            Ok(())
        }
    }
}

impl MigrationDriver for Fake {
    fn stage(&mut self, _: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        self.step("stage")
    }
    fn provision(&mut self, _: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        self.step("provision")
    }
    fn verify(&mut self, _: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        self.step("verify")
    }
    fn rollback(&mut self, _: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        self.step("rollback")
    }
    fn cleanup(&mut self, _: &SliceSavedStateRecord) -> Result<(), DaemonError> {
        self.step("cleanup")
    }
    fn audit(&mut self, status: SliceOperationStatus) -> Result<(), DaemonError> {
        self.step(if status == SliceOperationStatus::Completed {
            "complete-audit"
        } else {
            "failed-audit"
        })
    }
    fn persist(
        &mut self,
        checkpoint: &mut SliceSavedStateRecord,
        status: SliceOperationStatus,
    ) -> Result<(), DaemonError> {
        self.step(if status == SliceOperationStatus::Completed {
            "complete"
        } else {
            "failed"
        })?;
        checkpoint.last_operation_status = Some(status);
        Ok(())
    }
}

#[test]
fn verification_precedes_durable_success_and_source_cleanup() {
    let mut driver = Fake::default();
    let mut state = checkpoint();
    execute(&mut driver, &mut state).unwrap();
    assert_eq!(
        driver.calls,
        [
            "stage",
            "provision",
            "verify",
            "complete",
            "complete-audit",
            "cleanup"
        ]
    );
    assert_eq!(
        state.last_operation_status,
        Some(SliceOperationStatus::Completed)
    );
    assert_eq!(state.image_ref, "retained-image");
    assert_eq!(state.home_archive_path, "/private/checkpoint/home.tar.zst");
}

#[test]
fn failed_identity_staging_never_enters_candidate_cleanup() {
    let mut driver = Fake {
        fail: vec!["stage"],
        ..Fake::default()
    };
    assert!(execute(&mut driver, &mut checkpoint()).is_err());
    assert_eq!(driver.calls, ["stage"]);
}

#[test]
fn each_precommit_failure_restores_before_recording_failed_state() {
    for failure in ["provision", "verify", "complete"] {
        let mut driver = Fake {
            fail: vec![failure],
            ..Fake::default()
        };
        let mut state = checkpoint();
        let error = execute(&mut driver, &mut state).unwrap_err();
        assert!(error.to_string().contains("owned-checkpoint"));
        assert_eq!(
            &driver.calls[driver.calls.len() - 3..],
            ["rollback", "failed", "failed-audit"]
        );
        assert!(!driver.calls.contains(&"cleanup"));
        assert_eq!(
            state.last_operation_status,
            Some(SliceOperationStatus::Failed)
        );
        assert_eq!(state.image_ref, "retained-image");
    }
}

#[test]
fn incomplete_rollback_is_explicit_and_retains_the_checkpoint() {
    let mut driver = Fake {
        fail: vec!["verify", "rollback"],
        ..Fake::default()
    };
    let mut state = checkpoint();
    let error = execute(&mut driver, &mut state).unwrap_err();
    assert!(error.to_string().contains("rollback requires retry"));
    assert_eq!(state.id, "owned-checkpoint");
    assert!(!driver.calls.contains(&"cleanup"));
}

#[test]
fn verified_migration_can_retain_old_container_when_cleanup_is_unavailable() {
    let mut driver = Fake {
        fail: vec!["cleanup"],
        ..Fake::default()
    };
    let mut state = checkpoint();
    execute(&mut driver, &mut state).unwrap();
    assert_eq!(
        state.last_operation_status,
        Some(SliceOperationStatus::Completed)
    );
    assert!(!driver.calls.contains(&"rollback"));
}

#[test]
fn completed_migration_cleanup_retry_never_rolls_back_later_user_data() {
    for failure in ["provision", "verify"] {
        let mut driver = Fake {
            fail: vec![failure],
            ..Fake::default()
        };
        let mut state = checkpoint();
        state.last_operation_status = Some(SliceOperationStatus::Completed);
        assert!(execute(&mut driver, &mut state).is_err());
        assert!(!driver.calls.contains(&"stage"));
        assert!(!driver.calls.contains(&"rollback"));
        assert!(!driver.calls.contains(&"failed"));
        assert!(!driver.calls.contains(&"cleanup"));
        assert_eq!(
            state.last_operation_status,
            Some(SliceOperationStatus::Completed)
        );
    }
    let mut driver = Fake::default();
    let mut state = checkpoint();
    state.last_operation_status = Some(SliceOperationStatus::Completed);
    execute(&mut driver, &mut state).unwrap();
    assert_eq!(driver.calls, ["provision", "verify", "cleanup"]);
}

#[test]
fn audit_failure_after_durable_completion_never_triggers_rollback() {
    let mut driver = Fake {
        fail: vec!["complete-audit"],
        ..Fake::default()
    };
    let mut state = checkpoint();
    execute(&mut driver, &mut state).unwrap();
    assert_eq!(
        state.last_operation_status,
        Some(SliceOperationStatus::Completed)
    );
    assert!(driver.calls.contains(&"complete-audit"));
    assert!(driver.calls.contains(&"cleanup"));
    assert!(!driver.calls.contains(&"rollback"));
    assert!(!driver.calls.contains(&"failed"));
}

#[test]
fn failure_to_record_recovery_never_reports_success() {
    let mut driver = Fake {
        fail: vec!["provision", "failed"],
        ..Fake::default()
    };
    let error = execute(&mut driver, &mut checkpoint()).unwrap_err();
    assert!(error.to_string().contains("recovery record requires retry"));
    assert!(!driver.calls.contains(&"cleanup"));
}
