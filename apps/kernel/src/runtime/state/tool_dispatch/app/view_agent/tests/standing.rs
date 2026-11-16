//! The owner's standing permissions through the real kernel state: only the repository and
//! action class the owner named skip a card, only while their account binding lives, and only
//! the owner can make one.
use super::*;
use crate::durable_state::app_accounts::{AccountBinding, AccountCommand};
use crate::durable_state::app_standing::{StandingCommand, StandingPermission};
use crate::local::{AppRequestErrorCode, GrantAppStandingRequest, LocalDaemonRequest};
use crate::runtime::state::tool_dispatch::app::view_agent::backend;
use chariox_app_package::v2::StandingClass;
use chariox_app_package::RouteSafety;

fn gated(name: &str, effect: UiActionEffect, class: StandingClass) -> UiActionDeclaration {
    serde_json::from_value(json!({
        "name": name, "description": format!("{name} a pull request"),
        "inputSchema": {"type": "object", "additionalProperties": false,
            "properties": {"repo": {"type": "string"}, "account": {"type": "string"}}, "required": ["repo"]},
        "effect": match effect {
            UiActionEffect::Approval => "approval",
            _ => "person_confirmed",
        },
        "approvalReason": (effect == UiActionEffect::Approval).then_some("Changes a pull request"),
        "confirmationView": (effect == UiActionEffect::PersonConfirmed).then_some("preview"),
        "standing": {"class": class.word(), "repo": "repo", "account": "account"},
    }))
    .unwrap()
}

fn store(rig: &Rig) -> &crate::durable_state::DurableKernelStateStore {
    &rig.runtime.owned.durable_state_store
}

fn bind(rig: &Rig, account: &str) {
    store(rig)
        .app_account_command(AccountCommand::Bind {
            owner: "alice".into(),
            installation: APP.into(),
            binding: AccountBinding {
                account: account.into(),
                credential: "github".into(),
                vault_credential_id: format!("entry-{account}"),
                allow: RouteSafety::Destructive,
                bound_at_ms: 1,
            },
        })
        .unwrap();
}

fn stand(rig: &Rig, id: &str, repo: &str, class: StandingClass, expires_at_ms: Option<u64>) {
    store(rig)
        .app_standing_command(StandingCommand::Grant {
            owner: "alice".into(),
            installation: APP.into(),
            permission: StandingPermission {
                id: id.into(),
                account: "work".into(),
                repo: repo.into(),
                class,
                created_at_ms: 1,
                expires_at_ms,
                uses: 0,
                last_used_ms: None,
            },
        })
        .unwrap();
}

/// The action asks for its card (and is denied), or goes on to the page without one.
async fn asks(rig: &Rig, context: &ViewContext, action: &str, repo: &str) -> bool {
    asks_as(rig, context, action, json!({"repo": repo})).await
}

async fn asks_as(rig: &Rig, context: &ViewContext, action: &str, input: Value) -> bool {
    let invoke = rig
        .runtime
        .invoke_declared(rig.run.id(), &rig.agent, context, action, input);
    let (outcome, asked) = tokio::join!(invoke, async {
        let card = pending_approval(rig).await?;
        rig.runtime
            .resolve_runtime_interaction(&rig.session, card.id(), "deny", None)
            .await
            .unwrap();
        Some(())
    });
    match asked {
        Some(()) => {
            assert_eq!(outcome.unwrap_err().code, "approval_denied");
            true
        }
        None => {
            // Past every gate, the call ends at the page, which this rig does not have.
            assert!(!matches!(
                outcome.unwrap_err().code.as_str(),
                "approval_denied" | "approval_unavailable"
            ));
            false
        }
    }
}

