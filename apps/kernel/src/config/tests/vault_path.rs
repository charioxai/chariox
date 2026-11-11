use super::*;

#[test]
fn a_kernel_with_its_own_home_keeps_its_vault_inside_it() {
    crate::test_support::isolated_env_test!();
    let user_home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let shared = user_home.join(".chariox/vault/vault.json");
    let home = user_home.join("reviewer-kernel");
    std::env::set_var("CHARIOX_HOME", &home);
    let own = home.join("vault/vault.json");

    let default = UserCredentialVaultConfig::default();
    assert_eq!(std::path::Path::new(&default.path), own);
    assert_ne!(own, shared);

    let parsed: UserCredentialVaultConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(std::path::Path::new(&parsed.path), own);
    assert!(CharioxUserConfig::default().validate().is_ok());
}

#[test]
fn a_kernel_without_its_own_home_keeps_the_shared_vault() {
    crate::test_support::isolated_env_test!();
    std::env::remove_var("CHARIOX_HOME");
    assert_eq!(
        UserCredentialVaultConfig::default().path,
        "~/.chariox/vault/vault.json"
    );
}
