//! Row 276: an agent gives a Project Environment Secret its value from a Vault entry by id. The value
//! moves Vault to Vault inside the kernel and never appears in a tool result or the session.
use super::tests::{fixture, Fixture};
use super::vault_tests::{call, unlock};
use crate::project_environment::{
    project_environment_vault_service, ProjectEnvironmentRecord, SecretPresence,
    PROJECT_ENVIRONMENT_SCHEMA,
};
use serde_json::{json, Value};

const CAPTURE: &str = "chariox.vault.capture_command";
const FILL: &str = "chariox.environment.use_vault_entry";
const SECRET: &str = "ghp_EnvironmentSecretValue5d3c";

fn project_id(f: &Fixture) -> String {
    f.state
        .owned
        .session_store
        .get_session(f.run.session_id())
        .unwrap()
        .project_id()
        .to_owned()
}

fn save_secret_names(f: &Fixture, names: &[&str]) {
    f.state
        .environment_store()
        .save(
            &project_id(f),
            0,
            ProjectEnvironmentRecord {
                schema: PROJECT_ENVIRONMENT_SCHEMA,
                secrets: names.iter().map(|name| name.to_string()).collect(),
                ..Default::default()
            },
        )
        .unwrap();
}

async fn capture_for(f: &Fixture, variable: &str) -> String {
    let captured = call(
        f,
        CAPTURE,
        json!({"request_id": format!("env-{variable}"), "host": "github.com",
            "argv": ["/bin/sh", "-c", format!("printf '%s\\n' '{SECRET}'")],
            "extract": {"kind": "stdout"},
            "use_binding": {"target": {"kind": "command_environment", "name": variable, "origin": "https://github.com"}}}),
    )
    .await
    .unwrap();
    captured["credential_id"].as_str().unwrap().to_owned()
}

fn held(f: &Fixture, name: &str) -> Option<String> {
    let config = f.state.owned.config_projection.snapshot();
    crate::secret::project_environment_vault(&config)
        .unwrap()
        .find_secret(&project_environment_vault_service(&project_id(f)), name)
        .unwrap()
        .map(|value| value.to_string())
}

#[tokio::test]
async fn an_agent_fills_an_environment_secret_from_a_vault_entry_without_seeing_the_value() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    save_secret_names(&f, &["GH_TOKEN"]);
    let handle = capture_for(&f, "GH_TOKEN").await;
    assert_eq!(held(&f, "GH_TOKEN"), None);

    let filled: Value = call(
        &f,
        FILL,
        json!({"name": "GH_TOKEN", "credential_id": handle}),
    )
    .await
    .unwrap();
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some(SECRET));
    assert_eq!(
        filled["secrets"][0]["presence"],
        json!(SecretPresence::Present)
    );
    assert!(!filled.to_string().contains(SECRET));
    let session =
        serde_json::to_string(&f.state.owned.session_snapshot(f.run.session_id()).unwrap())
            .unwrap();
    assert!(!session.contains(SECRET));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn an_entry_made_for_another_variable_or_an_unsaved_name_is_refused_and_nothing_is_stored() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    save_secret_names(&f, &["GH_TOKEN", "OTHER"]);
    let handle = capture_for(&f, "GH_TOKEN").await;
    for (name, id) in [
        ("OTHER", handle.as_str()),
        ("NOT_SAVED", handle.as_str()),
        ("GH_TOKEN", "no-such-entry"),
    ] {
        assert!(
            call(&f, FILL, json!({"name": name, "credential_id": id}))
                .await
                .is_err(),
            "{name} {id}"
        );
        assert_eq!(held(&f, name), None, "{name}");
    }
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_filled_secret_follows_a_rotated_entry_until_a_typed_value_ends_the_link_and_keeps_its_value_when_the_entry_goes(
) {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    save_secret_names(&f, &["GH_TOKEN"]);
    let handle = capture_for(&f, "GH_TOKEN").await;
    call(
        &f,
        FILL,
        json!({"name": "GH_TOKEN", "credential_id": handle}),
    )
    .await
    .unwrap();
    let links = f.state.secret_links();
    assert_eq!(
        links.filled_from(&handle).unwrap(),
        vec![(project_id(&f), "GH_TOKEN".to_owned())]
    );

    // The owner replaces the entry's value: the copy follows it.
    let service = f.state.home_runtime_secret_service().unwrap();
    let key = match &service.credential(&handle).unwrap().source {
        crate::config::UserCredentialSourceConfig::Vault { key } => key.clone(),
        other => panic!("a captured entry lives in the Vault: {other:?}"),
    };
    service
        .set_vault_secret(&key, "ghp_RotatedValue77")
        .unwrap();
    f.state.refill_linked_environment_secrets(&handle).unwrap();
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some("ghp_RotatedValue77"));

    // The entry is removed: the copy stays as a normal Secret.
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .remove(&handle)
        .unwrap();
    // Nothing is left to follow, and the owner is told so if the entry were still replaced.
    assert!(f.state.refill_linked_environment_secrets(&handle).is_err());
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some("ghp_RotatedValue77"));

    // A typed value ends the link: a later change of the entry no longer reaches the Secret.
    links
        .write_linked(&project_id(&f), "GH_TOKEN", None, || Ok(true))
        .unwrap();
    assert!(links.filled_from(&handle).unwrap().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}

