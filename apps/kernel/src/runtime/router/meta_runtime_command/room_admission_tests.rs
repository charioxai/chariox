// MP-08 / MP-11, A01 focused policy regressions; same tests run against base.

fn agent(id: &str, parent: Option<&str>) -> crate::agent::AgentInstance {
    let mut a = crate::agent::AgentInstance::new(
        id,
        format!("ref-{id}"),
        "room",
        Some(id.into()),
        "dev-stub",
        None,
        None,
        None,
        crate::agent::GridPosition::new(0, 0, 1, 1),
    );
    a.set_parent_agent(parent.map(crate::agent::AgentRef::local));
    a
}

fn session() -> crate::session::RuntimeSession {
    crate::session::RuntimeSession::new("room", None, "workspace", "worktree", "machine", "kernel")
}

#[test]
fn room_admission_child_is_controlled_by_its_parent() {
    let a = agent("a", None);
    let b = agent("b", Some("a"));
    let result = super::request::meta_agent_request(
        &session(),
        &a,
        &["alias".into(), "b".into(), "new-name".into()],
        &[a.clone(), b],
        true,
    );
    assert!(result.is_ok(), "A01-S02: the parent controls its child");
}

#[test]
fn room_admission_only_the_direct_parent_deletes_a_grandchild() {
    let a = agent("a", None);
    let b = agent("b", Some("a"));
    let d = agent("d", Some("b"));
    let agents = [a.clone(), b.clone(), d];
    let delete = |actor: &crate::agent::AgentInstance| {
        super::request::meta_agent_request(
            &session(),
            actor,
            &["delete".into(), "d".into()],
            &agents,
            true,
        )
    };
    assert!(
        delete(&a).is_err(),
        "a rename or delete needs a direct child, not a deeper subtree member"
    );
    assert!(delete(&b).is_ok(), "the direct parent deletes its child");
}

#[test]
fn room_admission_sibling_peer_and_ancestor_are_outside_the_subtree() {
    let a = agent("a", None);
    let b = agent("b", Some("a"));
    let sibling = agent("s", Some("a"));
    let peer = agent("p", None);
    let agents = [a.clone(), b.clone(), sibling.clone(), peer.clone()];
    for (actor, target) in [(&b, "s"), (&a, "p"), (&b, "a"), (&b, "b")] {
        let result = super::request::meta_agent_request(
            &session(),
            actor,
            &["delete".into(), target.into()],
            &agents,
            true,
        );
        assert!(result.is_err(), "{} must not control {target}", actor.id());
    }
}

#[test]
fn room_admission_a_parent_cycle_ends_the_walk() {
    let a = agent("a", Some("b"));
    let b = agent("b", Some("a"));
    let outsider = agent("o", None);
    assert!(
        crate::runtime::room_tool_admission::in_subtree(&outsider, &a, &[a.clone(), b]).is_err(),
        "a corrupt cycle must end, not loop"
    );
}

#[test]
fn room_admission_peer_workflow_node_binding_needs_no_controller() {
    let a = agent("a", None);
    let p = agent("peer", None);
    let result = super::request::meta_workflow_request(
        &session(),
        &a,
        &["node".into(), "add".into(), "flow".into(), "peer".into()],
        &[p],
        true,
    );
    assert!(
        result.is_ok(),
        "A01 regular room peer may be bound to a workflow"
    );
}

#[test]
fn room_admission_admits_the_environment_commands_and_no_other() {
    let admitted = |line: &str| {
        let tokens = crate::runtime::metaagent_command_registry::tokenize_command(line).unwrap();
        crate::runtime::room_tool_admission::command(&tokens).is_ok()
    };
    for line in [
        "environment status",
        "environment update",
        "environment update from builder",
        "environment setup",
        "environment check",
    ] {
        assert!(admitted(line), "{line}");
    }
    assert!(!admitted("environment drop"));
    assert!(!admitted("credential list"));
}

#[test]
fn spawn_flags_carry_the_free_choices_and_the_bounded_ones() {
    let args = |line: &str| -> Vec<String> { line.split_whitespace().map(str::to_owned).collect() };
    let spawn = super::spawn_args::parse_meta_agent_spawn_args(
        &args("w1 --account-profile team-b --permission required --deadline 90m --command-id c-1"),
        &session(),
    )
    .unwrap();
    // The account profile is a choice, not an authority limit.
    assert_eq!(spawn.account_profile.as_deref(), Some("team-b"));
    assert_eq!(
        spawn.permission_level,
        Some(crate::provider::AgentPermissionLevel::Required)
    );
    assert_eq!(
        spawn.deadline,
        Some(std::time::Duration::from_secs(90 * 60))
    );
    assert_eq!(spawn.command_id.as_deref(), Some("c-1"));
    for bad in [
        "w1 --permission root",
        "w1 --deadline soon",
        "w1 --deadline 0m",
        "w1 --command-id",
    ] {
        assert!(
            super::spawn_args::parse_meta_agent_spawn_args(&args(bad), &session()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn deadline_values_are_checked_by_both_commands_without_panicking() {
    let parent = agent("p", None);
    let child = agent("c", Some("p"));
    let agents = [parent.clone(), child];
    let set = |value: &str| {
        super::request::meta_agent_request(
            &session(),
            &parent,
            &["deadline".into(), "c".into(), value.into()],
            &agents,
            true,
        )
    };
    let spawn = |value: &str| {
        super::spawn_args::parse_meta_agent_spawn_args(
            &["w1".into(), "--deadline".into(), value.into()],
            &session(),
        )
        .map(|spawn| spawn.deadline)
    };
    for bad in ["30秒", "30ｍ", "", "0s", "m", "99999999999999999999d", "1é"] {
        let duration_error = |result: Result<(), crate::error::DaemonError>| {
            matches!(
                result,
                Err(crate::error::DaemonError::LocalTransport { ref message, .. })
                    if message.starts_with("a deadline is a duration")
            )
        };
        assert!(duration_error(set(bad).map(|_| ())), "set {bad:?}");
        assert!(duration_error(spawn(bad).map(|_| ())), "spawn {bad:?}");
    }
    for (good, seconds) in [("90s", 90), ("30m", 1_800), ("2h", 7_200), ("1d", 86_400)] {
        assert!(set(good).is_ok(), "set {good}");
        assert_eq!(
            spawn(good).unwrap(),
            Some(std::time::Duration::from_secs(seconds)),
            "spawn {good}"
        );
    }
}
