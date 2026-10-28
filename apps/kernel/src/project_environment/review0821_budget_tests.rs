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
    let began = Instant::now();
    let result = store.run_check_until(
        context(root.path()),
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
