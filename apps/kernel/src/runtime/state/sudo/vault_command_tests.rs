//! MP-08 / MP-09 / MP-10 / MP-11 VAUDIT S2: Vault capture from a command and
//! use as env/stdin through the real MCP router and real child processes.
//! Live provider acceptance is the separate drill.
use super::tests::{fixture, Fixture};
use super::vault_tests::{call, elevated, names, unlock};
use super::*;
use serde_json::{json, Value};

// The public tool names, pinned literally so these tests also run on the base.
const CAPTURE: &str = "chariox.vault.capture_command";
const USE_ENV: &str = "chariox.vault.use_env";
const CAPTURE_FILE: &str = "chariox.vault.capture_file";
const WRITE_FILE: &str = "chariox.vault.write_file";
const SECRET: &str = "sk_test_51CaptureValue9f8e7d";

fn printing(text: &str) -> Value {
    json!(["/bin/sh", "-c", format!("printf '%s\\n' '{text}'")])
}

async fn capture(
    f: &Fixture,
    request_id: &str,
    argv: Value,
    extract: Value,
) -> Result<Value, DaemonError> {
    call(
        f,
        CAPTURE,
        json!({"request_id": request_id, "argv": argv, "host": "api.example.com",
            "label": "example key", "extract": extract}),
    )
    .await
}

fn stored(f: &Fixture, handle: &str) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(
        f.state
            .home_runtime_secret_service()
            .unwrap()
            .command_secret_input(handle)
            .unwrap(),
    )
}

fn workspace(f: &Fixture) -> std::path::PathBuf {
    f.run.working_directory().unwrap().to_path_buf()
}

