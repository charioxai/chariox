use super::*;
use std::cell::Cell;

#[test]
fn checkpoint_stops_only_the_original_id_after_its_name_is_rebound() {
    let source = "a".repeat(64);
    let unrelated = "b".repeat(64);
    let source_running = Cell::new(true);
    let unrelated_running = Cell::new(true);
    let mut commands = Vec::new();
    with_source_stopped(
        &source,
        |args, _| {
            commands.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
            match args[0] {
                "container" => {
                    assert_eq!(args.last().copied(), Some(source.as_str()));
                    Ok(source_running.get().to_string())
                }
                "exec" => {
                    assert_eq!(args[3], source);
                    Ok(String::new())
                }
                "stop" => {
                    assert_eq!(args.last().copied(), Some(source.as_str()));
                    source_running.set(false);
                    Ok(source.clone())
                }
                _ => panic!("unexpected operation"),
            }
        },
        || {
            assert!(!source_running.get());
            assert!(unrelated_running.get());
            Ok(())
        },
    )
    .unwrap();
    assert!(commands
        .iter()
        .flatten()
        .all(|argument| argument != &unrelated));
    assert_eq!(
        commands
            .iter()
            .filter(|command| command[0] == "stop")
            .count(),
        1
    );
}

#[test]
fn an_invalid_id_or_failed_stop_never_captures_a_checkpoint() {
    let captured = Cell::new(false);
    assert!(with_source_stopped(
        "canonical-name",
        |_, _| panic!("invalid identity executed"),
        || {
            captured.set(true);
            Ok(())
        }
    )
    .is_err());
    for failure in ["stop", "still-running", "malformed-state"] {
        assert!(with_source_stopped(
            &"a".repeat(64),
            |args, _| {
                if args[0] == "stop" && failure == "stop" {
                    return Err(error("injected stop failure"));
                }
                if args[0] == "container" {
                    return Ok(if failure == "malformed-state" {
                        "unknown"
                    } else {
                        "true"
                    }
                    .into());
                }
                Ok(String::new())
            },
            || {
                captured.set(true);
                Ok(())
            }
        )
        .is_err());
        assert!(!captured.get());
    }
}

#[test]
fn an_already_stopped_source_is_not_restarted_or_stopped_by_name() {
    let source = "a".repeat(64);
    let value = with_source_stopped(
        &source,
        |args, _| {
            assert_eq!(
                args,
                [
                    "container",
                    "inspect",
                    "--format",
                    "{{.State.Running}}",
                    source.as_str()
                ]
            );
            Ok("false\n".into())
        },
        || Ok(7),
    )
    .unwrap();
    assert_eq!(value, 7);
}

#[test]
fn archive_cleanup_requires_successful_creation_and_uses_only_its_returned_id() {
    for response in [Err(error("name collision")), Ok("unrelated-name".into())] {
        let mut response = Some(response);
        assert!(with_created_helper(
            &["create", "--name", "generated-name"],
            |args, _| {
                assert_eq!(args[0], "create");
                response
                    .take()
                    .expect("cleanup attempted without a created identity")
            },
            |_, _| Ok(())
        )
        .is_err());
    }
    let id = "a".repeat(64);
    let mut calls = Vec::new();
    assert!(with_created_helper(
        &["create", "--name", "generated-name"],
        |args, _| {
            calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
            Ok(id.clone())
        },
        |created, _| {
            assert_eq!(created, id);
            Err::<(), _>(error("injected capture failure"))
        }
    )
    .is_err());
    assert_eq!(calls.last().unwrap(), &["rm", "--force", id.as_str()]);
}
