//! The owner's Kind and Use for a Vault entry, through the real owner request router and the
//! agent's HTTP and command tools: an API key reaches its origin, a change of Kind and Use
//! takes effect at once, and a refused use names the Kind that would serve it.
use super::tests::{fixture, Fixture};
use super::vault_tests::{call, unlock};
use crate::config::{VaultUseBinding, VaultUseTarget, VaultValueKind};
use crate::local::{
    KernelConnectionClass, LocalDaemonRequest, LocalDaemonResponse, OwnerVaultRequest,
    OwnerVaultResult, VaultPassphrase,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const KEY: &str = "re_owner_only_dummy_key_0123456789";
const HTTP: &str = "chariox.http_request_with_credential";
const USE_ENV: &str = "chariox.vault.use_env";

/// A loopback origin that answers every request with 200 and remembers what it received.
struct Origin {
    origin: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

async fn origin() -> Origin {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requests.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buffer[..read]).to_lowercase());
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        }
    });
    Origin { origin, requests }
}

fn site(origin: &str) -> VaultUseBinding {
    VaultUseBinding {
        target: VaultUseTarget::site(origin).unwrap(),
        value_kind: VaultValueKind::Secret,
    }
}

async fn owner(f: &Fixture, request: OwnerVaultRequest) -> Result<LocalDaemonResponse, String> {
    let request = LocalDaemonRequest::OwnerVault(request);
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-kind",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(KernelConnectionClass::Terminal);
    f.router
        .dispatch(command, request)
        .await
        .map_err(|error| error.to_string())
}

async fn create(f: &Fixture, kind: &str, binding: VaultUseBinding, value: &str) -> String {
    let created = owner(
        f,
        OwnerVaultRequest::Create {
            name: "Resend".into(),
            kind: kind.into(),
            use_binding: binding,
            value: VaultPassphrase::new(value),
        },
    )
    .await
    .unwrap();
    let LocalDaemonResponse::OwnerVault {
        result: OwnerVaultResult::Saved { id },
    } = created
    else {
        panic!("owner creation failed")
    };
    id
}

async fn update(f: &Fixture, id: &str, kind: &str, binding: VaultUseBinding) -> Result<(), String> {
    owner(
        f,
        OwnerVaultRequest::Update {
            id: id.into(),
            kind: kind.into(),
            use_binding: binding,
        },
    )
    .await
    .map(|_| ())
}

