//! `CreateCredentialVault` and `SealCredentialVaultKey`: the owner's client gives a kernel its own Vault, also
//! one that opens with the passphrase of another kernel's Vault.
use super::vault_unlock_request::{caller_of, code_of, Fixture, PASSPHRASE};
use super::*;
use crate::config::CredentialVaultUnlockPolicy;
use crate::local::{
    CreateCredentialVaultRequest, CredentialVaultOrigin, KernelConnectionClass,
    SealCredentialVaultKeyRequest, VaultPassphrase,
};

fn kernel_without_vault() -> Fixture {
    Fixture::new(CredentialVaultUnlockPolicy::KernelInit, false)
}

async fn seal(source: &Fixture, target: &Fixture) -> Result<LocalDaemonResponse, DaemonError> {
    source
        .send(LocalDaemonRequest::SealCredentialVaultKey(
            SealCredentialVaultKeyRequest {
                target_kernel_id: target.kernel_id.clone(),
                target_public_key: target.relay_public_key.clone(),
            },
        ))
        .await
}

async fn sealed(source: &Fixture, target: &Fixture) -> crate::secret::VaultKeySeed {
    match seal(source, target).await.unwrap() {
        LocalDaemonResponse::CredentialVaultKeySealed { seed } => seed,
        other => panic!("unexpected {other:?}"),
    }
}

fn create(origin: CredentialVaultOrigin) -> LocalDaemonRequest {
    LocalDaemonRequest::CreateCredentialVault(CreateCredentialVaultRequest { origin })
}

#[tokio::test]
async fn a_new_kernel_creates_its_own_vault_with_the_passphrase_of_the_kernel_it_was_sent_from() {
    let source = Fixture::locked();
    source.unlock(PASSPHRASE).await.unwrap();
    let target = kernel_without_vault();
    assert!(!target.vault.exists());

    let seed = sealed(&source, &target).await;
    let created = target
        .send(create(CredentialVaultOrigin::Seed {
            seed: Box::new(seed),
        }))
        .await
        .unwrap();
    let LocalDaemonResponse::CredentialVaultUnlocked { status, action } = created else {
        panic!("unexpected {created:?}");
    };
    assert_eq!(action, "created");
    assert!(status.unlocked);
    assert!(
        target.status().await.unlocked,
        "the launch can write into it at once"
    );
    assert_ne!(
        std::fs::read(&target.vault).unwrap(),
        std::fs::read(&source.vault).unwrap(),
        "each kernel keeps its own file"
    );

    crate::secret::lock_chariox_encrypted_vault(&target.vault).unwrap();
    assert!(!target.status().await.unlocked);
    assert_eq!(
        code_of(target.unlock("not the passphrase").await).0,
        "vault_passphrase_wrong"
    );
    target.unlock(PASSPHRASE).await.unwrap();
    assert!(
        target.status().await.unlocked,
        "the same passphrase opens it later"
    );
}

#[tokio::test]
async fn a_new_kernel_creates_its_vault_with_a_passphrase_typed_for_it() {
    let target = kernel_without_vault();
    let request = || {
        create(CredentialVaultOrigin::Passphrase {
            passphrase: VaultPassphrase::new("typed for the new machine"),
        })
    };
    target.send(request()).await.unwrap();
    assert!(target.status().await.unlocked);
    crate::secret::lock_chariox_encrypted_vault(&target.vault).unwrap();
    target.unlock("typed for the new machine").await.unwrap();

    let (code, _) = code_of(target.send(request()).await);
    assert_eq!(code, "vault_exists", "a Vault is never replaced");
}

#[tokio::test]
async fn a_locked_vault_seals_nothing_and_asks_to_be_unlocked() {
    let source = Fixture::locked();
    let target = kernel_without_vault();
    let (code, message) = code_of(seal(&source, &target).await);
    assert_eq!(code, "vault_locked");
    assert_eq!(message, "The Vault is locked. Unlock it, then try again.");
}

#[tokio::test]
async fn a_seed_sealed_for_another_kernel_creates_nothing() {
    let source = Fixture::locked();
    source.unlock(PASSPHRASE).await.unwrap();
    let intended = kernel_without_vault();
    let other = kernel_without_vault();
    let seed = sealed(&source, &intended).await;
    other
        .send(create(CredentialVaultOrigin::Seed {
            seed: Box::new(seed),
        }))
        .await
        .unwrap_err();
    assert!(!other.vault.exists());
}

#[tokio::test]
async fn only_the_owners_own_client_may_create_or_seal() {
    let source = Fixture::locked();
    source.unlock(PASSPHRASE).await.unwrap();
    let target = kernel_without_vault();
    let seed = sealed(&source, &target).await;
    let agent = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(KernelConnectionClass::KernelAgent);
    for request in [
        create(CredentialVaultOrigin::Seed {
            seed: Box::new(seed),
        }),
        LocalDaemonRequest::SealCredentialVaultKey(SealCredentialVaultKeyRequest {
            target_kernel_id: target.kernel_id.clone(),
            target_public_key: target.relay_public_key.clone(),
        }),
    ] {
        let fixture = if matches!(request, LocalDaemonRequest::SealCredentialVaultKey(_)) {
            &source
        } else {
            &target
        };
        let (code, _) = code_of(fixture.send_as(agent.clone(), request).await);
        assert_eq!(code, "vault_unlock_owner_only");
    }
    assert!(!target.vault.exists());
}

