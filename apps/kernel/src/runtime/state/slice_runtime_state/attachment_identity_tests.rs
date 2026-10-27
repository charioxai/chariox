use super::*;

async fn active_slice() -> (
    KernelRuntimeState,
    crate::slice::SliceRecord,
    String,
    String,
) {
    let (_app, runtime, slice, session, agent) = super::tests::slice_runtime().await;
    runtime
        .owned
        .slice_store
        .set_worker_presence(
            &slice.id,
            Some("worker-kernel-1".into()),
            Some("actual-machine".into()),
            vec!["codex".into()],
            3,
        )
        .unwrap();
    let slice = runtime
        .owned
        .slice_store
        .set_status(&slice.id, crate::slice::SliceStatus::Running, 3)
        .unwrap();
    (runtime, slice, session, agent)
}

fn bind(runtime: &KernelRuntimeState, agent: &str, kernel: &str, machine: &str) {
    let mut binding = runtime
        .owned
        .agent_store
        .get_agent(agent)
        .unwrap()
        .remote_execution()
        .unwrap()
        .clone();
    binding.worker_kernel_id = kernel.into();
    binding.worker_machine_id = machine.into();
    runtime
        .owned
        .agent_store
        .bind_remote_execution(agent, binding)
        .unwrap();
}

#[tokio::test]
async fn slice_attachment_identity_rejects_matching_kernel_on_foreign_machine() {
    let (runtime, slice, _, agent) = active_slice().await;
    bind(&runtime, &agent, "worker-kernel-1", "foreign-machine");
    let detached = runtime
        .owned
        .slice_store
        .detach_agent(&slice.id, &agent, 4)
        .unwrap();
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&detached)
        .await
        .unwrap();
    assert!(
        reconciled.agent_ids.is_empty(),
        "matching Kernel alone is not slice placement"
    );
}

#[tokio::test]
async fn slice_attachment_identity_rejects_old_kernel_on_reused_synthetic_machine() {
    let (runtime, slice, _, agent) = active_slice().await;
    bind(
        &runtime,
        &agent,
        "old-worker-kernel",
        &format!("slice:{}", slice.id),
    );
    let detached = runtime
        .owned
        .slice_store
        .detach_agent(&slice.id, &agent, 4)
        .unwrap();
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&detached)
        .await
        .unwrap();
    assert!(
        reconciled.agent_ids.is_empty(),
        "synthetic Machine cannot override recorded Kernel"
    );
}

#[tokio::test]
async fn slice_attachment_identity_detaches_existing_conflicting_live_worker() {
    let (runtime, slice, _, agent) = active_slice().await;
    bind(&runtime, &agent, "worker-kernel-1", "foreign-machine");
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&slice)
        .await
        .unwrap();
    assert!(
        reconciled.agent_ids.is_empty(),
        "existing attachment must match both recorded identities"
    );
}

#[tokio::test]
async fn slice_attachment_identity_restores_exact_recorded_pair() {
    let (runtime, slice, session, agent) = active_slice().await;
    bind(&runtime, &agent, "worker-kernel-1", "actual-machine");
    let detached = runtime
        .owned
        .slice_store
        .detach_agent(&slice.id, &agent, 4)
        .unwrap();
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&detached)
        .await
        .unwrap();
    assert_eq!(reconciled.agent_ids, vec![agent]);
    assert_eq!(reconciled.session_ids, vec![session]);
}

#[tokio::test]
async fn slice_attachment_identity_rejects_duplicate_exact_recorded_workers() {
    let (runtime, slice, _, agent) = active_slice().await;
    bind(&runtime, &agent, "worker-kernel-1", "actual-machine");
    let mut duplicate = slice.clone();
    duplicate.id = "other-slice".into();
    duplicate.name = "other-slice".into();
    duplicate.agent_ids.clear();
    duplicate.session_ids.clear();
    runtime
        .owned
        .slice_store
        .restore_records(vec![slice.clone(), duplicate]);
    let detached = runtime
        .owned
        .slice_store
        .detach_agent(&slice.id, &agent, 4)
        .unwrap();
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&detached)
        .await
        .unwrap();
    assert!(
        reconciled.agent_ids.is_empty(),
        "duplicate identity cannot select one slice"
    );
}

#[tokio::test]
async fn slice_attachment_identity_preserves_existing_stopped_private_binding_without_live_identity(
) {
    let (_app, runtime, slice, session, agent) = super::tests::slice_runtime().await;
    assert!(slice.worker_kernel_id.is_none() && slice.worker_machine_id.is_none());
    assert_eq!(slice.status, crate::slice::SliceStatus::Stopped);
    bind(
        &runtime,
        &agent,
        "previous-worker-kernel",
        "slice:previous-worker",
    );
    runtime.owned.session_projection.remove(&session);
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&slice)
        .await
        .unwrap();
    assert_eq!(reconciled.agent_ids, vec![agent]);
    assert_eq!(reconciled.session_ids, vec![session]);
}

async fn canonical_stopped_slice(
    machine: &str,
) -> (KernelRuntimeState, crate::slice::SliceRecord, String) {
    let (_app, runtime, mut slice, _, agent) = super::tests::slice_runtime().await;
    slice.worker_kernel_ref = crate::slice::hosted_worker_test_support::canonical_worker(0);
    slice.owner_machine_id = crate::slice::hosted_worker_test_support::MACHINE.into();
    runtime
        .owned
        .slice_store
        .restore_records(vec![slice.clone()]);
    bind(&runtime, &agent, &slice.worker_kernel_ref, machine);
    (runtime, slice, agent)
}

#[tokio::test]
async fn slice_attachment_identity_rejects_canonical_stopped_parent_conflict() {
    let (runtime, slice, _) = canonical_stopped_slice("foreign-machine").await;
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&slice)
        .await
        .unwrap();
    assert!(
        reconciled.agent_ids.is_empty(),
        "canonical authority remains available while stopped"
    );
}

#[tokio::test]
async fn slice_attachment_identity_preserves_canonical_stopped_exact_parent() {
    let (runtime, slice, agent) =
        canonical_stopped_slice(crate::slice::hosted_worker_test_support::MACHINE).await;
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&slice)
        .await
        .unwrap();
    assert_eq!(reconciled.agent_ids, vec![agent]);
}

#[tokio::test]
async fn slice_attachment_identity_rejects_stopped_duplicate_even_without_live_observations() {
    let (_app, runtime, mut slice, _, agent) = super::tests::slice_runtime().await;
    slice.owner_machine_id = "actual-machine".into();
    slice.worker_kernel_ref = "previous-worker".into();
    let mut duplicate = slice.clone();
    duplicate.id = "other-slice".into();
    duplicate.name = "other-slice".into();
    duplicate.agent_ids.clear();
    duplicate.session_ids.clear();
    runtime
        .owned
        .slice_store
        .restore_records(vec![slice.clone(), duplicate]);
    bind(&runtime, &agent, "previous-worker", "actual-machine");
    let reconciled = runtime
        .reconcile_slice_agent_attachments(&slice)
        .await
        .unwrap();
    assert!(
        reconciled.agent_ids.is_empty(),
        "historical fallback must not accept ambiguous placement"
    );
}