#[tokio::test]
async fn a_standing_permission_skips_the_card_for_its_repository_and_class_only() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(vec![
        gated("approve", UiActionEffect::Approval, StandingClass::Review),
        gated("close_it", UiActionEffect::Approval, StandingClass::Close),
    ]);
    bind(&rig, "work");
    assert!(
        asks(&rig, &context, "approve", "O/A").await,
        "without a permission the card asks"
    );
    stand(&rig, "standing-1", "o/a", StandingClass::Review, None);
    assert!(
        !asks(&rig, &context, "approve", "O/A").await,
        "the repository the owner named, in any case"
    );
    assert!(
        asks(&rig, &context, "approve", "o/b").await,
        "another repository still asks"
    );
    assert!(
        asks(&rig, &context, "close_it", "o/a").await,
        "another action class still asks"
    );
    let [used] = store(&rig)
        .app_standing("alice", APP)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        used.uses, 1,
        "the skipped card is counted for the owner to see"
    );
}

#[tokio::test]
async fn a_standing_permission_covers_only_the_account_it_was_given_for() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(vec![gated(
        "approve",
        UiActionEffect::Approval,
        StandingClass::Review,
    )]);
    bind(&rig, "work");
    bind(&rig, "home");
    stand(&rig, "standing-1", "o/a", StandingClass::Review, None);
    assert!(
        !asks_as(
            &rig,
            &context,
            "approve",
            json!({"repo": "o/a", "account": "work"})
        )
        .await,
        "the account the owner named"
    );
    assert!(
        asks_as(
            &rig,
            &context,
            "approve",
            json!({"repo": "o/a", "account": "home"})
        )
        .await,
        "another account in the same repository still asks"
    );
    assert!(
        asks_as(&rig, &context, "approve", json!({"repo": "o/a"})).await,
        "with two accounts bound and nothing to say which one runs it, the card asks"
    );
    let [used] = store(&rig)
        .app_standing("alice", APP)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(used.uses, 1, "only the covered call is charged to it");
}

/// The App's own answer to which account a call that names none runs as, in a backend-served view.
fn backend_context(rig: &Rig, declared: Vec<UiActionDeclaration>) -> ViewContext {
    let mut context = rig.context(declared);
    context.host = ViewHost::Backend { changes: false };
    context
}

#[tokio::test]
async fn a_call_that_names_no_account_is_covered_when_the_app_resolves_it_to_the_permitted_one() {
    let mut rig = rig();
    rig.grant().await;
    let context = backend_context(
        &rig,
        vec![gated(
            "approve",
            UiActionEffect::Approval,
            StandingClass::Review,
        )],
    );
    bind(&rig, "work");
    bind(&rig, "home");
    stand(&rig, "standing-1", "o/a", StandingClass::Review, None);

    // The repository is mapped to "work" in the App: the call runs as work, which the owner permitted.
    backend::test_backend::serve(&rig.session, "view_account", json!({"account": "work"}));
    assert!(
        !asks_as(&rig, &context, "approve", json!({"repo": "O/A"})).await,
        "the App resolves the call to the account the permission names"
    );
    assert_eq!(
        backend::test_backend::asked(&rig.session, "view_account"),
        [json!({"repo": "o/a"})],
        "the App is asked about the repository the signed declaration names"
    );

    // Mapped to the other account, the permission for "work" does not cover it.
    backend::test_backend::serve(&rig.session, "view_account", json!({"account": "home"}));
    assert!(
        asks_as(&rig, &context, "approve", json!({"repo": "o/a"})).await,
        "resolved to another account, the card asks"
    );

    // A call that names its account is not resolved again.
    backend::test_backend::serve(&rig.session, "view_account", json!({"account": "home"}));
    assert!(
        !asks_as(
            &rig,
            &context,
            "approve",
            json!({"repo": "o/a", "account": "work"})
        )
        .await
    );
    assert!(backend::test_backend::asked(&rig.session, "view_account").is_empty());
    let [used] = store(&rig)
        .app_standing("alice", APP)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(used.uses, 2, "only the covered calls are charged to it");
}

