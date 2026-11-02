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
async fn s2_ordinary_agents_cannot_capture_or_use() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let catalog = names(&f);
    // Stable discovery is not a grant: dispatch checks the live window.
    assert!(catalog.iter().any(|name| name == CAPTURE), "{catalog:?}");
    assert!(catalog.iter().any(|name| name == USE_ENV), "{catalog:?}");
    let refused = capture(&f, "key", printing(SECRET), json!({"kind": "stdout"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("sudo"), "{refused}");
    let marker = workspace(&f).join("must-not-run");
    assert!(call(
        &f,
        USE_ENV,
        json!({"argv": ["/usr/bin/touch", marker], "env": {"KEY": "any"}}),
    )
    .await
    .is_err());
    assert!(!marker.exists());
    assert!(crate::credential::load_user_credentials()
        .unwrap()
        .is_empty());
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
        [crate::config::UserCredentialUse::Env]
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
    let arguments = json!({"argv": ["/bin/sh", "-c",
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
        json!({"argv": ["/bin/sleep", "30"], "env": {"K": handle}, "timeout_seconds": 1}),
    )
    .await
    .unwrap();
    assert_eq!(slow["status"], "timed_out");
    assert!(started.elapsed() < Duration::from_secs(15));
    for refused in [
        json!({"argv": ["/bin/true"]}),
        json!({"argv": ["/bin/true"], "env": {"BAD-NAME": handle}}),
        json!({"argv": ["/bin/true"], "env": {"K": "missing"}}),
        json!({"argv": ["/bin/true"], "env": {"K": handle}, "cwd": "../.."}),
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
        json!({"argv": ["/bin/sh", "-c", format!("printf x > '{}'", received.display())], "env": {"K": handle}}))
    .await
    .is_err());
    assert!(!received.exists());
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
        json!({"argv": ["/usr/bin/touch", marker], "env": {"K": "owner-terminal"}}),
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
