use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use super::*;

#[test]
fn transfer_resumes_retries_and_consumes_once_across_restart() {
    let root = test_root("resume");
    let archive = b"portable context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    assert!(!String::from_utf8(
        fs::read(root.join("state.json")).expect("read persisted transfer state")
    )
    .expect("transfer state UTF-8")
    .contains(&armed.capability));
    let source_thumbprint = sha256_bytes(b"source-key");
    let caller = caller(&source_thumbprint);
    let begun = store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    assert_eq!(begun.phase, ManagedContextTransferPhase::Receiving);
    let first = &archive[..8];
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            first,
            &sha256_bytes(first),
            now + 2,
        )
        .expect("upload first chunk");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            first,
            &sha256_bytes(first),
            now + 3,
        )
        .expect("retry first chunk idempotently");
    OpenOptions::new()
        .append(true)
        .open(store.archive_path(&armed.transfer_id))
        .and_then(|mut file| file.write_all(b"uncommitted crash tail"))
        .expect("simulate append before journal commit");
    drop(store);

    let store = ManagedContextTransferStore::open(root.clone()).expect("reopen transfer store");
    let status = store
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 4)
        .expect("resume status");
    assert_eq!(status.accepted_bytes, 8);
    assert_eq!(
        fs::metadata(store.archive_path(&armed.transfer_id))
            .expect("reconciled archive metadata")
            .len(),
        8
    );
    let rest = &archive[8..];
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            8,
            rest,
            &sha256_bytes(rest),
            now + 5,
        )
        .expect("finish upload");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 6)
            .expect("prepare and claim transfer"),
    );
    assert_eq!(
        fs::read(&ready.archive_path).expect("read archive"),
        archive
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 6)
            .expect("retry active import"),
        ManagedContextImportClaim::InProgress(_)
    ));
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir_all(&staging).expect("simulate interrupted import staging");
    fs::write(staging.join("partial"), b"partial materialization")
        .expect("write interrupted staging artifact");
    drop(store);
    let store =
        ManagedContextTransferStore::open(root.clone()).expect("recover interrupted import");
    assert!(!staging.exists());
    assert_eq!(
        store
            .get_status(&armed.transfer_id, &armed.capability, &caller, now + 20_000)
            .expect("recovered import status")
            .phase,
        ManagedContextTransferPhase::Importing
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 20_000,)
            .expect("reclaim import"),
        ManagedContextImportClaim::Claimed(_)
    ));
    let receipt = r#"{"transfer_id":"transfer-1"}"#;
    store
        .commit_import(&armed.transfer_id, &receipt, now + 20_001)
        .expect("consume transfer");
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 20_001,)
            .expect("retry consumed finalize"),
        ManagedContextImportClaim::Terminal(ManagedContextTransferStatus {
            phase: ManagedContextTransferPhase::Consumed,
            ..
        })
    ));
    assert!(!ready.archive_path.exists());
    store
        .commit_import(&armed.transfer_id, &receipt, now + 20_002)
        .expect("replay identical receipt");
    assert_eq!(
        store
            .get_status(&armed.transfer_id, &armed.capability, &caller, now + 20_003,)
            .expect("replay consumed status after upload expiry")
            .phase,
        ManagedContextTransferPhase::Consumed
    );
    let prune_now = now + 20_001 + COMPLETED_TRANSFER_RETENTION_MS + 1;
    let mut replacement = arm_request(b"replacement archive", prune_now + 10_000);
    replacement.destination_parent = root.join("destinations");
    store
        .arm(replacement, prune_now)
        .expect("prune retained completion after its replay window");
    assert!(store
        .get_status(&armed.transfer_id, &armed.capability, &caller, prune_now,)
        .is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn combined_receipt_can_wrap_a_near_limit_development_receipt() {
    let root = test_root("combined-receipt-limit");
    let archive = b"context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            archive,
            &sha256_bytes(archive),
            now + 2,
        )
        .expect("upload archive");
    claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim import"),
    );
    let combined_receipt = serde_json::json!({
        "schemaVersion": 1,
        "development": { "padding": "d".repeat(96 * 1024) },
        "kernelContext": { "kind": "from_kernel" }
    })
    .to_string();
    assert!(combined_receipt.len() > 64 * 1024);
    assert!(combined_receipt.len() <= MAX_IMPORT_RECEIPT_BYTES);
    store
        .commit_import(&armed.transfer_id, &combined_receipt, now + 4)
        .expect("commit bounded combined receipt");
    assert_eq!(
        store
            .get_status(&armed.transfer_id, &armed.capability, &caller, now + 5)
            .expect("combined receipt status")
            .import_receipt_json
            .as_deref(),
        Some(combined_receipt.as_str())
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn arming_reserves_aggregate_state_capacity_for_combined_receipts() {
    let root = test_root("combined-receipt-reservation");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let mut armed_count = 0;
    for sequence in 0..MAX_ACTIVE_TRANSFERS {
        let mut request = arm_request(b"x", now + 10_000);
        request.context_id = format!("context-{sequence}");
        request.destination_parent = root.join("destinations");
        match store.arm(request, now) {
            Ok(_) => armed_count += 1,
            Err(DaemonError::ManagedContext {
                code: "invalid_request",
                ..
            }) => break,
            Err(error) => panic!("unexpected reservation error: {error}"),
        }
    }
    assert!(armed_count > 0);
    assert!(armed_count < MAX_ACTIVE_TRANSFERS);
    let state = store.lock_state();
    ensure_state_capacity_with_receipt_reservations(&state)
        .expect("accepted transfers retain receipt capacity");
    assert!(persisted_state_size(&state).expect("measure state") <= MAX_STATE_FILE_BYTES as usize);
    drop(state);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn transfer_rejects_wrong_bindings_conflicts_expiry_and_oversize_chunks() {
    let root = test_root("authorization");
    let archive = b"archive";
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store
        .arm(arm_request(archive, now + 1_000), now)
        .expect("arm transfer");
    let source_thumbprint = sha256_bytes(b"source-key");
    let caller = caller(&source_thumbprint);
    let wrong = ManagedContextTransferCaller {
        kernel_id: "kernel-wrong".to_string(),
        ..caller.clone()
    };
    assert!(matches!(
        store.begin(&armed.transfer_id, &armed.capability, &wrong, now + 1),
        Err(DaemonError::ManagedContext {
            code: "unauthorized",
            retryable: false,
            ..
        })
    ));
    assert!(matches!(
        store.begin(&armed.transfer_id, "wrong-capability", &caller, now + 1),
        Err(DaemonError::ManagedContext {
            code: "unauthorized",
            retryable: false,
            ..
        })
    ));
    let rebound_target = ManagedContextTransferCaller {
        target_key_thumbprint: sha256_bytes(b"rotated-target-key"),
        ..caller.clone()
    };
    assert!(matches!(
        store.begin(
            &armed.transfer_id,
            &armed.capability,
            &rebound_target,
            now + 1,
        ),
        Err(DaemonError::ManagedContext {
            code: "unauthorized",
            retryable: false,
            ..
        })
    ));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin authorized transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            &archive[..4],
            &sha256_bytes(&archive[..4]),
            now + 2,
        )
        .expect("accept first chunk");
    assert!(store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            b"xxxx",
            &sha256_bytes(b"xxxx"),
            now + 3,
        )
        .is_err());
    let oversized = vec![0_u8; MAX_TRANSFER_CHUNK_BYTES + 1];
    assert!(store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            4,
            &oversized,
            &sha256_bytes(&oversized),
            now + 4,
        )
        .is_err());
    assert!(store
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 1_000,)
        .is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn arming_prunes_expired_state_and_archives_before_issuing_a_new_capability() {
    let root = test_root("prune");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let expired = store
        .arm(arm_request(b"expired", now + 500), now)
        .expect("arm expiring transfer");
    let source_thumbprint = sha256_bytes(b"source-key");
    let caller = caller(&source_thumbprint);
    store
        .begin(&expired.transfer_id, &expired.capability, &caller, now + 1)
        .expect("create expiring archive");
    let expired_archive = store.archive_path(&expired.transfer_id);
    assert!(expired_archive.exists());

    store
        .arm(arm_request(b"replacement", now + 2_000), now + 600)
        .expect("arm replacement transfer");
    assert!(!expired_archive.exists());
    assert!(store
        .get_status(
            &expired.transfer_id,
            &expired.capability,
            &caller,
            now + 600,
        )
        .is_err());
    drop(store);
    ManagedContextTransferStore::open(root.clone()).expect("reopen pruned store");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn persisted_transfer_ids_cannot_escape_the_private_archive_root() {
    let root = test_root("invalid-id");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    store
        .arm(arm_request(b"archive", now + 2_000), now)
        .expect("arm transfer");
    drop(store);

    let state_path = root.join("state.json");
    let mut state: PersistedTransferState = serde_json::from_slice(
        &fs::read(&state_path).expect("read transfer state for corruption regression"),
    )
    .expect("parse transfer state for corruption regression");
    let (_, entry) = state.entries.pop_first().expect("persisted transfer entry");
    state.entries.insert("../../outside".to_string(), entry);
    write_private_state_file(
        &state_path,
        &serde_json::to_vec(&state).expect("serialize malformed state"),
    )
    .expect("write malformed state");

    assert!(ManagedContextTransferStore::open(root.clone()).is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn uncertain_chunk_commit_keeps_bytes_when_the_new_state_was_renamed() {
    let root = test_root("uncertain-chunk");
    let archive_bytes = b"durably accepted chunk";
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store
        .arm(arm_request(archive_bytes, now + 10_000), now)
        .expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    let archive_path = store.archive_path(&armed.transfer_id);
    let mut archive = open_private_archive(&archive_path).expect("open transfer archive");
    archive
        .write_all(archive_bytes)
        .and_then(|_| archive.sync_all())
        .expect("append uncertain chunk");

    let mut state = store.lock_state();
    let mut renamed_state = state.clone();
    renamed_state
        .entries
        .get_mut(&armed.transfer_id)
        .expect("renamed transfer state")
        .accepted_bytes = archive_bytes.len() as u64;
    write_private_state_file(
        &root.join("state.json"),
        &serde_json::to_vec(&renamed_state).expect("serialize renamed state"),
    )
    .expect("persist renamed state");

    store
        .reconcile_uncertain_chunk_persist(
            &mut state,
            &armed.transfer_id,
            0,
            archive_bytes.len() as u64,
            &archive,
        )
        .expect("reconcile uncertain state write");
    assert_eq!(
        state
            .entries
            .get(&armed.transfer_id)
            .expect("reconciled transfer")
            .accepted_bytes,
        archive_bytes.len() as u64
    );
    assert_eq!(
        fs::metadata(&archive_path)
            .expect("reconciled archive metadata")
            .len(),
        archive_bytes.len() as u64
    );
    drop(state);
    drop(archive);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn uncertain_chunk_commit_rolls_back_bytes_when_old_state_remains_authoritative() {
    let root = test_root("uncertain-chunk-rollback");
    let archive_bytes = b"uncommitted chunk";
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store
        .arm(arm_request(archive_bytes, now + 10_000), now)
        .expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    let archive_path = store.archive_path(&armed.transfer_id);
    let mut archive = open_private_archive(&archive_path).expect("open transfer archive");
    archive
        .write_all(archive_bytes)
        .and_then(|_| archive.sync_all())
        .expect("append chunk before failed state write");

    let mut state = store.lock_state();
    store
        .reconcile_uncertain_chunk_persist(
            &mut state,
            &armed.transfer_id,
            0,
            archive_bytes.len() as u64,
            &archive,
        )
        .expect("reconcile old durable offset");
    assert_eq!(
        state
            .entries
            .get(&armed.transfer_id)
            .expect("reconciled transfer")
            .accepted_bytes,
        0
    );
    assert_eq!(
        fs::metadata(&archive_path)
            .expect("rolled-back archive metadata")
            .len(),
        0
    );
    drop(state);
    drop(archive);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn nonretryable_import_retirement_retains_replay_but_removes_artifacts_and_capacity() {
    let root = test_root("retire-import");
    let archive = b"invalid but digest-matching context";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            archive,
            &sha256_bytes(archive),
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("prepare and claim transfer"),
    );
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir(&staging).expect("create failed import staging");
    fs::write(staging.join("partial"), b"partial").expect("write failed import artifact");
    let component_staging = ready
        .archive_path
        .parent()
        .expect("archive parent")
        .join(format!(
            "{}.components",
            ready
                .archive_path
                .file_name()
                .expect("archive name")
                .to_string_lossy()
        ));
    fs::create_dir(&component_staging).expect("create package component staging");
    fs::write(component_staging.join("partial"), b"partial")
        .expect("write package component artifact");

    store
        .retire_import(&armed.transfer_id, "invalid_managed_context", now + 5)
        .expect("retire deterministic failure");
    assert!(!ready.archive_path.exists());
    assert!(!staging.exists());
    assert!(!component_staging.exists());
    let failed = store
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 6)
        .expect("replay failed transfer status");
    assert_eq!(failed.phase, ManagedContextTransferPhase::Failed);
    assert_eq!(
        failed.failure_code.as_deref(),
        Some("invalid_managed_context")
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 7)
            .expect("replay failed finalize"),
        ManagedContextImportClaim::Terminal(ManagedContextTransferStatus {
            phase: ManagedContextTransferPhase::Failed,
            ..
        })
    ));
    store
        .arm(arm_request(b"replacement", now + 20_000), now + 6)
        .expect("retired failure does not consume active capacity");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn interrupted_import_remains_recoverable_after_upload_and_restart_expiry() {
    let root = test_root("durable-import-recovery");
    let archive = b"context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            archive,
            &sha256_bytes(archive),
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("prepare and claim transfer"),
    );
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir(&staging).expect("create abandoned staging");
    let recovery_now = now + 7 * 24 * 60 * 60 * 1_000;
    let mut while_active = arm_request(b"while active", recovery_now + 10_000);
    while_active.destination_parent = root.join("destinations");
    store
        .arm(while_active, recovery_now)
        .expect("active import survives long recovery interval");
    assert!(ready.archive_path.exists());
    assert!(staging.exists());
    store
        .release_import(&armed.transfer_id)
        .expect("simulate crash");
    drop(store);
    let store =
        ManagedContextTransferStore::open(root.clone()).expect("restart with interrupted import");
    assert!(!staging.exists());
    let mut after_crash = arm_request(b"after crash", recovery_now + 10_001);
    after_crash.destination_parent = root.join("destinations");
    store
        .arm(after_crash, recovery_now + 1)
        .expect("unrelated pruning retains interrupted import");
    assert!(ready.archive_path.exists());
    assert!(!staging.exists());
    assert_eq!(
        store
            .get_status(
                &armed.transfer_id,
                &armed.capability,
                &caller,
                recovery_now + 1,
            )
            .expect("interrupted import remains authorized")
            .phase,
        ManagedContextTransferPhase::Importing
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(
                &armed.transfer_id,
                &armed.capability,
                &caller,
                recovery_now + 2,
            )
            .expect("reclaim interrupted import"),
        ManagedContextImportClaim::Claimed(_)
    ));
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v1_active_transfers_are_retired_without_blocking_startup() {
    let root = test_root("schema-v1-active");
    let archive = b"legacy development archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            archive,
            &sha256_bytes(archive),
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim legacy import"),
    );
    fs::create_dir_all(&ready.destination_root).expect("create legacy publication");
    fs::write(
        ready
            .destination_root
            .join(".chariox-managed-import-receipt.json"),
        serde_json::json!({
            "schemaVersion": 1,
            "publicationId": armed.transfer_id,
            "archiveSha256": sha256_bytes(archive),
            "projectId": "project-1",
            "destinationRoot": ready.destination_root,
            "primaryRepositoryId": "repository-1",
            "repositories": []
        })
        .to_string(),
    )
    .expect("write legacy publication receipt");
    drop(store);
    rewrite_state_as_v1(&root.join("state.json"));

    let reopened = ManagedContextTransferStore::open(root.clone())
        .expect("schema-v1 active transfer cannot block startup");
    assert!(reopened.lock_state().entries.is_empty());
    assert!(!ready.archive_path.exists());
    assert!(!ready.destination_root.exists());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(root.join("state.json")).expect("read migrated state")
        )
        .expect("parse migrated state")["schema_version"],
        2
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v1_consumed_receipt_migrates_to_explicit_empty_kernel_context() {
    let root = test_root("schema-v1-consumed");
    let archive = b"legacy development archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            archive,
            &sha256_bytes(archive),
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim legacy import"),
    );
    let development_receipt = serde_json::json!({
        "schemaVersion": 1,
        "publicationId": armed.transfer_id,
        "archiveSha256": sha256_bytes(archive),
        "projectId": "project-1",
        "destinationRoot": ready.destination_root,
        "primaryRepositoryId": "p".repeat(60 * 1024),
        "repositories": [{
            "repositoryId": "repository-1",
            "role": "primary",
            "targetDirectory": "repository",
            "destinationPath": ready.destination_root.join("repository"),
            "headSha": "a".repeat(40)
        }]
    })
    .to_string();
    assert!(development_receipt.len() > 60 * 1024);
    assert!(development_receipt.len() <= 64 * 1024);
    store
        .commit_import(&armed.transfer_id, &development_receipt, now + 4)
        .expect("consume legacy transfer");
    drop(store);
    rewrite_state_as_v1(&root.join("state.json"));

    let reopened = ManagedContextTransferStore::open(root.clone())
        .expect("schema-v1 consumed transfer cannot block startup");
    let status = reopened
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 5)
        .expect("migrated consumed status");
    let receipt: crate::managed_context::package::ManagedContextPackageImportReceipt =
        serde_json::from_str(
            status
                .import_receipt_json
                .as_deref()
                .expect("migrated receipt"),
        )
        .expect("parse migrated package receipt");
    assert!(matches!(
        receipt.kernel_context,
        crate::managed_context::package::ManagedContextImportedKernelContext::Empty
    ));
    assert_eq!(receipt.development.project_id, "project-1");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn startup_cleans_failed_artifacts_before_pruning_the_terminal_record() {
    let root = test_root("failed-startup-cleanup");
    let archive = b"failed archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            0,
            archive,
            &sha256_bytes(archive),
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("prepare and claim transfer"),
    );
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir(&staging).expect("create failed staging");
    fs::write(staging.join("partial"), b"partial").expect("write failed staging artifact");
    {
        let mut state = store.lock_state();
        let entry = state
            .entries
            .get_mut(&armed.transfer_id)
            .expect("persisted transfer");
        entry.phase = ManagedContextTransferPhase::Failed;
        entry.failure_code = Some("invalid_managed_context".to_string());
        entry.completed_at_ms = Some(now.saturating_sub(COMPLETED_TRANSFER_RETENTION_MS + 1));
        store.persist_locked(&state).expect("persist failed phase");
    }
    drop(store);

    ManagedContextTransferStore::open(root.clone()).expect("reopen and prune failed transfer");
    assert!(!ready.archive_path.exists());
    assert!(!staging.exists());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn startup_accepts_a_missing_workspace_parent_for_interrupted_and_failed_imports() {
    for terminal_failure in [false, true] {
        let label = if terminal_failure {
            "missing-failed-parent"
        } else {
            "missing-importing-parent"
        };
        let root = test_root(label);
        let archive = b"context archive";
        let now = current_time_ms();
        let mut request = arm_request(archive, now + 10_000);
        request.destination_parent = root.join("destinations");
        let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
        let armed = store.arm(request, now).expect("arm transfer");
        let caller = caller(&sha256_bytes(b"source-key"));
        store
            .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
            .expect("begin transfer");
        store
            .upload_chunk(
                &armed.transfer_id,
                &armed.capability,
                &caller,
                0,
                archive,
                &sha256_bytes(archive),
                now + 2,
            )
            .expect("upload archive");
        let ready = claimed(
            store
                .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
                .expect("prepare and claim transfer"),
        );
        if terminal_failure {
            {
                let mut state = store.lock_state();
                let entry = state
                    .entries
                    .get_mut(&armed.transfer_id)
                    .expect("persisted transfer");
                entry.phase = ManagedContextTransferPhase::Failed;
                entry.failure_code = Some("invalid_managed_context".to_string());
                entry.completed_at_ms = Some(now + 4);
                store.persist_locked(&state).expect("persist failed phase");
            }
        }
        let destination_parent = ready
            .destination_root
            .parent()
            .expect("destination parent")
            .to_path_buf();
        fs::remove_dir_all(&destination_parent).expect("remove managed workspace parent");
        drop(store);

        let reopened = ManagedContextTransferStore::open(root.clone())
            .expect("missing workspace parent is already clean");
        assert_eq!(
            reopened
                .get_status(&armed.transfer_id, &armed.capability, &caller, now + 5)
                .expect("retained transfer status")
                .phase,
            if terminal_failure {
                ManagedContextTransferPhase::Failed
            } else {
                ManagedContextTransferPhase::Importing
            }
        );
        fs::remove_dir_all(root).expect("remove transfer root");
    }
}

