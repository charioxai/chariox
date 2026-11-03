//! MP-08 / MP-10 / MP-11: fail-first review regression.
use super::*;
// MP-08 / MP-10 / MP-11: exercise operation settlement after a composed probe budget.
#[test]
fn review0821_multi_probe_requirement_settles_at_operation_deadline() {
    use std::{
        io::{Read, Write},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    std::thread::sleep(Duration::from_millis(90));
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    });
    let root = crate::test_support::TestWorktree::new("review0821-budget");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = project(root.path(), "budget");
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    let mut setup = requirement("setup");
    setup.spec=serde_json::from_value(serde_json::json!({"kind":"setup_checks","boundary":"disposable_worker","commands":[],"probes":vec![serde_json::json!({"kind":"http","url":url,"expected_status":200,"timeout_ms":1000});10],"source_path":null,"source_digest":null,"legacy":null})).unwrap();
    draft.project_requirements = vec![setup];
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: env.revision,
        expected_content_digest: env.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store.save_revision_locked(&env, &save, "owner").unwrap();
    env = store.snapshot(&project).unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "generation".into(),
        slice_ref: None,
    };
    let op = operation(&env, &target, vec![]);
    let mut journal = store.load_checks(project.id()).unwrap();
    journal.operations.push(op.clone());
    store.save_checks(project.id(), &journal).unwrap();
    let ctx = context(root.path());
    let began = Instant::now();
    let result = store.run_check_until(
        ctx,
        env,
        op,
        || Ok(project.clone()),
        began + Duration::from_millis(220),
    );
    let elapsed = began.elapsed();
    done.store(true, Ordering::SeqCst);
    server.join().unwrap();
    result.unwrap();
    assert!(
        elapsed < Duration::from_millis(600),
        "multi-probe Check exceeded its absolute budget: {elapsed:?}"
    );
    let checks = store.load_checks(project.id()).unwrap();
    assert_eq!(
        checks.operations.last().unwrap().phase,
        EnvironmentOperationPhase::Ready
    );
    assert_eq!(checks.observations[0].reason_code, "check_budget_exceeded");
    assert_ne!(
        checks.observations[0].status,
        EnvironmentObservationStatus::Ready
    );
}

// MP-08 / MP-10 / MP-11: large regular Files settle within the runner budget and release admission.
#[test]
fn files_measurement_settles_at_operation_deadline_and_allows_next_check() {
    use std::time::{Duration, Instant};
    let root = crate::test_support::TestWorktree::new("files-budget");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = project(root.path(), "files-budget");
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    let folder = draft.folders[0].folder_id.clone();
    let path = root.path().join("document.bin");
    std::fs::write(&path, b"public document").unwrap();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"document.bin","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: env.revision,
        expected_content_digest: env.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store.save_revision_locked(&env, &save, "owner").unwrap();
    env = store.snapshot(&project).unwrap();
    // Sparse bytes use little disk while exercising the real streaming scanner.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(256 * 1024 * 1024)
        .unwrap();
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "generation".into(),
        slice_ref: None,
    };
    let request = |id: &str| crate::local::CheckProjectEnvironmentRequest {
        target: target.clone(),
        project_id: project.id().into(),
        revision_digest: env.content_digest.clone(),
        operation_id: id.into(),
        selected_items: vec!["file".into()],
    };
    let CheckStart::New {
        environment,
        operation,
        ..
    } = store
        .start_check(request("slow-files"), "owner", target.clone(), || {
            Ok(project.clone())
        })
        .unwrap()
    else {
        panic!("new Check expected")
    };
    // MP-08 / MP-10 / MP-11: create durable fixtures before measuring the runner budget.
    let ctx = context(root.path());
    let began = Instant::now();
    store
        .run_check_until(
            ctx,
            *environment,
            operation,
            || Ok(project.clone()),
            began + Duration::from_millis(80),
        )
        .unwrap();
    let elapsed = began.elapsed();
    let journal = store.load_checks(project.id()).unwrap();
    assert!(
        elapsed < Duration::from_millis(600),
        "Files held Check beyond its budget: {elapsed:?}"
    );
    assert_eq!(journal.observations[0].reason_code, "check_budget_exceeded");
    assert_eq!(
        journal.operations.last().unwrap().recovery_state,
        EnvironmentRecoveryState::Settled
    );
    assert!(matches!(
        store
            .start_check(request("next-files"), "owner", target.clone(), || Ok(
                project.clone()
            ))
            .unwrap(),
        CheckStart::New { .. }
    ));
}

// MP-08 / MP-10 / MP-11: parent supervision settles an I/O stall that cannot check time itself.
#[cfg(unix)]
#[test]
fn files_check_reaps_its_owned_worker_during_an_io_stall() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("files-io-stall");
    let store = ProjectEnvironmentStore::new(root.path());
    let project = project(root.path(), "files-io-stall");
    let mut env = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&env);
    let folder = draft.folders[0].folder_id.clone();
    std::fs::write(root.path().join("README.md"), b"public documentation").unwrap();
    draft.folders[0].requirements.push(serde_json::from_value(serde_json::json!({"requirement_id":"file","title":"Public document","scope":{"kind":"folder","folder_id":folder},"origins":[],"spec":{"kind":"files","folder_id":folder,"entries":[{"relative_path":"README.md","kind":"file","content_digest":null,"folder_id":folder,"user_selected":true,"git_ignored":null,"byte_count":null,"credential_filter_verdict":"not_checked","transfer_inclusion":"include","reason":null,"secret_looking":false}]},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap());
    let save = crate::local::SaveProjectEnvironmentRevisionRequest {
        project_id: project.id().into(),
        expected_revision: env.revision,
        expected_content_digest: env.content_digest.clone(),
        draft,
        accepted_proposal_ids: vec![],
        excluded_proposal_ids: vec![],
    };
    store.save_revision_locked(&env, &save, "owner").unwrap();
    env = store.snapshot(&project).unwrap();
    let marker = root.path().join("owned-worker.pid");
    std::env::set_var("CHARIOX_FILES_CHECK_IO_STALL_MARKER", &marker);
    let target = EnvironmentTargetBinding {
        machine_id: "machine".into(),
        target_instance_generation: "generation".into(),
        slice_ref: None,
    };
    let op = operation(&env, &target, vec!["file"]);
    let mut journal = store.load_checks(project.id()).unwrap();
    journal.operations.push(op.clone());
    store.save_checks(project.id(), &journal).unwrap();
    let ctx = context(root.path());
    let state = ctx.config.private_runtime_state_root();
    let began = std::time::Instant::now();
    store
        .run_check_until(
            ctx,
            env,
            op,
            || Ok(project.clone()),
            began + std::time::Duration::from_millis(300),
        )
        .unwrap();
    std::env::remove_var("CHARIOX_FILES_CHECK_IO_STALL_MARKER");
    assert!(began.elapsed() < std::time::Duration::from_millis(800));
    let pid = std::fs::read_to_string(&marker)
        .expect("worker reached the admitted read")
        .parse::<i32>()
        .unwrap();
    assert!(pid > 1);
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "owned worker must be reaped before Check settles"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert!(
        !std::fs::read_dir(state).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("files-check-")),
        "owned helper metadata must be removed"
    );
    assert_eq!(
        store.load_checks(project.id()).unwrap().observations[0].reason_code,
        "check_budget_exceeded"
    );
}
