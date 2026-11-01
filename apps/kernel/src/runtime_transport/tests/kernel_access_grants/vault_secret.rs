//! An outside agent asks for a Vault value: it gets a link and the outcome, the owner's
//! client takes the value, and nothing on the kernel ever holds it in the clear.
use super::*;
use base64::Engine;
use sha2::{Digest, Sha256};

const CANARY: &str = "canary-8f3a1c52-9d4e-4b7a-a1f0-6e2d3c4b5a69";

const HOST: &str = "api.example.com";

fn request(id: &str, overwrite: bool) -> Value {
    request_for(id, overwrite, &[HOST])
}

fn request_for(id: &str, overwrite: bool, hosts: &[&str]) -> Value {
    serde_json::json!({"RequestVaultSecret":{"credential_id":id,"label":"Drill key","hosts":hosts,"overwrite":overwrite}})
}

/// The owner's own client opens the Vault, as it does before an agent can use it.
async fn unlock(kernel: &mut Kernel) {
    let unlocked = kernel
        .request(serde_json::json!({"UnlockCredentialVault":{"passphrase":PASSKEY}}))
        .await;
    assert!(unlocked["error"].is_null(), "{unlocked}");
}

fn status(interaction_id: &str) -> Value {
    serde_json::json!({"GetVaultSecretStatus":{"interaction_id":interaction_id}})
}

fn answer(id: &str, choice: &str, reply: Option<&str>) -> Value {
    serde_json::json!({"RespondToInteraction":{
        "session_id":crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE,
        "interaction_id":id,"choice_id":choice,"custom_reply":reply,
    }})
}

impl Kernel {
    async fn secret_popup(&mut self, interaction_id: &str) -> Value {
        self.secret_prompts(interaction_id)
            .await
            .into_iter()
            .find(|prompt| prompt["interaction_id"] == interaction_id)
            .unwrap()
    }

    /// Every prompt the owner's feed shows in the frame that first carries `interaction_id`.
    async fn secret_prompts(&mut self, interaction_id: &str) -> Vec<Value> {
        timeout(Duration::from_secs(10), async {
            while let Some(message) = self.popups.next().await {
                if let Ok(Message::Text(text)) = message {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    let event = &frame["event"];
                    if event["event"] == "passkey_prompts_changed" {
                        let prompts = event["prompts"].as_array().cloned().unwrap_or_default();
                        if prompts
                            .iter()
                            .any(|prompt| prompt["interaction_id"] == interaction_id)
                        {
                            return prompts;
                        }
                    }
                }
            }
            panic!("popup socket closed");
        })
        .await
        .unwrap()
    }

