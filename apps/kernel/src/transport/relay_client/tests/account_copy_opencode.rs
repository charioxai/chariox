//! Native OpenCode copy follows the public source router and encrypted target transaction.
use super::*;
use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};

const ENVIRONMENT: [&str; 2] = [
    "CHARIOX_OPENCODE_BIN",
    "CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS",
];

fn source_login(
    profiles: &ProviderAccountProfileRegistry,
    label: &str,
    login: &[u8],
) -> (String, PathBuf) {
    let profile = profiles.create_managed("local", "opencode", label).unwrap();
    let environment = profiles
        .resolve_environment("local", "opencode", &profile.profile_id)
        .unwrap();
    let auth = Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
    std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
    std::fs::write(&auth, login).unwrap();
    profiles
        .update_observation(
            "local",
            "opencode",
            &profile.profile_id,
            ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    (profile.profile_id, auth)
}

fn copy_request(source: &str, operation: &str, target: &DaemonConfig) -> serde_json::Value {
    serde_json::json!({"CopyProviderAccountToKernel": {
        "provider":"opencode", "account_profile":source, "operation_id":operation,
        "target_kernel_id":target.daemon_id,
        "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&target.relay_public_key),
        "replace":false
    }})
}

fn target_logins(target_profiles: &ProviderAccountProfileRegistry) -> usize {
    target_profiles
        .list("local", Some("opencode"))
        .unwrap()
        .len()
}

pub(super) async fn check_opencode_copy(
    root: &Path,
    target: &DaemonConfig,
    source_router: &CommandRouter,
    source_profiles: &ProviderAccountProfileRegistry,
    target_profiles: &ProviderAccountProfileRegistry,
) {
    let _restore = Environment(
        ENVIRONMENT
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect(),
    );
    let executable = root.join("opencode-copy-check");
    std::fs::write(&executable, "#!/bin/sh\n[ \"$1\" = --version ] && { echo 1.19.0; exit 0; }\n[ \"$1 $2\" = 'auth list' ] || exit 2\n[ -s \"$XDG_DATA_HOME/opencode/auth.json\" ] || exit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
    let canary = "synthetic-opencode-copy";
    let login = format!(r#"{{"openai":{{"type":"api","key":"{canary}"}}}}"#);
    let (source, source_auth) = source_login(source_profiles, "Native source", login.as_bytes());
    // The target already holds its own default OpenCode profile; count what a copy adds.
    let baseline = target_logins(target_profiles);

    // A1: the login arrives as an independent, authenticated profile.
    let request = copy_request(&source, "opencode-copy", target);
    let response = source_request(source_router, request.clone())
        .await
        .unwrap();
    assert!(!response.to_string().contains(canary));
    let id = response["ProviderAccountCopied"]["receipt"]["account_profile"]
        .as_str()
        .unwrap();
    let source_environment = source_profiles
        .resolve_environment("local", "opencode", &source)
        .unwrap();
    let target_environment = target_profiles
        .resolve_environment("local", "opencode", id)
        .unwrap();
    assert_ne!(
        source_environment["XDG_DATA_HOME"],
        target_environment["XDG_DATA_HOME"]
    );
    let target_auth = Path::new(&target_environment["XDG_DATA_HOME"]).join("opencode/auth.json");
    assert_eq!(std::fs::read_to_string(&target_auth).unwrap(), login);
    assert_eq!(
        target_profiles
            .get("local", "opencode", id)
            .unwrap()
            .auth_state,
        ProviderAccountAuthState::Authenticated
    );
    assert!(
        !crate::provider::provider_account_credential_uses_vault("local", "opencode", id).unwrap(),
        "an OpenCode login lives only in its native store"
    );

    // A3: a retry of the completed operation leaves the target's own login alone.
    std::fs::write(&target_auth, "target-independent-login").unwrap();
    std::fs::write(&source_auth, "source-rotated-login").unwrap();
    assert_eq!(
        source_request(source_router, request.clone())
            .await
            .unwrap(),
        response
    );
    assert_eq!(
        std::fs::read_to_string(&target_auth).unwrap(),
        "target-independent-login"
    );
    // B5: the same operation with only `replace` changed names that difference.
    let mut replace_only = request.clone();
    replace_only["CopyProviderAccountToKernel"]["replace"] = true.into();
    assert_code(
        source_request(source_router, replace_only).await,
        "provider_account_copy_replace_differs",
        canary,
    );
    let mut other_operation = request.clone();
    other_operation["CopyProviderAccountToKernel"]["operation_id"] = "opencode-again".into();
    assert_code(
        source_request(source_router, other_operation).await,
        "provider_account_copy_conflict",
        canary,
    );
    assert_eq!(target_logins(target_profiles), baseline + 1);

    // A4: a login over the copy limit is refused at the source, before it is sent.
    let big = format!(
        r#"{{"openai":{{"type":"api","key":"{}"}}}}"#,
        "k".repeat(crate::provider_account_copy::MAX_COPY_CREDENTIAL_BYTES)
    );
    let (too_large, _) = source_login(source_profiles, "Too large", big.as_bytes());
    assert_code(
        source_request(
            source_router,
            copy_request(&too_large, "opencode-big", target),
        )
        .await,
        "provider_account_copy_credential_too_large",
        canary,
    );

    // A5: a login OpenCode cannot use is refused by the receiver and publishes nothing.
    for (name, bytes) in [
        ("malformed", "not json".to_string()),
        (
            "empty-key",
            r#"{"openai":{"type":"api","key":""}}"#.to_string(),
        ),
    ] {
        let (bad, _) = source_login(source_profiles, name, bytes.as_bytes());
        assert_code(
            source_request(source_router, copy_request(&bad, name, target)).await,
            "provider_account_copy_rejected",
            canary,
        );
    }

    // A6: the provider's own answer to an OpenCode Go key decides.
    let go = r#"{"opencode-go":{"type":"api","key":"synthetic-go-login"}}"#;
    let (go_source, _) = source_login(source_profiles, "Go", go.as_bytes());
    std::env::set_var("CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS", "401");
    assert_code(
        source_request(
            source_router,
            copy_request(&go_source, "go-rejected", target),
        )
        .await,
        "provider_account_copy_rejected",
        "synthetic-go-login",
    );
    assert_eq!(target_logins(target_profiles), baseline + 1);
    std::env::set_var("CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS", "200");
    let verified = source_request(
        source_router,
        copy_request(&go_source, "go-verified", target),
    )
    .await
    .unwrap();
    assert!(!verified.to_string().contains("synthetic-go-login"));
    assert_eq!(target_logins(target_profiles), baseline + 2);
    std::env::remove_var("CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS");

    // B4: a receiver without the OpenCode CLI says so, whoever the provider is.
    let (no_cli, _) = source_login(source_profiles, "No CLI", login.as_bytes());
    std::env::set_var("CHARIOX_OPENCODE_BIN", root.join("missing-opencode"));
    assert_code(
        source_request(source_router, copy_request(&no_cli, "no-cli", target)).await,
        "provider_account_copy_cli_missing",
        canary,
    );
    assert_eq!(target_logins(target_profiles), baseline + 2);

    // Failed copies leave no staging profile behind.
    let providers = target_auth
        .ancestors()
        .nth(4)
        .expect("provider account root")
        .to_path_buf();
    assert!(!std::fs::read_dir(providers).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".account-copy-")));
}

/// A machine enrolled with its machine credential alone chooses an offer itself: the
/// recipient-side listing and `CopyProviderAccountFromKernel` admit it like any kernel.
pub(super) async fn check_machine_enrolled_receiver_pulls(
    root: &Path,
    target: &DaemonConfig,
    source: &DaemonConfig,
    source_router: &CommandRouter,
    target_router: &CommandRouter,
    source_profiles: &ProviderAccountProfileRegistry,
    target_profiles: &ProviderAccountProfileRegistry,
) {
    let _ = source_router;
    let _restore = Environment(
        ENVIRONMENT
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect(),
    );
    let executable = root.join("opencode-pull-check");
    std::fs::write(&executable, "#!/bin/sh\n[ \"$1\" = --version ] && { echo 1.19.0; exit 0; }\n[ \"$1 $2\" = 'auth list' ] || exit 2\n[ -s \"$XDG_DATA_HOME/opencode/auth.json\" ] || exit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
    let canary = "synthetic-machine-pull";
    let login = format!(r#"{{"openai":{{"type":"api","key":"{canary}"}}}}"#);
    let (profile, _) = source_login(source_profiles, "Pulled", login.as_bytes());
    // The source publishes its new login to the relay in the background.
    let mut offers = serde_json::Value::Null;
    for _ in 0..200 {
        offers = source_request(
            target_router,
            serde_json::json!({"ListProviderAccountCopyOffers":{}}),
        )
        .await
        .expect("a machine enrolled with its machine credential lists the offers");
        if offers["ProviderAccountCopyOffers"]["offers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|offer| {
                offer["provider"] == "opencode"
                    && offer["account_profile"] == profile
                    && offer["source_kernel_id"] == source.daemon_id
            })
        {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    assert!(
        offers["ProviderAccountCopyOffers"]["offers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|offer| offer["provider"] == "opencode"
                && offer["account_profile"] == profile
                && offer["source_kernel_id"] == source.daemon_id),
        "{offers}"
    );
    let response = source_request(
        target_router,
        serde_json::json!({"CopyProviderAccountFromKernel":{
            "provider":"opencode", "account_profile":profile, "operation_id":"machine-pull",
            "source_kernel_id":source.daemon_id,
            "source_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&source.relay_public_key),
            "replace":false
        }}),
    )
    .await
    .expect("the machine-enrolled receiver pulls the login");
    assert!(!response.to_string().contains(canary));
    let id = response["ProviderAccountCopied"]["receipt"]["account_profile"]
        .as_str()
        .unwrap();
    let environment = target_profiles
        .resolve_environment("local", "opencode", id)
        .unwrap();
    let auth = Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json");
    assert_eq!(std::fs::read_to_string(auth).unwrap(), login);
    assert_eq!(target.cloud_relay.as_ref().unwrap().kernel_credential, None);
}
