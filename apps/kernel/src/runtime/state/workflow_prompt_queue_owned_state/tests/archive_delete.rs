//! Deleting a workflow removes its copies' agents and worktrees, and refuses
//! while a run is in flight.
use super::*;

fn remove_op(workflow_id: &str) -> crate::local::ApplyWorkflowDesignOpRequest {
    crate::local::ApplyWorkflowDesignOpRequest {
        session_id: String::new(),
        origin_client_id: "archive-delete-test".to_string(),
        op_id: "remove".to_string(),
        op: crate::local::WorkflowDesignOp::WorkflowRemove {
            workflow_id: workflow_id.to_string(),
        },
    }
}

fn git_fixture(root: &std::path::Path) {
    std::fs::write(root.join("README.md"), "archive delete fixture\n").expect("fixture file");
    for args in [
        vec!["init"],
        vec!["config", "user.email", "tests@chariox.invalid"],
        vec!["config", "user.name", "Chariox Tests"],
        vec!["add", "README.md"],
        vec!["commit", "-m", "fixture"],
    ] {
        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .expect("git fixture command should run");
        assert!(output.status.success(), "git {args:?} failed");
    }
}

#[test]
fn deleting_a_workflow_removes_its_copies_agents_and_worktrees() {
    let (runtime, session_id, workflow_id, endpoint_id, test_root) = runtime_with_idle_workflow();
    git_fixture(&test_root.0);
    let instance_root = runtime
        .owned
        .config_projection
        .snapshot()
        .workflow_runtime_artifact_root()
        .join("instances")
        .join(&session_id);
    std::fs::create_dir_all(&instance_root).expect("instance root");
    let (_, registered_worktree) =
        crate::git_worktree_placement::prepare_workflow_runtime_workspace(
            &test_root.0,
            &instance_root,
            "prepare archive delete test worktree",
        )
        .expect("runtime worktree should be created");
    let instance_worktree = std::path::PathBuf::from(&registered_worktree);
    let workflow = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("workflow should resolve");
    let source = runtime
        .owned
        .agent_store
        .get_session_agents(&session_id)
        .into_iter()
        .find(|agent| agent.visible_in_freeform())
        .expect("source agent");
    let copy = runtime
        .owned
        .agent_store
        .materialize_workflow_runtime_agent(source.clone(), &session_id, &registered_worktree);
    let node_agent_ids = workflow
        .nodes()
        .iter()
        .map(|node| (node.id().to_string(), copy.id().to_string()))
        .collect::<std::collections::BTreeMap<_, _>>();
    runtime
        .owned
        .session_store
        .write()
        .register_workflow_runtime_instance(
            &session_id,
            crate::session::WorkflowEndpointRuntimeInstance::new(
                "copy-1",
                &workflow_id,
                &endpoint_id,
                workflow.revision(),
                2,
                false,
                node_agent_ids,
                registered_worktree,
            ),
        )
        .expect("copy should register");
    assert!(instance_worktree.exists());

    let mut request = remove_op(&workflow_id);
    request.session_id = session_id.clone();
    runtime
        .owned
        .workflow_apply_design_op(request, "alice", None)
        .expect("an idle workflow deletes");

    let sessions = runtime.owned.session_store.read();
    assert!(sessions
        .resolve_workflow_ref(&session_id, &workflow_id)
        .is_err());
    assert!(sessions
        .get_session(&session_id)
        .expect("session remains")
        .workflow_runtime_instances()
        .is_empty());
    drop(sessions);
    assert!(runtime.owned.agent_store.get_agent(copy.id()).is_err());
    assert!(
        runtime.owned.agent_store.get_agent(source.id()).is_ok(),
        "the source agent belongs to the room, not to the workflow"
    );
    assert!(!instance_worktree.exists());
    let listed = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&test_root.0)
        .output()
        .expect("git worktree list should run");
    assert!(
        !String::from_utf8_lossy(&listed.stdout).contains(&instance_worktree.display().to_string())
    );
}

#[test]
fn deleting_a_workflow_with_a_run_in_flight_names_the_run_and_keeps_everything() {
    let (runtime, session_id, workflow_id, endpoint_id, _test_root) = runtime_with_idle_workflow();
    let run = runtime
        .owned
        .session_store
        .write()
        .invoke_workflow_endpoint(&session_id, &workflow_id, &endpoint_id, None)
        .expect("run should start");

    let mut request = remove_op(&workflow_id);
    request.session_id = session_id.clone();
    let error = runtime
        .owned
        .workflow_apply_design_op(request, "alice", None)
        .expect_err("delete must refuse while the run is in flight");
    assert!(error.to_string().contains(run.id()), "{error}");
    assert!(runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .is_ok());
}

#[test]
fn only_the_room_owner_archives_or_deletes_as_a_person() {
    let (runtime, session_id, workflow_id, _endpoint_id, _test_root) = runtime_with_idle_workflow();
    for op in [
        crate::local::WorkflowDesignOp::WorkflowArchive {
            workflow_id: workflow_id.clone(),
            archived: true,
        },
        crate::local::WorkflowDesignOp::WorkflowRemove {
            workflow_id: workflow_id.clone(),
        },
    ] {
        let error = runtime
            .owned
            .workflow_apply_design_op(
                crate::local::ApplyWorkflowDesignOpRequest {
                    session_id: session_id.clone(),
                    origin_client_id: "guest".to_string(),
                    op_id: "guest-op".to_string(),
                    op,
                },
                "bob",
                None,
            )
            .expect_err("a person who is not the room owner is refused");
        assert!(error.to_string().contains("bob"), "{error}");
    }
    let workflow = runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("workflow is untouched");
    assert!(!workflow.is_archived());
    runtime
        .owned
        .workflow_apply_design_op(
            crate::local::ApplyWorkflowDesignOpRequest {
                session_id: session_id.clone(),
                origin_client_id: "owner".to_string(),
                op_id: "owner-op".to_string(),
                op: crate::local::WorkflowDesignOp::WorkflowArchive {
                    workflow_id: workflow_id.clone(),
                    archived: true,
                },
            },
            "alice",
            None,
        )
        .expect("the room owner archives");
    assert!(runtime
        .owned
        .session_store
        .read()
        .resolve_workflow_ref(&session_id, &workflow_id)
        .expect("archived workflow resolves")
        .is_archived());
}
