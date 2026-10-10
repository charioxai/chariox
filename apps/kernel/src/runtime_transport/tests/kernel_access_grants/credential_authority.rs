use super::*;

async fn vault_prompt(kernel: &mut Kernel, prefix: &str) -> String {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(bytes) = std::fs::read(kernel.root.join("batch-result")) {
                let result: Value = serde_json::from_slice(&bytes).unwrap();
                panic!(
                    "batch finished before the vault wait: {}",
                    result["failures"]
                );
            }
            for prompt in kernel.prompts().await {
                if let Some(id) = prompt["id"].as_str().filter(|id| id.starts_with(prefix)) {
                    return id.to_owned();
                }
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

fn answer(id: &str, choice: &str, reply: Option<&str>) -> Value {
    serde_json::json!({"RespondToInteraction":{
        "session_id":SESSION,"interaction_id":id,"choice_id":choice,"custom_reply":reply,
    }})
}

async fn wait_file(kernel: &Kernel, name: &str) -> Vec<u8> {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(bytes) = std::fs::read(kernel.root.join(name)) {
                return bytes;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn kernel_access_external_holder_cannot_manage_vault_or_choose_new_passkey() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let before = std::fs::read(kernel.root.join("vault.json")).unwrap();
    kernel.control("vault-manage").await;
    kernel.control("vault-check").await;
    let status = std::fs::read(kernel.root.join("vault-status")).unwrap();
    let id = vault_prompt(&mut kernel, "vault-manage-").await;
    for choice in [
        "lock_now",
        "extend_30m",
        "extend_60m",
        "change_passphrase",
        "dismiss",
    ] {
        let response = holder.request(answer(&id, choice, None));
        assert!(
            response["error"].is_object(),
            "external vault answer accepted: {response}"
        );
    }
    kernel.control("vault-check").await;
    assert!(status == std::fs::read(kernel.root.join("vault-status")).unwrap());
    assert!(kernel.request(answer(&id, "change_passphrase", None)).await["error"].is_null());
    for (step, reply) in [
        ("current", PASSKEY),
        ("new", "Owner New Test Passkey"),
        ("repeat", "Owner New Test Passkey"),
    ] {
        let id = vault_prompt(&mut kernel, &format!("vault-passphrase-{step}-")).await;
        for request in [
            answer(&id, "passphrase", Some("External Chosen Test Passkey")),
            answer(&id, "cancel", None),
        ] {
            let response = holder.request(request);
            assert!(
                response["error"].is_object(),
                "external passphrase answer accepted: {response}"
            );
        }
        let request = if step == "repeat" {
            answer(&id, "cancel", None)
        } else {
            answer(&id, "passphrase", Some(reply))
        };
        assert!(kernel.request(request).await["error"].is_null());
    }
    assert_eq!(wait_file(&kernel, "vault-result").await, b"cancelled");
    kernel.control("vault-check").await;
    assert!(status == std::fs::read(kernel.root.join("vault-status")).unwrap());
    assert!(before == std::fs::read(kernel.root.join("vault.json")).unwrap());
    assert!(
        holder.request(serde_json::json!({"GetSessionState":{"session_id":SESSION}}))["error"]
            .is_null()
    );
    kernel.control("agent-question").await;
    // MP-08/MP-11: ordinary external grants may answer routine questions;
    // the Vault/passkey interactions above remain owner-only.
    assert!(holder.request(answer("routine-access-question", "continue", None))["error"].is_null());
}

#[tokio::test]
async fn kernel_access_external_holder_cannot_answer_vault_unlock() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    kernel.control("vault-unlock").await;
    let id = vault_prompt(&mut kernel, "vault-unlock-").await;
    let response = holder.request(answer(&id, "passphrase", Some(PASSKEY)));
    assert!(
        response["error"].is_object(),
        "external vault unlock accepted: {response}"
    );
    assert!(kernel.request(answer(&id, "cancel", None)).await["error"].is_null());
    assert_eq!(wait_file(&kernel, "vault-result").await, b"cancelled");
    kernel.control("vault-check").await;
    let status: Value =
        serde_json::from_slice(&std::fs::read(kernel.root.join("vault-status")).unwrap()).unwrap();
    assert_eq!(status["unlocked"], false);
}

#[tokio::test]
async fn kernel_access_revocation_stops_waiting_and_deferred_provider_launches() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    let grant_id = grant(&mut kernel, &mut holder).await;
    kernel.control("provider-batch").await;
    let id = vault_prompt(&mut kernel, "vault-unlock-").await;
    let revoked = kernel
        .request(
            serde_json::to_value(LocalDaemonRequest::RevokeKernelAccessGrant(
                crate::local::RevokeKernelAccessGrantRequest {
                    grant_id: Some(grant_id),
                },
            ))
            .unwrap(),
        )
        .await;
    assert!(revoked["error"].is_null(), "{revoked}");
    assert!(kernel
        .request(answer(&id, "passphrase", Some(PASSKEY)))
        .await["error"]
        .is_null());
    let result: Value = serde_json::from_slice(&wait_file(&kernel, "batch-result").await).unwrap();
    assert_eq!(result["provider_run_count"], 0);
    let failures = result["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2, "{result}");
    assert!(
        failures.iter().all(|failure| failure
            .as_str()
            .unwrap()
            .contains("grant revoked or expired")),
        "{result}"
    );
}