/// The owner replaces the entry through their Vault window, as the client does.
async fn owner_replaces(
    f: &Fixture,
    id: &str,
    value: &str,
) -> Result<(), crate::error::DaemonError> {
    use crate::local::{LocalDaemonRequest, OwnerVaultRequest, VaultPassphrase};
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let request = LocalDaemonRequest::OwnerVault(OwnerVaultRequest::Replace {
        id: id.into(),
        value: VaultPassphrase::new(value),
    });
    let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    let command = KernelCommand::from_local_request_with_caller(
        "owner-replace",
        KernelCommandSource::LocalCli,
        caller,
        None,
        None,
        &request,
    );
    f.router.dispatch(command, request).await.map(|_| ())
}

// Review of #1680: a refresh that fails after the source value was replaced must not look like a
// successful rotation. The owner's Replace names the Secrets that kept the old value, and the
// Secrets that could be refreshed still follow.
#[tokio::test]
async fn a_replace_names_the_linked_secrets_that_could_not_take_the_new_value() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    save_secret_names(&f, &["GH_TOKEN", "OTHER"]);
    let handle = capture_for(&f, "GH_TOKEN").await;
    call(
        &f,
        FILL,
        json!({"name": "GH_TOKEN", "credential_id": handle}),
    )
    .await
    .unwrap();
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some(SECRET));
    // A Secret the entry was not made for cannot be refreshed from it.
    f.state
        .secret_links()
        .write_linked(
            &project_id(&f),
            "OTHER",
            Some(crate::project_environment::SecretLink::Entry(handle.clone())),
            || Ok(true),
        )
        .unwrap();

    let failure = owner_replaces(&f, &handle, "ghp_RotatedValue77")
        .await
        .expect_err("a refresh that failed is reported to the owner");
    let message = failure.to_string();
    assert!(message.contains("OTHER"), "{message}");
    assert!(
        !message.contains("ghp_"),
        "no value in the message: {message}"
    );
    // The one that could be refreshed took the new value; the failed one still holds nothing new.
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some("ghp_RotatedValue77"));
    assert_eq!(held(&f, "OTHER"), None);

    // A replace with every link refreshable succeeds.
    f.state
        .secret_links()
        .write_linked(&project_id(&f), "OTHER", None, || Ok(true))
        .unwrap();
    owner_replaces(&f, &handle, "ghp_SecondRotation88")
        .await
        .unwrap();
    assert_eq!(
        held(&f, "GH_TOKEN").as_deref(),
        Some("ghp_SecondRotation88")
    );
    f.state.shutdown_cleanup().await.unwrap();
}

async fn owner_sends(f: &Fixture, request: crate::local::LocalDaemonRequest) {
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    let command = KernelCommand::from_local_request_with_caller(
        "owner-request",
        KernelCommandSource::LocalCli,
        caller,
        None,
        None,
        &request,
    );
    f.router.dispatch(command, request).await.unwrap();
}

async fn filled_entry(f: &Fixture) -> (String, String) {
    save_secret_names(f, &["GH_TOKEN"]);
    let handle = capture_for(f, "GH_TOKEN").await;
    call(f, FILL, json!({"name": "GH_TOKEN", "credential_id": handle}))
        .await
        .unwrap();
    let key = match &f
        .state
        .home_runtime_secret_service()
        .unwrap()
        .credential(&handle)
        .unwrap()
        .source
    {
        crate::config::UserCredentialSourceConfig::Vault { key } => key.clone(),
        other => panic!("a captured entry lives in the Vault: {other:?}"),
    };
    (handle, key)
}

#[tokio::test]
async fn a_raw_write_of_the_entry_value_refills_its_linked_secrets_like_a_replace() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let (_, key) = filled_entry(&f).await;
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some(SECRET));
    owner_sends(
        &f,
        crate::local::LocalDaemonRequest::SetCredentialSecret(
            crate::local::SetCredentialSecretRequest {
                session_id: None,
                agent_id: None,
                key,
                value: "ghp_RawKeyWrite91".into(),
            },
        ),
    )
    .await;
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some("ghp_RawKeyWrite91"));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn deleting_the_entry_keeps_the_copy_as_a_normal_secret() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let (handle, _) = filled_entry(&f).await;
    owner_sends(
        &f,
        crate::local::LocalDaemonRequest::OwnerVault(crate::local::OwnerVaultRequest::Delete {
            id: handle.clone(),
        }),
    )
    .await;
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some(SECRET));
    // Nothing stands behind it any more, so a kernel copying it no longer follows it.
    assert!(f.state.secret_links().filled_from(&handle).unwrap().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}
