//! MP-08 / MP-09 / MP-10 / MP-11: automatic trust is only for the selected folder.
use super::*;

fn replace_current_run(app: &mut DaemonApp, run: &RuntimeProviderRun) {
    let current = app.providers().get_run(run.id()).unwrap();
    let mut replacement = serde_json::to_value(current).unwrap();
    replacement["id"] = serde_json::json!("replacement");
    for field in ["started_at_ms", "last_activity_at_ms"] {
        replacement[field] = serde_json::json!(replacement[field].as_u64().unwrap() + 1);
    }
    app.providers_mut()
        .insert_run_for_test(serde_json::from_value(replacement).unwrap());
}

#[test]
fn startup_trust_cards_keep_the_selected_workspace_scope() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("trust-scope-cards");
    let selected = root.path().join("selected  folder");
    let compacted = root.path().join("selected folder");
    let unicode_space = root.path().join("selected\u{a0} folder");
    let trailing_space = root.path().join("selected  folder ");
    let trailing_unicode = root.path().join("selected  folder\u{a0}");
    let child = selected.join("child");
    let sibling = root.path().join("sibling");
    let home = root.path().join("home");
    for path in [
        &selected,
        &compacted,
        &unicode_space,
        &trailing_space,
        &trailing_unicode,
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
        ("selected replacement", &selected, &selected, false, true),
        ("replaced run", &selected, &selected, false, false),
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
        (
            "trailing path space",
            &selected,
            &trailing_space,
            false,
            false,
        ),
        (
            "trailing Unicode space",
            &selected,
            &trailing_unicode,
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
        if name == "replaced run" {
            replace_current_run(&mut app, &run);
        }
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
            let changed = if name == "selected replacement" {
                replace_current_run(&mut app, &run);
                frame.clone()
            } else {
                frame.replace(
                    &selected.display().to_string(),
                    &ancestor.display().to_string(),
                )
            };
            ProviderOutputClaudeNativeBridge::new(&mut app)
                .process_terminal_output(
                    session.id(),
                    run.id(),
                    &run,
                    Some(std::sync::Arc::new(bridge.clone())),
                    &changed,
                )
                .unwrap();
            if name != "selected arrow" {
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
        .expect_err("invalid selection state must settle with a reason");
    assert!(error.to_string().contains("selection state is invalid"));
    app.shutdown_cleanup().unwrap();
}

/// Raw PTY frames use CRLF, may pad headings with blank lines, and the
/// selected folder may itself end in a literal space.
#[test]
fn startup_trust_survives_terminal_line_boundaries_around_the_path() {
    crate::test_support::isolated_env_test!();
    let failures = startup_trust_failures("one drain", &|frame, _| vec![frame.to_string()]);
    assert!(
        failures.is_empty(),
        "terminal line boundaries changed the selected workspace: {failures:?}"
    );
}

/// The runtime passes whatever bytes are available per drain, so one frame
/// may be cut right before, inside or after the path's terminating CRLF, with
/// control-only drains in between. A drain edge is not a terminal boundary.
#[test]
fn startup_trust_survives_pty_drain_boundaries_around_the_path() {
    crate::test_support::isolated_env_test!();
    // `skip` is the frame text the extra drains replace.
    let cut = |frame: &str, path: &str, offset: isize, middle: &[&str], skip: usize| {
        let end = frame.find(path).expect("frame shows the path") + path.len();
        let at = end.checked_add_signed(offset).expect("cut inside frame");
        let mut drains = vec![frame[..at].to_string()];
        drains.extend(middle.iter().map(|chunk| chunk.to_string()));
        drains.push(frame[at + skip..].to_string());
        drains
    };
    let mut failures = Vec::new();
    for (name, offset, middle, skip) in [
        ("before CRLF", 0, &[][..], 0),
        ("between CR and LF", 1, &[][..], 0),
        ("after CRLF", 2, &[][..], 0),
        ("control-only CRLF", 0, &["\r\n"][..], 2),
        ("control-only CR then LF", 0, &["\r", "\n"][..], 2),
        ("control-only cursor move", 0, &["\x1b[1B"][..], 0),
    ] {
        failures.extend(startup_trust_failures(name, &|frame, path| {
            cut(frame, path, offset, middle, skip)
        }));
    }
    // A literal trailing space arrives as its own drain before the line break.
    failures.extend(startup_trust_failures(
        "space then CRLF",
        &|frame, path| match path.strip_suffix(' ') {
            Some(_) => {
                let end = frame.find(path).unwrap() + path.len();
                vec![
                    frame[..end - 1].to_string(),
                    " ".to_string(),
                    frame[end..].to_string(),
                ]
            }
            None => vec![frame.to_string()],
        },
    ));
    assert!(
        failures.is_empty(),
        "a PTY drain boundary changed the selected workspace: {failures:?}"
    );
}

fn startup_trust_failures(
    label: &str,
    drains_of: &dyn Fn(&str, &str) -> Vec<String>,
) -> Vec<String> {
    let root = crate::test_support::TestWorktree::new("trust-scope-crlf");
    let home = root.path().join("home");
    let plain = root.path().join("selected");
    let trailing = root.path().join("selected ");
    for path in [&home, &plain, &trailing] {
        fs::create_dir_all(path).unwrap();
    }
    let crlf = |displayed: &std::path::Path, blank: &str, yes_first: bool| {
        let choices = if yes_first {
            "❯ 1. Yes, I trust this folder\r\n2. No, exit"
        } else {
            "❯ 1. No, exit\r\n2. Yes, I trust this folder"
        };
        format!(
            "Accessing workspace:\r\n{blank}{}\r\n{blank}Quick safety check\r\nDo you trust this folder?\r\n{choices}",
            displayed.display(),
        )
    };
    let mut failures = Vec::new();
    for (name, selected, displayed, blank, expected) in [
        ("crlf", &plain, &plain, "", true),
        ("blank lines", &plain, &plain, "\r\n\r\n", true),
        ("literal trailing space", &trailing, &trailing, "", true),
        (
            "trailing space with blank lines",
            &trailing,
            &trailing,
            "\r\n\r\n",
            true,
        ),
        ("dropped trailing space", &trailing, &plain, "\r\n", false),
        ("added trailing space", &plain, &trailing, "\r\n", false),
    ] {
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
            "trust-crlf-run",
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
        run["working_directory"] = serde_json::to_value(selected).unwrap();
        run["pty_env"]["HOME"] = serde_json::to_value(&home).unwrap();
        let mut run: RuntimeProviderRun = serde_json::from_value(run).unwrap();
        run.mark_running();
        app.providers_mut().insert_run_for_test(run.clone());
        app.pty.spawn_for_run(&run).unwrap();
        let bridge = StartupTrustBridge::default();
        let feed = |app: &mut DaemonApp, frame: &str| {
            for drain in drains_of(frame, &displayed.display().to_string()) {
                ProviderOutputClaudeNativeBridge::new(app)
                    .process_terminal_output(
                        session.id(),
                        run.id(),
                        &run,
                        Some(std::sync::Arc::new(bridge.clone())),
                        &drain,
                    )
                    .unwrap();
            }
        };
        feed(&mut app, &crlf(displayed, blank, false));
        let interaction = bridge.wait_for_interaction();
        let reusable = interaction
            .id()
            .starts_with("claude-selected-workspace-trust-");
        if reusable != expected {
            failures.push(format!("{label}/{name}: reusable={reusable}"));
        }
        if reusable {
            bridge.resolve("allow_once", "allow");
            let context_file = context.to_str().unwrap();
            let pump = |app: &mut DaemonApp| {
                ProviderOutputClaudeNativeBridge::new(app)
                    .process_pending_claude_workspace_trust(run.id(), context_file)
            };
            for _ in 0..500 {
                pump(&mut app).unwrap();
                if claude_workspace_trust_selection_started_at(context_file).is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            // The repaint after the arrow key must still prove the same folder.
            feed(&mut app, &crlf(displayed, blank, true));
            let mut approved = Err("selection was not confirmed".to_string());
            for _ in 0..500 {
                match pump(&mut app) {
                    Err(error) => {
                        approved = Err(error.to_string());
                        break;
                    }
                    Ok(Some(_)) => {
                        approved = Ok(());
                        break;
                    }
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(2)),
                }
            }
            if let Err(error) = approved {
                failures.push(format!("{label}/{name}: revalidation failed: {error}"));
            }
        } else {
            bridge.resolve("deny", "deny");
        }
        app.shutdown_cleanup().unwrap();
    }
    failures
}