    /// The hosts the stored handle is bound to, read from the Vault registry.
    async fn credential_allowed_hosts(&mut self, id: &str) -> Vec<String> {
        self.control(&format!("credential-hosts:{id}")).await;
        std::fs::read_to_string(self.root.join("credential-hosts"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What the real HTTP credential policy says about sending the handle to `url`.
    async fn credential_policy(&mut self, id: &str, url: &str) -> String {
        self.control(&format!("credential-policy:{id}|{url}")).await;
        std::fs::read_to_string(self.root.join("credential-policy")).unwrap()
    }

    /// The stored handle record as the registry holds it.
    async fn credential_record(&mut self, id: &str) -> Value {
        self.control(&format!("credential-record:{id}")).await;
        serde_json::from_str(&std::fs::read_to_string(self.root.join("credential-record")).unwrap())
            .unwrap()
    }

    /// SHA-256 of the value the Vault holds under `key`.
    async fn vault_value_sha256(&mut self, key: &str) -> String {
        self.control(&format!("vault-secret-sha256:{key}")).await;
        std::fs::read_to_string(self.root.join("vault-secret-sha256")).unwrap()
    }

    async fn secret_status(&mut self, holder: &mut Client, interaction_id: &str, want: &str) {
        timeout(Duration::from_secs(10), async {
            loop {
                let reported = holder.request(status(interaction_id));
                assert!(reported["error"].is_null(), "{reported}");
                if reported["response"]["VaultSecretStatusReported"]["status"] == want {
                    return;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the request never became {want}"));
    }
}

fn files_under(root: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, found);
        } else {
            found.push(path);
        }
    }
}

/// What the kernel logged, for the message of a failed request.
fn kernel_log(root: &Path) -> String {
    let mut files = Vec::new();
    files_under(&root.join("state"), &mut files);
    files
        .iter()
        .filter(|file| file.to_string_lossy().contains("log"))
        .map(|file| {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let tail = text.len().saturating_sub(2500);
            format!(
                "{}:\n{}",
                file.display(),
                text.get(tail..).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_value_absent(root: &Path, texts: &[String]) {
    let forms: [String; 2] = [
        CANARY.to_owned(),
        base64::engine::general_purpose::STANDARD.encode(CANARY),
    ];
    let utf16: Vec<u8> = CANARY.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut files = Vec::new();
    files_under(root, &mut files);
    for file in files {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !forms.iter().any(|form| text.contains(form.as_str()))
                && !bytes.windows(utf16.len()).any(|window| window == utf16),
            "the value is in {}",
            file.display()
        );
    }
    for text in texts {
        assert!(
            !forms.iter().any(|form| text.contains(form.as_str())),
            "the value is in a response: {text}"
        );
    }
}

#[tokio::test]
async fn an_outside_agent_gets_a_link_and_the_owner_types_the_value_in_a_client() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;

    let unavailable = holder.request(request("drill_key", false));
    assert!(
        unavailable["error"]["code"] == "secret_link_unavailable",
        "no page host must fail loudly: {unavailable}"
    );

    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(
        holder.request(set.clone())["error"].is_object(),
        "an agent must not choose where the link points"
    );
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let started = holder.request(request("drill_key", false));
    assert!(
        started["error"].is_null(),
        "{started}\n{}",
        kernel_log(&kernel.root)
    );
    let requested = &started["response"]["VaultSecretRequested"];
    let interaction_id = requested["interaction_id"].as_str().unwrap().to_owned();
    assert_eq!(requested["credential_id"], "drill_key");
    assert!(requested["expires_at"].as_u64().unwrap() > crate::session::unix_epoch_ms());
    let link = requested["link"].as_str().unwrap();
    assert!(
        link.starts_with("https://page.test/s/") && link.ends_with(&format!("/{interaction_id}")),
        "{link}"
    );

    let popup = kernel.secret_popup(&interaction_id).await;
    assert_eq!(popup["kind"], "vault_secret");
    let message = popup["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("Drill key, for {HOST}.")),
        "the prompt names the label and the host the key is bound to: {message}"
    );
    assert_eq!(
        popup["session_id"],
        crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE
    );
    kernel
        .secret_status(&mut holder, &interaction_id, "pending")
        .await;

    for refused in [
        answer(&interaction_id, "secret", Some(CANARY)),
        answer(&interaction_id, "cancel", None),
    ] {
        assert!(
            holder.request(refused)["error"].is_object(),
            "an agent answered the owner's secret prompt"
        );
    }
    kernel
        .secret_status(&mut holder, &interaction_id, "pending")
        .await;

    let answered = kernel
        .request(answer(&interaction_id, "secret", Some(CANARY)))
        .await;
    assert!(answered["error"].is_null(), "{answered}");
    kernel
        .secret_status(&mut holder, &interaction_id, "stored")
        .await;

    kernel.control("vault-secret-sha256:drill_key").await;
    assert_eq!(
        std::fs::read_to_string(kernel.root.join("vault-secret-sha256")).unwrap(),
        format!("{:x}", Sha256::digest(CANARY))
    );

    let handle = kernel.credential_allowed_hosts("drill_key").await;
    assert_eq!(
        handle,
        [HOST],
        "the handle is bound to the hosts the agent named"
    );
    assert_eq!(
        kernel
            .credential_policy("drill_key", &format!("https://{HOST}/v1/models"))
            .await,
        "ok",
        "the bound host passes the policy"
    );
    for elsewhere in [
        "https://evil.example.net/v1",
        "https://api.example.com.evil.net/",
        "http://127.0.0.1:9/echo",
    ] {
        assert_eq!(
            kernel.credential_policy("drill_key", elsewhere).await,
            "credential_policy",
            "{elsewhere} must not receive the key"
        );
    }

    let mut texts = vec![answered.to_string()];
    texts.push(holder.request(status(&interaction_id)).to_string());
    texts.push(
        kernel
            .request(serde_json::json!({"GetSessionState":{"session_id":SESSION}}))
            .await
            .to_string(),
    );
    assert_value_absent(&kernel.root, &texts);
}

#[tokio::test]
async fn the_owner_cancels_and_a_second_request_for_the_same_handle_needs_overwrite() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let first = holder.request(request("drill_key", false));
    assert!(
        first["error"].is_null(),
        "{first}\n{}",
        kernel_log(&kernel.root)
    );
    let id = first["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(kernel.request(answer(&id, "cancel", None)).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "cancelled").await;

    let unknown = holder.request(status("secret-unknown"));
    assert!(unknown["error"].is_object(), "{unknown}");

    let second = holder.request(request("drill_key", false));
    let id = second["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(kernel.request(answer(&id, "secret", Some(CANARY))).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "stored").await;

    let third = holder.request(request("drill_key", false));
    assert!(
        third["error"]["code"] == "secret_request_exists",
        "an existing handle needs overwrite: {third}"
    );
    let fourth = holder.request(request("drill_key", true));
    assert!(fourth["error"].is_null(), "{fourth}");
}

#[tokio::test]
async fn a_request_that_names_no_usable_host_is_refused_and_creates_nothing() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let missing = holder.request(
        serde_json::json!({"RequestVaultSecret":{"credential_id":"drill_key","label":"Drill key"}}),
    );
    let mut refusals = vec![missing];
    for hosts in [
        &[][..],
        &["*.example.com"],
        &["https://api.example.com"],
        &["api.example.com/v1"],
        &["API.example.com"],
        &["8.8.8.8"],
        &["api.example.com", ""],
    ] {
        refusals.push(holder.request(request_for("drill_key", false, hosts)));
    }
    for refused in refusals {
        assert!(
            refused["error"]["code"] == "secret_request_invalid",
            "{refused}"
        );
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--host api.example.com"),
            "the refusal names the next step: {refused}"
        );
    }

    let loopback = holder.request(request_for(
        "drill_key",
        false,
        &["127.0.0.1:8080", "localhost", "[::1]:9"],
    ));
    let id = loopback["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap_or_else(|| panic!("loopback hosts are allowed: {loopback}"))
        .to_owned();
    let asked = kernel
        .secret_prompts(&id)
        .await
        .into_iter()
        .filter(|prompt| prompt["kind"] == "vault_secret")
        .count();
    assert_eq!(asked, 1, "a refused request left a prompt behind");
}

#[tokio::test]
async fn overwriting_a_handle_cannot_widen_the_hosts_it_is_bound_to() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let first = holder.request(request("drill_key", false));
    let id = first["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(kernel.request(answer(&id, "secret", Some(CANARY))).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "stored").await;

    for hosts in [&["evil.example.net"][..], &[HOST, "evil.example.net"]] {
        let widened = holder.request(request_for("drill_key", true, hosts));
        assert!(
            widened["error"]["code"] == "secret_request_invalid",
            "overwrite widened the hosts: {widened}"
        );
    }
    assert_eq!(kernel.credential_allowed_hosts("drill_key").await, [HOST]);

    let same = holder.request(request_for("drill_key", true, &[HOST]));
    assert!(same["error"].is_null(), "{same}");
}

#[tokio::test]
async fn overwriting_a_handle_keeps_its_record_and_changes_only_the_value() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    kernel
        .control("seed-credential:owner_key|owner-vault-slot")
        .await;
    let before = kernel.credential_record("owner_key").await;
    assert_eq!(before["injection"]["name"], "x-api-key");
    let seeded = kernel.vault_value_sha256("owner-vault-slot").await;

    let started = holder.request(request("owner_key", true));
    let id = started["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{started}"))
        .to_owned();
    assert!(kernel.request(answer(&id, "secret", Some(CANARY))).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "stored").await;

    assert_eq!(
        kernel.credential_record("owner_key").await,
        before,
        "the overwrite replaced the handle's record"
    );
    let after = kernel.vault_value_sha256("owner-vault-slot").await;
    assert_ne!(
        after, seeded,
        "the value at the handle's own Vault key did not change"
    );
    assert_eq!(
        after,
        format!("{:x}", Sha256::digest(CANARY.as_bytes())),
        "the new value is not at the handle's own Vault key"
    );
    assert_eq!(kernel.credential_allowed_hosts("owner_key").await, [HOST]);
}

#[tokio::test]
async fn a_new_handle_cannot_take_a_vault_slot_another_handle_uses() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    // `owner_key` keeps its value under the Vault key `token_slot`; a new handle named
    // `token_slot` would store under that same key.
    kernel.control("seed-credential:owner_key|token_slot").await;
    let seeded = kernel.vault_value_sha256("token_slot").await;

    let started = holder.request(request("token_slot", false));
    assert!(
        started["error"]["code"] == "secret_request_exists",
        "a new handle was offered another handle's Vault slot: {started}"
    );
    assert_eq!(
        kernel.vault_value_sha256("token_slot").await,
        seeded,
        "the other handle's value was replaced"
    );
}

async fn store_value(kernel: &mut Kernel, key: &str, value: &str) {
    let stored = kernel
        .request(serde_json::json!({"SetCredentialSecret":{"session_id":null,"agent_id":null,"key":key,"value":value}}))
        .await;
    assert!(stored["error"].is_null(), "{stored}");
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[tokio::test]
async fn a_new_handle_cannot_take_a_vault_slot_that_holds_a_value_without_a_record() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    store_value(&mut kernel, "token_slot", "original-value").await;

    let started = holder.request(request("token_slot", false));
    assert!(
        started["error"]["code"] == "secret_request_exists",
        "a new handle was offered a Vault slot that holds a value: {started}"
    );
    assert!(
        started["error"]["message"]
            .as_str()
            .is_some_and(|next| next.contains("overwrite")),
        "the refusal does not say what to do next: {started}"
    );
    assert_eq!(
        kernel.vault_value_sha256("token_slot").await,
        sha256_hex("original-value"),
        "the stored value was replaced"
    );
}

#[tokio::test]
async fn overwrite_replaces_the_value_of_a_vault_slot_that_has_no_record() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    store_value(&mut kernel, "token_slot", "original-value").await;

    let started = holder.request(request("token_slot", true));
    let id = started["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{started}"))
        .to_owned();
    assert_eq!(
        kernel.vault_value_sha256("token_slot").await,
        sha256_hex("original-value"),
        "the value changed before the owner typed anything"
    );
    assert!(kernel.request(answer(&id, "secret", Some(CANARY))).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "stored").await;
    assert_eq!(
        kernel.vault_value_sha256("token_slot").await,
        sha256_hex(CANARY)
    );
    assert_eq!(kernel.credential_allowed_hosts("token_slot").await, [HOST]);
}

#[tokio::test]
async fn a_deleted_handle_leaves_its_vault_value_and_a_new_request_treats_it_as_occupied() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let first = holder.request(request("drill_key", false));
    let id = first["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(kernel.request(answer(&id, "secret", Some(CANARY))).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "stored").await;

    kernel.control("remove-credential-record:drill_key").await;

    let again = holder.request(request("drill_key", false));
    assert!(
        again["error"]["code"] == "secret_request_exists",
        "the value of a deleted handle was offered for replacement: {again}"
    );
    assert_eq!(
        kernel.vault_value_sha256("drill_key").await,
        sha256_hex(CANARY),
        "the value of the deleted handle was lost"
    );

    let replacing = holder.request(request("drill_key", true));
    assert!(replacing["error"].is_null(), "{replacing}");
}

#[tokio::test]
async fn a_bearer_token_longer_than_a_passkey_is_taken_and_stored_whole() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let token = format!("{CANARY}-{}", "t".repeat(1_000 - CANARY.len() - 1));
    assert_eq!(token.chars().count(), 1_000);
    let started = holder.request(request("long_token", false));
    let id = started["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{started}"))
        .to_owned();
    let answered = kernel.request(answer(&id, "secret", Some(&token))).await;
    assert!(answered["error"].is_null(), "{answered}");
    kernel.secret_status(&mut holder, &id, "stored").await;
    assert_eq!(
        kernel.vault_value_sha256("long_token").await,
        sha256_hex(&token)
    );
}

