use super::*;

#[tokio::test]
async fn cold_restart_releases_open_kernel_access_decisions() {
    let scratch = std::env::temp_dir().join(format!("dc-r-{}", std::process::id()));
    std::fs::create_dir_all(scratch.join("home/.chariox")).unwrap();
    std::fs::write(
        scratch.join("home/.chariox/config.toml"),
        format!(
            "[credential_vault]\nbackend = \"chariox_encrypted\"\npath = \"{}\"\nservice = \"chariox\"\nunlock_policy = \"kernel_init\"\n",
            scratch.join("vault/vault.json").display()
        ),
    )
    .unwrap();
    let mut kernel = start_laptop_kernel(&scratch, &[]);
    let mut terminal = connect(&kernel, Duration::from_secs(60)).await;
    request(
        &mut terminal,
        json!({"CreateCredentialVault": {"origin": {"kind": "passphrase",
            "passphrase": "decision restart fixture"}}}),
    )
    .await;
    let before = request_access(&kernel, &mut terminal).await;
    drop(terminal);
    // Force the process to end with the owner decision still open.
    kernel.child.kill().unwrap();
    kernel.child.wait().unwrap();
    let restarted = start_laptop_kernel(&scratch, &[]);
    let mut terminal = connect(&restarted, Duration::from_secs(60)).await;
    assert_eq!(
        request(&mut terminal, json!({"ListOwnerInteractions": {}})).await
            ["OwnerInteractionsListed"]["interactions"],
        json!([]),
    );
    let after = request_access(&restarted, &mut terminal).await;
    assert_ne!(before, after);
}

async fn request_access(kernel: &Kernel, terminal: &mut Socket) -> String {
    let stream = tokio::net::UnixStream::connect(kernel.scratch.join("k.sock"))
        .await
        .unwrap();
    let (mut unix, _) = tokio_tungstenite::client_async("ws://localhost/kernel", stream)
        .await
        .unwrap();
    request(
        &mut unix,
        json!({"RequestKernelAccess": {
            "holder_pid": std::process::id(), "term": "until_holder_exits", "no_wait": true
        }}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let result = request(terminal, json!({"ListOwnerInteractions": {}})).await;
            let cards = result["OwnerInteractionsListed"]["interactions"]
                .as_array()
                .unwrap();
            if let [card] = cards.as_slice() {
                assert_eq!(card["session_id"], "kernel-access");
                return card["interaction_id"].as_str().unwrap().to_owned();
            }
            assert!(
                cards.is_empty(),
                "expected one access request, found {cards:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the access request did not reach the owner")
}
