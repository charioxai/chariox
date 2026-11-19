//! Native Codex copy follows the public source router and encrypted target transaction.
use super::*;
use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfileRegistry};

pub(super) async fn check_native_copy(
    root: &Path,
    target: &DaemonConfig,
    source_router: &CommandRouter,
    target_router: &CommandRouter,
    source_profiles: &ProviderAccountProfileRegistry,
    target_profiles: &ProviderAccountProfileRegistry,
) {
    let executable = root.join("codex-copy-check");
    std::fs::write(&executable, "#!/bin/sh\n[ \"$1 $2\" = 'login status' ] || exit 2\n[ -s \"$CODEX_HOME/auth.json\" ] || exit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &executable);
    let canary = r#"{"tokens":{"access_token":"synthetic-native-copy"}}"#;
    let profile = source_profiles
        .create_managed("local", "codex", "Native source")
        .unwrap();
    let source_environment = source_profiles
        .resolve_environment("local", "codex", &profile.profile_id)
        .unwrap();
    let source_auth = Path::new(&source_environment["CODEX_HOME"]).join("auth.json");
    std::fs::write(&source_auth, canary).unwrap();
    source_profiles
        .update_observation(
            "local",
            "codex",
            &profile.profile_id,
            ProviderAccountAuthState::Authenticated,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let request = serde_json::json!({"CopyProviderAccountToKernel": {
        "provider":"codex", "account_profile":profile.profile_id, "operation_id":"native-copy",
        "target_kernel_id":target.daemon_id,
        "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&target.relay_public_key),
        "replace":false
    }});
    let response = source_request(source_router, request.clone())
        .await
        .unwrap();
    assert!(!response.to_string().contains("synthetic-native-copy"));
    let id = response["ProviderAccountCopied"]["receipt"]["account_profile"]
        .as_str()
        .unwrap();
    let target_environment = target_profiles
        .resolve_environment("local", "codex", id)
        .unwrap();
    assert_ne!(
        source_environment["CODEX_HOME"],
        target_environment["CODEX_HOME"]
    );
    let target_auth = Path::new(&target_environment["CODEX_HOME"]).join("auth.json");
    assert_eq!(std::fs::read_to_string(&target_auth).unwrap(), canary);
    assert_eq!(
        target_profiles
            .get("local", "codex", id)
            .unwrap()
            .auth_state,
        ProviderAccountAuthState::Authenticated
    );
    assert!(
        crate::provider::resolve_provider_account_credentials(target, "local", "codex", id)
            .unwrap()
            .is_empty(),
        "the Vault transfer receipt is never a launch environment or a refresh source"
    );
    let inbox = source_request(target_router, serde_json::json!({"GetPersonInbox":null}))
        .await
        .unwrap();
    let card = inbox["PersonInbox"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["body"]["notice"]["action"]["account_profile"] == id)
        .unwrap();
    let notice = &card["body"]["notice"];
    assert!(notice["title"].as_str().unwrap().contains("was copied to"));
    assert_eq!(notice["action"]["kind"], "provider_sign_in");
    assert_eq!(notice["action"]["kernel_id"], target.daemon_id);
    assert!(!inbox.to_string().contains("synthetic-native-copy"));
    source_request(
        target_router,
        serde_json::json!({"SetInboxCardState":{
            "card_ids":[card["card_id"]], "change":{"read":{"read":true}}
        }}),
    )
    .await
    .unwrap();
    // Both native refresh and a new device login may replace auth.json. Retry
    // must return the same receipt and leave that new lineage alone.
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
    assert_eq!(
        std::fs::read_to_string(&source_auth).unwrap(),
        "source-rotated-login"
    );
    let inbox = source_request(target_router, serde_json::json!({"GetPersonInbox":null}))
        .await
        .unwrap();
    let cards = inbox["PersonInbox"]["cards"].as_array().unwrap();
    let matching = cards
        .iter()
        .filter(|card| card["body"]["notice"]["action"]["account_profile"] == id)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0]["body"]["notice"]["read"], true);
    let mut conflict = request.clone();
    conflict["CopyProviderAccountToKernel"]["operation_id"] = "native-conflict".into();
    assert_code(
        source_request(source_router, conflict).await,
        "provider_account_copy_conflict",
        "synthetic-native-copy",
    );
    let foreign = request_as(
        target_router,
        serde_json::json!({"GetPersonInbox":null}),
        |caller| {
            caller.user_id = Some("another-owner".into());
        },
    )
    .await;
    if let Ok(inbox) = foreign {
        assert!(!inbox.to_string().contains(id));
    }
    // A stranded copy replaces the already-read advice with an unread recovery
    // card. A successful sign-in clears it, and another expiry can notify again.
    for _ in 0..2 {
        target_profiles
            .update_observation(
                "local",
                "codex",
                id,
                ProviderAccountAuthState::Expired,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        source_request(
            target_router,
            serde_json::json!({"GetProviderAccountProfile":{
                "provider":"codex", "account_profile":id
            }}),
        )
        .await
        .unwrap();
        let inbox = source_request(target_router, serde_json::json!({"GetPersonInbox":null}))
            .await
            .unwrap();
        let card = inbox["PersonInbox"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["body"]["notice"]["action"]["account_profile"] == id)
            .unwrap();
        assert!(card["body"]["notice"]["title"]
            .as_str()
            .unwrap()
            .contains("Sign in again on"));
        assert_eq!(card["body"]["notice"]["read"], false);
        assert_eq!(card["body"]["notice"]["priority"], "high");
        target_profiles
            .update_observation(
                "local",
                "codex",
                id,
                ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        source_request(
            target_router,
            serde_json::json!({"GetProviderAccountProfile":{
                "provider":"codex", "account_profile":id
            }}),
        )
        .await
        .unwrap();
        let inbox = source_request(target_router, serde_json::json!({"GetPersonInbox":null}))
            .await
            .unwrap();
        assert!(!inbox["PersonInbox"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["body"]["notice"]["action"]["account_profile"] == id));
    }
    let approval = root.join("approve-copy-target");
    let launches = root.join("copy-target-account-endpoints");
    let quote = |s: &Path| format!("'{}'", s.to_str().unwrap().replace('\'', "'\\''"));
    std::fs::write(&executable, format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo fixture; exit; fi\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = --listen ]; then shift; export CTXFIX_ACCOUNT_LISTEN=\"$1\"; break; fi; shift; done\nexport CTXFIX_ACCOUNT_LOG={}\nexport CTXFIX_ACCOUNT_LOGIN_COMPLETE={}\nexec {} --exact local::provider_requests::catalog::codex_refresh_tests::account_endpoint_child --ignored --nocapture\n",
        quote(&launches), quote(&approval), quote(&std::env::current_exe().unwrap())
    )).unwrap();
    std::fs::write(
        &target_auth,
        r#"{"tokens":{"access_token":"synthetic-accepted-login-interim"}}"#,
    )
    .unwrap();
    let started = source_request(
        target_router,
        serde_json::json!({"StartProviderLogin":{
            "provider":"codex", "account_profile":id, "method":"device_code"
        }}),
    )
    .await
    .unwrap();
    let login_id = started["ProviderLoginStarted"]["login"]["login_id"]
        .as_str()
        .unwrap();
    std::fs::write(approval, "approve").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = source_request(
                target_router,
                serde_json::json!({"GetProviderLoginStatus":{
                    "login_id":login_id
                }}),
            )
            .await
            .unwrap();
            if status["ProviderLoginStatus"]["login"]["state"] == "succeeded" {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("independent target sign-in must complete");
    let inbox = source_request(target_router, serde_json::json!({"GetPersonInbox":null}))
        .await
        .unwrap();
    assert!(
        !inbox["PersonInbox"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["body"]["notice"]["action"]["account_profile"] == id),
        "independent sign-in must clear copy advice before replay"
    );
    let target_login = std::fs::read_to_string(&target_auth).unwrap();
    assert!(target_login.contains("new-device"));
    assert_eq!(
        source_request(source_router, request).await.unwrap(),
        response
    );
    assert!(std::fs::read_to_string(&target_auth).unwrap() == target_login);
    let inbox = source_request(target_router, serde_json::json!({"GetPersonInbox":null}))
        .await
        .unwrap();
    assert!(
        !inbox["PersonInbox"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["body"]["notice"]["action"]["account_profile"] == id),
        "retrying a completed copy must not recreate sign-in advice after independent sign-in"
    );
    crate::provider::shutdown_codex_account_endpoints();
    source_profiles
        .remove_registration("local", "codex", &profile.profile_id)
        .unwrap();
    assert_eq!(
        target_profiles
            .resolve_environment("local", "codex", id)
            .unwrap(),
        target_environment,
        "an independent copy resolves without its source profile or kernel"
    );
    assert!(!std::fs::read_dir(
        target_auth
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
    )
    .unwrap()
    .any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".account-copy-")));
}