#[tokio::test]
async fn a_default_port_binding_is_accepted_and_usable_on_that_port_only() {
    let mut kernel = Kernel::start().await;
    let mut holder = Client::start(&kernel.root);
    grant(&mut kernel, &mut holder).await;
    let set = serde_json::json!({"SetUserConfigValue":{"path":"secret_link.page_url","value":"https://page.test"}});
    assert!(kernel.request(set).await["error"].is_null());
    unlock(&mut kernel).await;

    let started = holder.request(request_for(
        "drill_key",
        false,
        &["api.example.com:443", "localhost:80", "localhost:8080"],
    ));
    let id = started["response"]["VaultSecretRequested"]["interaction_id"]
        .as_str()
        .unwrap_or_else(|| panic!("default ports are bindable: {started}"))
        .to_owned();
    assert!(kernel.request(answer(&id, "secret", Some(CANARY))).await["error"].is_null());
    kernel.secret_status(&mut holder, &id, "stored").await;

    for url in [
        "https://api.example.com/v1",
        "http://localhost/v1",
        "http://localhost:8080/v1",
    ] {
        assert_eq!(
            kernel.credential_policy("drill_key", url).await,
            "ok",
            "{url}"
        );
    }
    for url in [
        "https://api.example.com:8443/v1",
        "http://api.example.com/v1",
        "http://localhost:9090/v1",
        "https://evil.example.net/v1",
    ] {
        assert_eq!(
            kernel.credential_policy("drill_key", url).await,
            "credential_policy",
            "{url}"
        );
    }
}
