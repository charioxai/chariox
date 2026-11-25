//! Row 556: `chariox.vault.generate` binds the generated value to one command use (environment,
//! stdin or file) directly, through the real MCP router and real child processes. RED on main:
//! `use_binding` is an unknown field there ("invalid Vault generation arguments").
use super::tests::{fixture, Fixture};
use super::vault_tests::{call, elevated, unlock};
use super::*;
use serde_json::{json, Value};

const GENERATE: &str = "chariox.vault.generate";
const USE_ENV: &str = "chariox.vault.use_env";
const WRITE_FILE: &str = "chariox.vault.write_file";
const ORIGIN: &str = "https://hooks.example.com";

fn env_binding(name: &str) -> Value {
    json!({"target":{"kind":"command_environment","name":name,"origin":ORIGIN}})
}

fn stored(f: &Fixture, handle: &str) -> zeroize::Zeroizing<String> {
    let service = f.state.home_runtime_secret_service().unwrap();
    let target = service
        .credential(handle)
        .unwrap()
        .metadata
        .as_ref()
        .unwrap()
        .use_binding
        .as_ref()
        .unwrap()
        .target
        .clone();
    zeroize::Zeroizing::new(
        service
            .workspace_secret_input_for_target(handle, &target)
            .unwrap(),
    )
}

async fn generate(f: &Fixture, request_id: &str, extra: Value) -> Result<Value, DaemonError> {
    let mut arguments = json!({"request_id": request_id});
    for (key, value) in extra.as_object().unwrap() {
        arguments[key] = value.clone();
    }
    call(f, GENERATE, arguments).await
}

/// The cell for row 556: a 32-byte (64 hex) secret bound to command env WEBHOOK_SECRET; a command
/// reads it through use_env; the value is in no result the model sees (canary).
#[tokio::test]
async fn generate_binds_a_random_value_to_a_command_environment() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    let turn = elevated(&f);
    unlock(&f);
    let created = generate(
        &f,
        "webhook",
        json!({"use_binding": env_binding("WEBHOOK_SECRET"), "format":"hex", "length":64,
            "description":"webhook secret"}),
    )
    .await
    .unwrap();
    let handle = created["credential_id"].as_str().unwrap().to_string();
    assert!(handle.starts_with("gen-"));
    assert_eq!(created["created"], true);
    assert_eq!(
        created["use_binding"],
        json!({"target":{"kind":"command_environment","name":"WEBHOOK_SECRET","origin":ORIGIN},"value_kind":"secret"})
    );
    let secret = stored(&f, &handle);
    assert_eq!(secret.len(), 64);
    assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(!created.to_string().contains(secret.as_str()));
    let metadata = crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .get(&handle)
        .unwrap()
        .unwrap();
    assert_eq!(metadata.allowed_uses, vec![crate::config::UserCredentialUse::Env]);
    assert_eq!(
        metadata.metadata.as_ref().unwrap().created_by_kind.as_deref(),
        Some("vault_generate")
    );
    assert_eq!(
        metadata.metadata.unwrap().created_by_id.as_deref(),
        Some(turn.agent_id.as_str())
    );
    // A command reads it; the reply masks it.
    let used = call(
        &f,
        USE_ENV,
        json!({"origin":ORIGIN,"argv":["/bin/sh","-c","printf '%s' \"$WEBHOOK_SECRET\" | wc -c; echo \"$WEBHOOK_SECRET\""],
            "env":{"WEBHOOK_SECRET":handle}}),
    )
    .await
    .unwrap();
    assert!(!used.to_string().contains(secret.as_str()), "{used}");
    assert!(used["stdout"].as_str().unwrap().contains(&format!("[vault:{handle}]")));
    assert!(used["stdout"].as_str().unwrap().trim_start().starts_with("64"));
    // Never in the session the model reads.
    let session =
        serde_json::to_string(&f.state.owned.session_snapshot(&turn.session_id).unwrap()).unwrap();
    assert!(!session.contains(secret.as_str()));
}