#[tokio::test]
async fn s2_ordinary_agents_capture_and_use_without_sudo() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let catalog = names(&f);
    assert!(catalog.iter().any(|name| name == CAPTURE));
    assert!(catalog.iter().any(|name| name == USE_ENV));
    let captured = capture(&f, "key", printing(SECRET), json!({"kind":"stdout"}))
        .await
        .unwrap();
    let handle = captured["credential_id"].as_str().unwrap();
    let used = call(
        &f,
        USE_ENV,
        json!({"origin":"https://api.example.com","argv":["/usr/bin/env"],"env":{"KEY":handle}}),
    )
    .await
    .unwrap();
    assert!(!used.to_string().contains(SECRET));
    assert!(used["stdout"]
        .as_str()
        .unwrap()
        .contains(&format!("KEY=[vault:{handle}]")));
    let marker = workspace(&f).join("must-not-run");
    assert!(call(&f,USE_ENV,json!({"origin":"https://api.example.com","argv":["/usr/bin/touch",marker],"env":{"KEY":"missing"}})).await.is_err());
    assert!(!marker.exists());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn s2_capture_stores_the_printed_value_and_never_returns_it() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let runs = workspace(&f).join("runs");
    // The command counts its runs, so a retry that re-ran it would show.
    let argv = json!([
        "/bin/sh",
        "-c",
        format!(
            "echo run >> '{}'; printf 'created\\ntoken: %s\\n' '{SECRET}'; echo noise >&2",
            runs.display()
        )
    ]);
    let created = capture(
        &f,
        "stripe-key",
        argv.clone(),
        json!({"kind": "regex", "pattern": r"token: (\S+)", "group": 1}),
    )
    .await
    .unwrap();
    let handle = created["credential_id"].as_str().unwrap().to_string();
    assert!(handle.starts_with("cap-"), "{created}");
    assert_eq!(
        created,
        json!({"credential_id": handle, "label": "example key", "host": "api.example.com",
            "length": SECRET.len(), "created": true})
    );
    assert_eq!(stored(&f, &handle).as_str(), SECRET);
    let session =
        serde_json::to_string(&f.state.owned.session_snapshot(&turn.session_id).unwrap()).unwrap();
    assert!(!session.contains(SECRET));
    let retried = capture(&f, "stripe-key", argv, json!({"kind": "stdout"}))
        .await
        .unwrap();
    assert_eq!(retried["credential_id"], handle.as_str());
    assert_eq!(retried["created"], false);
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "run\n");
    let credential = crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .get(&handle)
        .unwrap()
        .unwrap();
    assert_eq!(credential.allowed_hosts, ["api.example.com"]);
    assert_eq!(
        credential.allowed_uses,
        [
            crate::config::UserCredentialUse::Env,
            crate::config::UserCredentialUse::File
        ]
    );
    let metadata = credential.metadata.unwrap();
    assert_eq!(metadata.created_by_kind.as_deref(), Some("vault_capture"));
    assert_eq!(
        metadata.created_by_id.as_deref(),
        Some(turn.agent_id.as_str())
    );
    // JSON field extraction.
    let json_key = capture(
        &f,
        "json-key",
        printing(&format!(r#"{{"data":{{"key":"{SECRET}-json"}}}}"#)),
        json!({"kind": "json", "pointer": "/data/key"}),
    )
    .await
    .unwrap();
    assert_eq!(
        stored(&f, json_key["credential_id"].as_str().unwrap()).as_str(),
        format!("{SECRET}-json")
    );
    // Failures say what went wrong without the output they looked at.
    for (request, argv, extract) in [
        (
            "failed",
            json!(["/bin/sh", "-c", format!("echo {SECRET}; exit 3")]),
            json!({"kind": "stdout"}),
        ),
        (
            "unmatched",
            printing(SECRET),
            json!({"kind": "regex", "pattern": "absent (x)"}),
        ),
        (
            "not-json",
            printing(SECRET),
            json!({"kind": "json", "pointer": "/key"}),
        ),
    ] {
        let message = capture(&f, request, argv, extract)
            .await
            .unwrap_err()
            .to_string();
        assert!(message.contains("nothing was stored"), "{message}");
        assert!(!message.contains(SECRET), "{request}: {message}");
    }
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
    assert!(capture(
        &f,
        "after-revoke",
        printing(SECRET),
        json!({"kind": "stdout"})
    )
    .await
    .is_err());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn s2_use_env_injects_values_and_masks_this_output() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let created = capture(&f, "key", printing(SECRET), json!({"kind": "stdout"}))
        .await
        .unwrap();
    let handle = created["credential_id"].as_str().unwrap().to_string();
    let received = workspace(&f).join("received");
    let arguments = json!({"origin":"https://api.example.com","argv": ["/bin/sh", "-c",
        format!("printf '%s' \"$API_KEY\" > '{}'; echo \"key=$API_KEY\"; cat; echo \"err $API_KEY\" >&2; exit 4", received.display())],
        "env": {"API_KEY": handle}, "stdin": handle});
    assert!(!arguments.to_string().contains(SECRET));
    let used = call(&f, USE_ENV, arguments).await.unwrap();
    // The command really got the value; the model gets only the mask.
    assert_eq!(std::fs::read_to_string(&received).unwrap(), SECRET);
    let mask = format!("[vault:{handle}]");
    assert_eq!(
        used,
        json!({"status": "exited", "exit_code": 4, "stdout": format!("key={mask}\n{mask}"),
            "stderr": format!("err {mask}\n"), "truncated": false})
    );
    let session =
        serde_json::to_string(&f.state.owned.session_snapshot(&turn.session_id).unwrap()).unwrap();
    assert!(!session.contains(SECRET));
    let started = std::time::Instant::now();
    let slow = call(
        &f,
        USE_ENV,
        json!({"origin":"https://api.example.com","argv": ["/bin/sleep", "30"], "env": {"K": handle}, "timeout_seconds": 1}),
    )
    .await
    .unwrap();
    assert_eq!(slow["status"], "timed_out");
    assert!(started.elapsed() < Duration::from_secs(15));
    for refused in [
        json!({"origin":"https://api.example.com","argv": ["/bin/true"]}),
        json!({"origin":"https://api.example.com","argv": ["/bin/true"], "env": {"BAD-NAME": handle}}),
        json!({"origin":"https://api.example.com","argv": ["/bin/true"], "env": {"K": "missing"}}),
        json!({"origin":"https://api.example.com","argv": ["/bin/true"], "env": {"K": handle}, "cwd": "../.."}),
    ] {
        assert!(
            call(&f, USE_ENV, refused.clone()).await.is_err(),
            "{refused}"
        );
    }
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
    std::fs::remove_file(&received).unwrap();
    assert!(call(&f, USE_ENV,
        json!({"origin":"https://api.example.com","argv": ["/bin/sh", "-c", format!("printf x > '{}'", received.display())], "env": {"K": handle}}))
    .await
    .is_err());
    assert!(!received.exists());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn s2_use_env_refuses_credentials_not_made_for_commands() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    unlock(&f);
    // An owner's terminal credential is not an environment credential.
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .upsert_vault_backed_credential_with_secret(
            &crate::credential::CharioxCredentialRegistry::user().unwrap(),
            crate::config::UserCredentialConfig {
                id: "owner-terminal".into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault {
                    key: "owner-terminal".into(),
                },
                allowed_hosts: Vec::new(),
                allowed_uses: vec![crate::config::UserCredentialUse::Pty],
                injection: crate::config::UserCredentialInjectionConfig::Pty,
                metadata: None,
            },
            "s2-owner-terminal-value",
            false,
        )
        .unwrap();
    let marker = workspace(&f).join("must-not-run");
    let refused = call(
        &f,
        USE_ENV,
        json!({"origin":"https://api.example.com","argv": ["/usr/bin/touch", marker], "env": {"K": "owner-terminal"}}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("command input") || refused.contains("not allowed"),
        "{refused}"
    );
    assert!(!marker.exists());
}

#[tokio::test]
async fn s4_capture_file_and_write_file_move_values_through_the_kernel() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let catalog = names(&f);
    assert!(
        catalog.iter().any(|name| name == CAPTURE_FILE),
        "{catalog:?}"
    );
    assert!(catalog.iter().any(|name| name == WRITE_FILE), "{catalog:?}");
    let pulled = workspace(&f).join(".env.local");
    std::fs::write(&pulled, format!("# pulled\nAPI_KEY={SECRET}\nOTHER=1\n")).unwrap();
    let captured = call(
        &f,
        CAPTURE_FILE,
        json!({"request_id": "pulled", "path": ".env.local",
        "host": "api.example.com", "extract": {"kind": "regex", "pattern": "(?m)^API_KEY=(.+)$"},
        "replace_with_handle": true}),
    )
    .await
    .unwrap();
    let handle = captured["credential_id"].as_str().unwrap().to_string();
    assert!(!captured.to_string().contains(SECRET), "{captured}");
    assert_eq!(captured["replaced_with_handle"], true);
    assert_eq!(stored(&f, &handle).as_str(), SECRET);
    assert_eq!(
        std::fs::read_to_string(&pulled).unwrap(),
        format!("# pulled\nAPI_KEY=${{chariox:{handle}}}\nOTHER=1\n")
    );
    // A file capture is usable by commands too.
    let used = call(
        &f,
        USE_ENV,
        json!({"origin":"https://api.example.com","argv": ["/bin/sh", "-c", "printf %s \"$K\""], "env": {"K": handle}}),
    )
    .await
    .unwrap();
    assert_eq!(used["stdout"], format!("[vault:{handle}]"));
    let written = call(
        &f,
        WRITE_FILE,
        json!({"path": "config/.env",
        "template": "# app\nAPI_KEY={{KEY}}\nAGAIN={{KEY}}\n", "values": {"KEY": handle}}),
    )
    .await;
    assert!(written.is_err(), "the directory must exist");
    std::fs::create_dir(workspace(&f).join("config")).unwrap();
    let written = call(
        &f,
        WRITE_FILE,
        json!({"path": "config/.env",
        "template": "# app\nAPI_KEY={{KEY}}\nAGAIN={{KEY}}\n", "values": {"KEY": handle}}),
    )
    .await
    .unwrap();
    assert!(!written.to_string().contains(SECRET), "{written}");
    let target = workspace(&f).join("config/.env");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        format!("# app\nAPI_KEY={SECRET}\nAGAIN={SECRET}\n")
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let session =
        serde_json::to_string(&f.state.owned.session_snapshot(&turn.session_id).unwrap()).unwrap();
    assert!(!session.contains(SECRET));
    for refused in [
        json!({"path": "../outside.env", "template": "K={{K}}", "values": {"K": handle}}),
        json!({"path": "/tmp/outside.env", "template": "K={{K}}", "values": {"K": handle}}),
        json!({"path": "x.env", "template": "no placeholder", "values": {"K": handle}}),
        json!({"path": "x.env", "template": "K={{K}}", "values": {"K": "missing"}}),
        json!({"path": "x.env", "template": "K={{K}}", "values": {"K": handle}, "mode": "rw"}),
    ] {
        assert!(
            call(&f, WRITE_FILE, refused.clone()).await.is_err(),
            "{refused}"
        );
    }
    assert!(!workspace(&f).join("x.env").exists());
    assert!(call(
        &f,
        CAPTURE_FILE,
        json!({"request_id": "escape", "path": "../../etc/hostname",
        "host": "api.example.com"})
    )
    .await
    .is_err());
    f.state
        .revoke_sudo(Some("local"), Some(&turn.entry_id), "explicit_revoke")
        .unwrap();
    assert!(call(
        &f,
        WRITE_FILE,
        json!({"path": "late.env", "template": "K={{K}}", "values": {"K": handle}})
    )
    .await
    .is_err());
    assert!(!workspace(&f).join("late.env").exists());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn ordinary_vault_capture_retains_the_provider_command_mode() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    f.state
        .owned
        .provider_store
        .update_run_execution_config(
            f.run.id(),
            crate::provider::AgentExecutionMode::Plan,
            crate::provider::AgentPermissionLevel::Yolo,
        )
        .unwrap();
    let marker = workspace(&f).join("must-not-execute-in-plan");
    let argv = json!([
        "/bin/sh",
        "-c",
        format!("touch '{}'; printf '%s' '{SECRET}'", marker.display())
    ]);
    let failure = capture(&f, "plan-capture", argv, json!({"kind":"stdout"}))
        .await
        .unwrap_err();
    assert!(failure.to_string().contains("unrestricted build run"));
    assert!(!marker.exists());
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
    assert!(f.state.owned.sudo_turns.lock().unwrap().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}
