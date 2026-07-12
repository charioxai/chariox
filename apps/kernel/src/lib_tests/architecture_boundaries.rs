use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[test]
fn transport_output_pump_is_ready_run_driven() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime/state/transport_runtime_state.rs"),
    )
    .expect("transport runtime source should be readable");
    let pump = source
        .split("pub(crate) async fn pump_transport_runtime")
        .nth(1)
        .and_then(|tail| {
            tail.split("pub(crate) async fn record_terminal_attachment_heartbeat")
                .next()
        })
        .expect("transport output pump should remain discoverable");
    for forbidden in [
        "pump_active_prompt_outputs",
        "list_sessions",
        "list_non_ended_sessions_including_hidden",
    ] {
        assert!(
            !pump.contains(forbidden),
            "transport output pump must consume ready-run identities, not `{forbidden}`"
        );
    }
    assert!(pump.contains("take_ready_provider_run_ids"));
    assert!(pump.contains("take_due_provider_run_ids"));
}

#[test]
fn provider_run_actors_use_async_mailboxes_and_bounded_blocking_work() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider/run_actor/worker.rs"),
    )
    .expect("provider run actor worker source should be readable");
    for forbidden in ["thread::spawn", "mpsc::sync_channel"] {
        assert!(
            !source.contains(forbidden),
            "provider run actors must not allocate a native worker thread per run via `{forbidden}`"
        );
    }
    assert!(source.contains("tokio_mpsc::channel"));
    assert!(source.contains("spawn_blocking"));
    assert!(source.contains("blocking_executor_permits"));
}

#[test]
fn remote_provider_launches_preserve_explicit_adapter_selection() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for relative in [
        "runtime/state/provider_launch_runtime.rs",
        "local/provider_requests.rs",
    ] {
        let source = std::fs::read_to_string(src_root.join(relative))
            .expect("remote provider launch source should be readable");
        assert!(
            source.contains("adapter_key_for_provider(&request.adapter_key)"),
            "{relative} must forward the requested adapter independently from provider identity"
        );
        assert!(
            !source.contains("adapter_key_for_provider(&request.provider)"),
            "{relative} must not replace an explicit remote adapter with provider identity"
        );
    }
    let peer_dispatch =
        std::fs::read_to_string(src_root.join("transport/relay_client/peer_requests.rs"))
            .expect("relay peer dispatch source should be readable");
    assert!(peer_dispatch.contains("adapter_key,"));
    assert!(!peer_dispatch.contains("adapter_key: _,"));
    assert!(!peer_dispatch.contains("adapter_key_for_provider(&provider)"));
}

#[test]
fn runtime_command_paths_do_not_lock_daemon_app() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = BTreeSet::new();
    for root in [
        src_root.join("runtime"),
        src_root.join("runtime_transport"),
        src_root.join("transport/relay_client"),
    ] {
        paths.extend(rust_files(&root));
    }
    for path in [
        src_root.join("runtime_transport.rs"),
        src_root.join("transport/relay_client.rs"),
    ] {
        if path.exists() {
            paths.insert(path);
        }
    }

    let mut violations = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(&src_root)
            .expect("kernel source should live under src")
            .to_path_buf();
        if !scan_runtime_command_path(&relative) {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("runtime source should be readable");
        let production_source = strip_cfg_test_items(&source);
        for pattern in [
            "app.lock().await",
            "app.lock().await;",
            "lock_app_instrumented(",
        ] {
            for (line_index, line) in production_source.lines().enumerate() {
                if line.contains(pattern) {
                    if allowed_direct_app_lock(&relative, line) {
                        continue;
                    }
                    violations.push(format!(
                        "{}:{}: {}",
                        relative.display(),
                        line_index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    violations.sort();
    violations.dedup();
    assert!(
        violations.is_empty(),
        "runtime command paths must route app mutations through KernelRuntimeState/owned stores, not direct app.lock().await:\n{}",
        violations.join("\n")
    );
}

fn scan_runtime_command_path(relative: &Path) -> bool {
    if relative.components().any(|component| {
        let Some(component) = component.as_os_str().to_str() else {
            return false;
        };
        component == "tests" || component.ends_with("_tests")
    }) {
        return false;
    }
    if relative
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .is_some_and(|file_name| file_name == "tests.rs" || file_name.ends_with("_tests.rs"))
    {
        return false;
    }
    if relative == Path::new("runtime/router/composition.rs") {
        return false;
    }
    relative
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("rs")
}

fn allowed_direct_app_lock(relative: &Path, line: &str) -> bool {
    relative == Path::new("runtime/state/mod.rs")
        && line
            .trim()
            .contains("lock_app_instrumented(&self.app, \"kernel_runtime_state\")")
        || relative == Path::new("runtime/app_lock.rs")
        || matches!(
            relative.to_str(),
            Some(
                "runtime/external_provider_session_control.rs"
                    | "runtime/router/meta_runtime_command.rs"
                    | "runtime/waiting_room_control.rs"
            )
        )
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn strip_cfg_test_items(source: &str) -> String {
    let mut stripped = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(attribute_offset) = source[cursor..].find("#[cfg(test)]") {
        let attribute_start = cursor + attribute_offset;
        let after_attribute = attribute_start + "#[cfg(test)]".len();
        let item_start = skip_attributes_and_whitespace(source, after_attribute);
        let Some(item_end) = cfg_item_end(source, item_start) else {
            break;
        };
        stripped.push_str(&source[cursor..attribute_start]);
        for _ in 0..source[attribute_start..=item_end]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
        {
            stripped.push('\n');
        }
        cursor = item_end + 1;
    }
    stripped.push_str(&source[cursor..]);
    stripped
}

fn skip_attributes_and_whitespace(source: &str, mut cursor: usize) -> usize {
    loop {
        cursor += source[cursor..]
            .bytes()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        if source[cursor..].starts_with("#[") {
            let Some(close_attribute) = source[cursor..].find(']') else {
                return cursor;
            };
            cursor += close_attribute + 1;
            continue;
        }
        return cursor;
    }
}

fn cfg_item_end(source: &str, item_start: usize) -> Option<usize> {
    let next_brace = source[item_start..]
        .find('{')
        .map(|offset| item_start + offset);
    let next_semicolon = source[item_start..]
        .find(';')
        .map(|offset| item_start + offset);
    match (next_brace, next_semicolon) {
        (Some(open_brace), Some(semicolon)) if semicolon < open_brace => Some(semicolon),
        (Some(open_brace), _) => matching_brace(source, open_brace),
        (None, Some(semicolon)) => Some(semicolon),
        (None, None) => None,
    }
}

fn matching_brace(source: &str, open_brace: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in source.as_bytes()[open_brace..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open_brace + offset);
                }
            }
            _ => {}
        }
    }
    None
}