async fn http(f: &Fixture, id: &str, url: &str) -> Result<(), String> {
    call(
        f,
        HTTP,
        json!({"credential_id": id, "url": url, "timeout_ms": 5000}),
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn activity(f: &Fixture, id: &str) -> Vec<String> {
    f.state
        .home_runtime_secret_service()
        .unwrap()
        .vault_activity(id)
        .unwrap()
        .into_iter()
        .map(|row| row.event)
        .collect()
}

#[tokio::test]
async fn an_api_key_entry_sends_its_bearer_to_exactly_its_origin() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let (api, other) = (origin().await, origin().await);
    let id = create(&f, "API key", site(&api.origin), KEY).await;
    http(&f, &id, &format!("{}/emails", api.origin))
        .await
        .unwrap();
    assert!(
        api.requests.lock().unwrap()[0]
            .contains(&format!("authorization: bearer {}", KEY.to_lowercase())),
        "{:?}",
        api.requests.lock().unwrap()
    );
    let refused = http(&f, &id, &other.origin).await.unwrap_err();
    assert!(!refused.contains(KEY));
    assert!(other.requests.lock().unwrap().is_empty());
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_password_refused_for_http_or_a_command_names_the_kind_that_would_serve_it() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let api = origin().await;
    let id = create(&f, "Password", site(&api.origin), KEY).await;
    let refused = http(&f, &id, &api.origin).await.unwrap_err();
    assert!(
        refused.contains("saved as Password (browser fill); change Kind to API key in the Vault"),
        "{refused}"
    );
    assert!(api.requests.lock().unwrap().is_empty());
    let refused = call(
        &f,
        USE_ENV,
        json!({"origin": api.origin, "argv": ["/usr/bin/env"], "env": {"KEY": id}}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("change Kind to Command secret in the Vault"),
        "{refused}"
    );
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn changing_kind_and_use_keeps_the_entry_and_value_and_takes_effect_at_once() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let (first, second) = (origin().await, origin().await);
    let id = create(&f, "Password", site(&first.origin), KEY).await;
    assert!(http(&f, &id, &first.origin).await.is_err());

    update(&f, &id, "API key", site(&first.origin))
        .await
        .unwrap();
    http(&f, &id, &first.origin).await.unwrap();
    assert!(first.requests.lock().unwrap()[0].contains(&format!("bearer {}", KEY.to_lowercase())));

    update(&f, &id, "API key", site(&second.origin))
        .await
        .unwrap();
    assert!(http(&f, &id, &first.origin).await.is_err());
    http(&f, &id, &second.origin).await.unwrap();
    assert!(second.requests.lock().unwrap()[0].contains(&format!("bearer {}", KEY.to_lowercase())));

    let LocalDaemonResponse::OwnerVault {
        result: OwnerVaultResult::Revealed {
            id: revealed,
            value,
        },
    } = owner(&f, OwnerVaultRequest::Reveal { id: id.clone() })
        .await
        .unwrap()
    else {
        panic!("reveal failed")
    };
    assert_eq!((revealed, value.expose_secret()), (id.clone(), KEY));
    let rows = activity(&f, &id);
    let changed: Vec<_> = rows
        .iter()
        .filter(|row| row.starts_with("Kind and Use changed by owner"))
        .collect();
    assert_eq!(changed.len(), 2, "{rows:?}");
    assert_eq!(
        changed[0].as_str(),
        format!(
            "Kind and Use changed by owner: Password, {} -> API key, {}",
            first.origin, first.origin
        )
    );
    assert!(!rows.iter().any(|row| row.contains(KEY)));
    f.state.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn a_saved_card_and_a_2fa_key_refuse_a_change_of_kind_with_the_reason() {
    crate::test_support::isolated_env_test!();
    let f = fixture();
    unlock(&f);
    let api = origin().await;
    let card = create(
        &f,
        "Payment card",
        crate::config::VaultUseBinding {
            target: VaultUseTarget::AnySite,
            value_kind: VaultValueKind::PaymentCard,
        },
        &json!({"number":"5555555555554444","expiry_month":11,"expiry_year":2098,"security_code":"9081","name":"Grace Hopper"}).to_string(),
    )
    .await;
    let refused = update(&f, &card, "Password", site(&api.origin))
        .await
        .unwrap_err();
    assert!(
        refused.contains("A Payment card keeps its Kind and Use"),
        "{refused}"
    );
    let password = create(&f, "Password", site(&api.origin), KEY).await;
    let refused = update(
        &f,
        &password,
        "Payment card",
        VaultUseBinding {
            target: VaultUseTarget::AnySite,
            value_kind: VaultValueKind::PaymentCard,
        },
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("cannot become a Payment card"),
        "{refused}"
    );
    let refused = update(
        &f,
        &password,
        "Password",
        VaultUseBinding {
            target: VaultUseTarget::File {
                path: "/work/x".into(),
            },
            value_kind: VaultValueKind::Secret,
        },
    )
    .await
    .unwrap_err();
    assert!(refused.contains("Choose the matching kind"), "{refused}");
    // Nothing changed: the Password still fills at its site.
    assert_eq!(
        crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .get(&password)
            .unwrap()
            .unwrap()
            .allowed_uses,
        vec![crate::config::UserCredentialUse::Browser]
    );
    f.state.shutdown_cleanup().await.unwrap();
}
