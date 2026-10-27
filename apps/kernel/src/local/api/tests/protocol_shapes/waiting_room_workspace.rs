use super::*;

#[test]
fn mp02_mp08_owner13_workspace_git_protocol_542() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let target = crate::local::WaitingRoomLaunchTarget {
        repository_state: Some("not-repository".into()),
        workspace_id: "/ordinary".into(),
        worktree_id: "/ordinary".into(),
        workspace_label: None,
        directory: Some("/ordinary".into()),
        worktree_label: None,
    };
    let actual = serde_json::to_value(target).unwrap();
    let expected = serde_json::json!({"repository_state":"not-repository","workspace_id":"/ordinary","worktree_id":"/ordinary","directory":"/ordinary"});
    assert_eq!(actual, expected);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())),
        "af0029e5490136eba195f99a73baad515750e261c5f6b66d74aecb177ce07393"
    );
}

#[test]
fn mp08_owner15_pin_request_shape_542() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let request = LocalDaemonRequest::SetProjectPinned(crate::local::SetProjectPinnedRequest {
        project_id: "project".into(),
        pinned: true,
    });
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({ "SetProjectPinned": {"project_id":"project", "pinned":true} })
    );
}

#[test]
fn mp08_owner19_named_project_selection_protocol_542() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let selection = crate::session::SessionProjectSelection::New {
        name: Some("Release".into()),
    };
    assert_eq!(
        serde_json::to_value(selection).unwrap(),
        serde_json::json!({"kind":"new", "name":"Release"})
    );
    let legacy: crate::session::SessionProjectSelection =
        serde_json::from_value(serde_json::json!({"kind":"new"})).unwrap();
    assert!(matches!(
        legacy,
        crate::session::SessionProjectSelection::New { name: None }
    ));
}

#[test]
fn mp02_mp08_workspace_metadata_and_pin_responses_542_are_hashed() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let wire = serde_json::json!([{"WorkspaceDirectoriesSearched":{"directories":["/ordinary"],"workspace_git":[{"workspace_id":"/ordinary","repository_state":"not-repository","workspace_label":null}]}},{"ProjectPinned":{"project":{"id":"project","owner_user_id":"local","workspace_id":"/ordinary","workspace_ids":["/ordinary"],"name":"Release","pinned":true,"kind":"named","status":"active","created_at_ms":1,"updated_at_ms":2}}}]);
    let responses = wire
        .as_array()
        .unwrap()
        .iter()
        .map(|value| {
            let response: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
            serde_json::to_value(response).unwrap()
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_value(responses).unwrap();
    assert_eq!(encoded, wire);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&encoded).unwrap())
        ),
        "26c6c23ba15393999780f42302efa7532cc190ad3b66a4449e96926b863f1073"
    );
}
