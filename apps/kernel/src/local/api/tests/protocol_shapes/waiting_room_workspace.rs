use super::*;

#[test]
fn mp02_mp08_owner13_workspace_git_protocol_542() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 542);
    let target = crate::local::WaitingRoomLaunchTarget {
        repository_state: Some("not-repository".into()), workspace_id: "/ordinary".into(),
        worktree_id: "/ordinary".into(), workspace_label: None, directory: Some("/ordinary".into()), worktree_label: None,
    };
    let actual = serde_json::to_value(target).unwrap();
    let expected = serde_json::json!({"repository_state":"not-repository","workspace_id":"/ordinary","worktree_id":"/ordinary","directory":"/ordinary"});
    assert_eq!(actual, expected);
    assert_eq!(format!("{:x}", Sha256::digest(serde_json::to_vec(&actual).unwrap())), "af0029e5490136eba195f99a73baad515750e261c5f6b66d74aecb177ce07393");
}

#[test]
fn mp08_owner15_pin_request_shape_542() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 542);
    let request = LocalDaemonRequest::SetProjectPinned(crate::local::SetProjectPinnedRequest { project_id: "project".into(), pinned: true });
    assert_eq!(serde_json::to_value(request).unwrap(), serde_json::json!({ "SetProjectPinned": {"project_id":"project", "pinned":true} }));
}