#[tokio::test]
async fn revoking_the_binding_or_expiry_brings_the_card_back_at_once() {
    let mut rig = rig();
    rig.grant().await;
    let context = rig.context(vec![gated(
        "approve",
        UiActionEffect::Approval,
        StandingClass::Review,
    )]);
    bind(&rig, "work");
    stand(&rig, "standing-1", "o/a", StandingClass::Review, None);
    assert!(!asks(&rig, &context, "approve", "o/a").await);
    store(&rig)
        .app_account_command(AccountCommand::Revoke {
            owner: "alice".into(),
            installation: APP.into(),
            account: "work".into(),
        })
        .unwrap();
    assert!(
        asks(&rig, &context, "approve", "o/a").await,
        "no binding, no standing permission"
    );
    bind(&rig, "work");
    let soon = crate::session::unix_epoch_ms() + 300;
    stand(&rig, "standing-2", "o/a", StandingClass::Review, Some(soon));
    assert!(!asks(&rig, &context, "approve", "o/a").await);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(
        asks(&rig, &context, "approve", "o/a").await,
        "an expired permission asks again"
    );
}

#[tokio::test]
async fn a_person_confirmed_action_with_a_standing_permission_needs_no_card_or_app_tab() {
    let mut rig = rig();
    rig.grant().await;
    let mut context = rig.context(vec![gated(
        "merge_pull_request",
        UiActionEffect::PersonConfirmed,
        StandingClass::Merge,
    )]);
    context.host = ViewHost::Backend { changes: false };
    bind(&rig, "work");
    stand(&rig, "standing-1", "o/a", StandingClass::Merge, None);
    let outcome = rig
        .runtime
        .invoke_declared(
            rig.run.id(),
            &rig.agent,
            &context,
            "merge_pull_request",
            json!({"repo": "o/a"}),
        )
        .await;
    // It reaches the backend, which this rig does not run; it was not stopped for a card.
    assert!(!matches!(
        outcome.unwrap_err().code.as_str(),
        "confirmation_unavailable" | "confirmation_declined"
    ));
    let session = rig
        .runtime
        .owned
        .session_store
        .get_session(&rig.session)
        .unwrap();
    assert!(session
        .active_interactions()
        .iter()
        .all(|i| !i.id().starts_with("app-confirm-")));
}

#[tokio::test]
async fn only_the_owners_request_makes_one_and_only_for_a_class_the_app_declares() {
    let mut rig = rig_for_catalog(true);
    rig.grant().await;
    rig.stage_signed_v2_fixture();
    bind(&rig, "work");
    let grant = |class: StandingClass, repo: &str, expires_at_ms| {
        LocalDaemonRequest::GrantAppStanding(GrantAppStandingRequest {
            installation_id: APP.into(),
            account: "work".into(),
            repo: repo.into(),
            class,
            expires_at_ms,
        })
    };
    // The fixture App declares no action with a standing class: nothing to cover.
    let refused = rig
        .runtime
        .app_standing_request(
            "alice".into(),
            APP.into(),
            grant(StandingClass::Merge, "o/a", None),
        )
        .await;
    assert_eq!(refused.unwrap_err(), AppRequestErrorCode::InvalidRequest);
    for (repo, expires) in [("not a repo", None), ("o/a/b", None), ("o/a", Some(1))] {
        let refused = rig
            .runtime
            .app_standing_request(
                "alice".into(),
                APP.into(),
                grant(StandingClass::Merge, repo, expires),
            )
            .await;
        assert_eq!(
            refused.unwrap_err(),
            AppRequestErrorCode::InvalidRequest,
            "{repo} {expires:?}"
        );
    }
    // There is no class for what must never be granted, and no agent tool or App import for the request.
    for class in [
        "force_push",
        "delete_default_branch",
        "grant",
        "account_binding",
    ] {
        let request = json!({"GrantAppStanding": {"installation_id": APP, "account": "work", "repo": "o/a", "class": class}});
        assert!(
            serde_json::from_value::<LocalDaemonRequest>(request).is_err(),
            "{class}"
        );
    }
    assert!(
        rig.tool_names()
            .iter()
            .all(|name| !name.contains("standing")),
        "{:?}",
        rig.tool_names()
    );
    assert!(store(&rig).app_standing("alice", APP).unwrap().is_empty());
}