#[test]
fn the_requests_and_their_answer_keep_a_secret_out_of_debug_and_command_records() {
    let request = create(CredentialVaultOrigin::Passphrase {
        passphrase: VaultPassphrase::new("sentinel-passphrase-9a2e"),
    });
    assert!(!format!("{request:?}").contains("sentinel-passphrase-9a2e"));
    let recorded = crate::runtime::command::KernelCommand::from_local_request_with_caller(
        "c",
        KernelCommandSource::LocalCli,
        KernelCaller::for_source(&KernelCommandSource::LocalCli),
        None,
        None,
        &request,
    );
    assert!(!format!("{recorded:?}").contains("sentinel-passphrase-9a2e"));
}

#[tokio::test]
async fn a_kernel_with_no_vault_is_awaiting_its_passphrase_until_one_is_chosen() {
    let target = kernel_without_vault();
    let status = target.status().await;
    assert!(status.awaiting_passphrase && !status.unlocked);

    target
        .send(create(CredentialVaultOrigin::Passphrase {
            passphrase: VaultPassphrase::new("chosen on the first visit"),
        }))
        .await
        .unwrap();
    assert!(!target.status().await.awaiting_passphrase);
    crate::secret::lock_chariox_encrypted_vault(&target.vault).unwrap();
    let locked = target.status().await;
    assert!(!locked.awaiting_passphrase && !locked.unlocked);
}

const OWNER: &str = "enrolled-owner";

fn owner_over_the_relay() -> KernelCaller {
    caller_of(Some(KernelConnectionClass::Terminal), Some(OWNER))
}

#[tokio::test]
async fn the_owner_creates_the_vault_from_a_remote_client_with_a_passphrase() {
    let target = Fixture::enrolled(OWNER, false);
    let created = target
        .send_as(
            owner_over_the_relay(),
            create(CredentialVaultOrigin::Passphrase {
                passphrase: VaultPassphrase::new("typed in the browser"),
            }),
        )
        .await
        .unwrap();
    let LocalDaemonResponse::CredentialVaultUnlocked { action, .. } = created else {
        panic!("unexpected {created:?}");
    };
    assert_eq!(action, "created");
    assert!(target.vault.exists());
}

#[tokio::test]
async fn a_remote_client_gives_a_new_kernel_the_passphrase_of_another_of_the_owners_kernels() {
    let source = Fixture::enrolled(OWNER, true);
    source
        .send_as(
            owner_over_the_relay(),
            LocalDaemonRequest::UnlockCredentialVault(crate::local::UnlockCredentialVaultRequest {
                passphrase: VaultPassphrase::new(PASSPHRASE),
                duration: crate::local::VaultUnlockDuration::Minutes { minutes: 30 },
                surface: crate::local::VaultUnlockSurface::VaultWindow,
            }),
        )
        .await
        .unwrap();
    let target = Fixture::enrolled(OWNER, false);
    // The client reads the target's id and relay key from the kernel list, asks the
    // unlocked kernel to seal for them, then hands the seed to the new kernel.
    let seed = match source
        .send_as(
            owner_over_the_relay(),
            LocalDaemonRequest::SealCredentialVaultKey(SealCredentialVaultKeyRequest {
                target_kernel_id: target.kernel_id.clone(),
                target_public_key: target.relay_public_key.clone(),
            }),
        )
        .await
        .unwrap()
    {
        LocalDaemonResponse::CredentialVaultKeySealed { seed } => seed,
        other => panic!("unexpected {other:?}"),
    };
    target
        .send_as(
            owner_over_the_relay(),
            create(CredentialVaultOrigin::Seed {
                seed: Box::new(seed),
            }),
        )
        .await
        .unwrap();
    crate::secret::lock_chariox_encrypted_vault(&target.vault).unwrap();
    target.unlock(PASSPHRASE).await.unwrap();
    assert!(target.status().await.unlocked);
}

#[tokio::test]
async fn a_member_of_a_shared_session_cannot_create_the_owners_vault() {
    let target = Fixture::enrolled(OWNER, false);
    let member = caller_of(
        Some(KernelConnectionClass::Terminal),
        Some("session-member"),
    );
    let (code, _) = code_of(
        target
            .send_as(
                member,
                create(CredentialVaultOrigin::Passphrase {
                    passphrase: VaultPassphrase::new("not the owner"),
                }),
            )
            .await,
    );
    assert_eq!(code, "vault_unlock_owner_only");
    assert!(!target.vault.exists());
}
