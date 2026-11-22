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
    f.state.refill_linked_environment_secrets(&handle);
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some("ghp_RotatedValue77"));

    // The entry is removed: the copy stays as a normal Secret.
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .remove(&handle)
        .unwrap();
    f.state.refill_linked_environment_secrets(&handle);
    assert_eq!(held(&f, "GH_TOKEN").as_deref(), Some("ghp_RotatedValue77"));

    // A typed value ends the link: a later change of the entry no longer reaches the Secret.
    links.remove(&project_id(&f), "GH_TOKEN").unwrap();
    assert!(links.filled_from(&handle).unwrap().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}