fn rewrite_state_as_v1(path: &std::path::Path) {
    let mut state: serde_json::Value = serde_json::from_slice(
        &fs::read(path).expect("read current transfer state before v1 rewrite"),
    )
    .expect("parse current transfer state before v1 rewrite");
    state["schema_version"] = serde_json::json!(1);
    for entry in state["entries"]
        .as_object_mut()
        .expect("transfer entries")
        .values_mut()
    {
        entry
            .as_object_mut()
            .expect("transfer entry")
            .remove("context_id");
    }
    fs::write(
        path,
        serde_json::to_vec(&state).expect("serialize v1 state"),
    )
    .expect("write v1 transfer state");
}

fn claimed(claim: ManagedContextImportClaim) -> ReadyManagedContextImport {
    match claim {
        ManagedContextImportClaim::Claimed(ready) => ready,
        other => panic!("expected claimed import, got {other:?}"),
    }
}

fn arm_request(archive: &[u8], expires_at_ms: u64) -> ArmManagedContextTransfer {
    ArmManagedContextTransfer {
        context_id: "context-1".to_string(),
        target_environment_id: "environment-1".to_string(),
        target_kernel_id: "kernel-target".to_string(),
        target_key_thumbprint: sha256_bytes(b"target-key"),
        source_kernel_id: "kernel-source".to_string(),
        source_key_thumbprint: sha256_bytes(b"source-key"),
        owner_user_id: "user-1".to_string(),
        realm_id: "realm-1".to_string(),
        project_id: "project-1".to_string(),
        archive_sha256: sha256_bytes(archive),
        archive_size_bytes: archive.len() as u64,
        destination_parent: std::env::temp_dir().join("managed-projects"),
        expires_at_ms,
    }
}

fn caller(source_thumbprint: &str) -> ManagedContextTransferCaller {
    ManagedContextTransferCaller {
        kernel_id: "kernel-source".to_string(),
        key_thumbprint: source_thumbprint.to_string(),
        owner_user_id: "user-1".to_string(),
        realm_id: "realm-1".to_string(),
        target_environment_id: "environment-1".to_string(),
        target_kernel_id: "kernel-target".to_string(),
        target_key_thumbprint: sha256_bytes(b"target-key"),
    }
}

fn test_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "chariox-managed-transfer-{label}-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ))
}
