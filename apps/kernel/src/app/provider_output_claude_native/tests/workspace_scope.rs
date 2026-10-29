//! MP-08 / MP-09 / MP-10 / MP-11: automatic trust is only for the selected folder.
use super::*;

#[test]
fn startup_trust_cards_keep_the_selected_workspace_scope() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("trust-scope-cards");
    let selected = root.path().join("selected  folder");
    let compacted = root.path().join("selected folder");
    let unicode_space = root.path().join("selected\u{a0} folder");
    let child = selected.join("child");
    let sibling = root.path().join("sibling");
    let home = root.path().join("home");
    for path in [
        &selected,
        &compacted,
        &unicode_space,
        &child,
        &sibling,
        &home,
    ] {
        fs::create_dir_all(path).unwrap();
    }
    let ancestor = root.path().to_path_buf();
    let mut failures = Vec::new();
    for (name, cwd, displayed, warmed, expected) in [
        ("selected arrow", &selected, &selected, false, true),
        ("selected", &selected, &selected, false, true),
        ("ancestor", &selected, &ancestor, false, false),
        ("child", &selected, &child, false, false),
        ("sibling", &selected, &sibling, false, false),
        ("changed cwd", &sibling, &selected, false, false),
        (
            "unicode path spaces",
            &selected,
            &unicode_space,
            false,
            false,
        ),
        (
            "different literal spaces",
            &selected,
            &compacted,
            false,
            false,
        ),
        ("ancestor Git", &selected, &selected, false, false),
        ("worktree main checkout", &selected, &selected, false, false),
        ("selected Git root", &selected, &selected, false, true),
        ("cursor gaps", &selected, &selected, false, true),
        ("home", &selected, &selected, false, false),
        ("warm run", &selected, &selected, true, false),
    ] {
        match name {
            "ancestor Git" => fs::create_dir(root.path().join(".git")).unwrap(),
            "selected Git root" => fs::create_dir(selected.join(".git")).unwrap(),
            "worktree main checkout" => fs::write(
                selected.join(".git"),
                "gitdir: /unchosen/main/.git/worktrees/selected",
            )
            .unwrap(),
            _ => {}
        }
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                selected.to_str().unwrap(),
                selected.to_str().unwrap(),
            ))
            .unwrap();
        let native = root
            .path()
            .join(format!("native-{}", name.replace(' ', "_")));
        fs::create_dir(&native).unwrap();
        let context = native.join("context.txt");
        let events = native.join("events.jsonl");
        fs::write(&context, "").unwrap();
        fs::write(&events, "").unwrap();
        let mut run = serde_json::to_value(startup_readiness_run(
            session.id(),
            agent.id(),
            "trust-scope-run",
            &context,
            &events,
            format!(
                "tee '{}' >/dev/null",
                native
                    .join("input.log")
                    .display()
                    .to_string()
                    .replace('\'', "'\\''")
            ),
        ))
        .unwrap();
        run["working_directory"] = serde_json::to_value(cwd).unwrap();
        run["pty_env"]["HOME"] =
            serde_json::to_value(if name == "home" { &selected } else { &home }).unwrap();
        let mut run: RuntimeProviderRun = serde_json::from_value(run).unwrap();
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        if name.starts_with("selected") && !name.contains("Git") {
            app.pty.spawn_for_run(&run).unwrap();
        }
        if warmed {
            write_claude_native_marker(context.to_str().unwrap(), "accepted:previous-prompt");
            write_claude_native_marker(context.to_str().unwrap(), "");
        }
        let bridge = StartupTrustBridge::default();
        let mut frame = format!(
            "Accessing workspace:\n{}\nQuick safety check\nDo you trust this folder?\n❯ 1. Yes, I trust this folder\n2. No, exit",
            displayed.display(),
        );
        if name == "selected arrow" {
            frame = frame.replace(
                "❯ 1. Yes, I trust this folder\n2. No, exit",
                "❯ 1. No, exit\n2. Yes, I trust this folder",
            );
        }
        if name == "cursor gaps" {
            frame = frame
                .replace("Accessing workspace:", "Accessing\x1b[12Gworkspace:")
                .replace("Quick safety check", "Quick\x1b[7Gsafety\x1b[14Gcheck");
        }
        ProviderOutputClaudeNativeBridge::new(&mut app)
            .process_terminal_output(
                session.id(),
                run.id(),
                &run,
                Some(std::sync::Arc::new(bridge.clone())),
                &frame,
            )
            .unwrap();
        let interaction = bridge.wait_for_interaction();
        let reusable = interaction
            .id()
            .starts_with("claude-selected-workspace-trust-");
        if reusable != expected {
            failures.push(name);
        }
        if name.starts_with("selected") && !name.contains("Git") {
            if name == "selected arrow" {
                bridge.resolve("allow_once", "allow");
                for _ in 0..500 {
                    ProviderOutputClaudeNativeBridge::new(&mut app)
                        .process_pending_claude_workspace_trust(run.id(), context.to_str().unwrap())
                        .unwrap();
                    if claude_workspace_trust_selection_started_at(context.to_str().unwrap())
                        .is_some()
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            let changed = frame.replace(
                &selected.display().to_string(),
                &ancestor.display().to_string(),
            );
            ProviderOutputClaudeNativeBridge::new(&mut app)
                .process_terminal_output(
                    session.id(),
                    run.id(),
                    &run,
                    Some(std::sync::Arc::new(bridge.clone())),
                    &changed,
                )
                .unwrap();
            if name == "selected" {
                bridge.resolve("allow_once", "allow");
            }
            let result = (|| {
                for _ in 0..500 {
                    match ProviderOutputClaudeNativeBridge::new(&mut app)
                        .process_pending_claude_workspace_trust(run.id(), context.to_str().unwrap())
                    {
                        Err(error) => return Some(error.to_string()),
                        Ok(Some(_)) => return None,
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(2)),
                    }
                }
                None
            })();
            std::thread::sleep(std::time::Duration::from_millis(30));
            if !result.is_some_and(|message| message.contains("selected workspace"))
                || fs::read(native.join("input.log"))
                    .unwrap_or_default()
                    .contains(&b'\n')
            {
                failures.push("target changed while startup decision was pending");
            }
        } else {
            bridge.resolve("deny", "deny");
        }
        app.shutdown_cleanup().unwrap();
        match name {
            "ancestor Git" => fs::remove_dir(root.path().join(".git")).unwrap(),
            "selected Git root" => fs::remove_dir(selected.join(".git")).unwrap(),
            "worktree main checkout" => fs::remove_file(selected.join(".git")).unwrap(),
            _ => {}
        }
    }
    assert!(
        failures.is_empty(),
        "startup trust escaped its selected scope: {failures:?}"
    );
}

#[test]
fn invalid_startup_trust_selection_has_an_actionable_failure() {
    let root = crate::test_support::TestWorktree::new("invalid-trust-selection");
    let context = root.path().join("context.txt");
    fs::write(&context, "").unwrap();
    write_claude_native_marker(
        context.to_str().unwrap(),
        "startup-workspace-trust:select-yes:123",
    );
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let error = ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_pending_claude_workspace_trust("run", context.to_str().unwrap())
        .err()
        .expect("invalid selection state must settle with a reason");
    assert!(error.to_string().contains("selection state is invalid"));
    app.shutdown_cleanup().unwrap();
}