#[tokio::test]
async fn generated_command_value_is_usable_only_at_its_binding_and_is_no_site_password() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    unlock(&f);
    let handle = generate(&f, "one-use", json!({"use_binding": env_binding("DB_SECRET")}))
        .await
        .unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    // Another variable name, another origin, stdin, and a browser fill are all refused.
    for refused in [
        json!({"origin":ORIGIN,"argv":["/bin/true"],"env":{"OTHER_NAME":handle}}),
        json!({"origin":"https://elsewhere.example.com","argv":["/bin/true"],"env":{"DB_SECRET":handle}}),
        json!({"origin":ORIGIN,"argv":["/bin/true"],"stdin":handle}),
    ] {
        assert!(call(&f, USE_ENV, refused.clone()).await.is_err(), "{refused}");
    }
    assert!(f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .browser_secret_input_for_target_url(&handle, "https://hooks.example.com/login")
        .is_err());
    // The default for a command value has no shell-hostile symbols.
    let secret = stored(&f, &handle);
    assert!(secret.chars().all(|c| c.is_ascii_alphanumeric()), "{}", secret.len());
}

#[tokio::test]
async fn generate_for_stdin_and_a_workspace_file() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    unlock(&f);
    let stdin = generate(
        &f,
        "stdin",
        json!({"use_binding":{"target":{"kind":"command_stdin","origin":ORIGIN}}}),
    )
    .await
    .unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    let used = call(
        &f,
        USE_ENV,
        json!({"origin":ORIGIN,"argv":["/usr/bin/wc","-c"],"stdin":stdin}),
    )
    .await
    .unwrap();
    assert_eq!(used["exit_code"], 0);
    let workspace = f.run.working_directory().unwrap().to_path_buf();
    let path = workspace.join("config/app.secret");
    std::fs::create_dir(workspace.join("config")).unwrap();
    let file = generate(&f, "file", json!({"use_binding":{"target":{"kind":"file","path":path}}}))
        .await
        .unwrap()["credential_id"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        &f,
        WRITE_FILE,
        json!({"path":"config/app.secret","template":"{{SECRET}}","values":{"SECRET":file}}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        stored(&f, &file).as_str()
    );
    // A file outside the workspace is refused before anything is generated.
    assert!(generate(
        &f,
        "outside",
        json!({"use_binding":{"target":{"kind":"file","path":"/etc/chariox-generated"}}})
    )
    .await
    .is_err());
}

#[tokio::test]
async fn generate_retry_arguments_and_the_site_password_path_stay_as_before() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    elevated(&f);
    unlock(&f);
    let first = generate(&f, "retry", json!({"use_binding": env_binding("TOKEN")}))
        .await
        .unwrap();
    let again = generate(&f, "retry", json!({"use_binding": env_binding("TOKEN")}))
        .await
        .unwrap();
    assert_eq!(again["credential_id"], first["credential_id"]);
    assert_eq!(again["created"], false);
    // The same request_id never rebinds to another use.
    assert!(generate(&f, "retry", json!({"use_binding": env_binding("OTHER")}))
        .await
        .is_err());
    // Exactly one of origin (a site password) or use_binding; the error names both.
    for invalid in [
        json!({}),
        json!({"origin":"https://example.com","use_binding": env_binding("TOKEN")}),
        json!({"use_binding":{"target":{"kind":"any_site"}}}),
        json!({"use_binding":{"target":{"kind":"site","origin":"https://example.com"}}}),
    ] {
        let error = generate(&f, "invalid", invalid.clone()).await.unwrap_err().to_string();
        assert!(error.contains("origin") && error.contains("use_binding"), "{invalid}: {error}");
    }
    // A site password with no binding is unchanged.
    let site = generate(&f, "site", json!({"origin":"https://example.com"}))
        .await
        .unwrap();
    assert_eq!(site["origin"], "https://example.com");
    assert_eq!(stored_len(&f, site["credential_id"].as_str().unwrap()), 24);
}

fn stored_len(f: &Fixture, handle: &str) -> usize {
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .browser_secret_input_for_target_url(handle, "https://example.com/login")
        .unwrap()
        .chars()
        .count()
}
