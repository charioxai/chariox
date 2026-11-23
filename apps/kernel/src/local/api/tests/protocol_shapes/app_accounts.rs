//! GHA-1: the owner binds accounts to the credentialed destinations of an App.
use crate::local::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn app_accounts_contract_snapshot() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let requests = [
        json!({"GrantAppAccount":{"installation_id":"github","account":"work","credential":"github","vault_credential_id":"cpy-1","allow":"write"}}),
        json!({"RevokeAppAccount":{"installation_id":"github","account":"work"}}),
        json!({"ListAppAccounts":{"installation_id":"github"}}),
    ];
    for value in &requests {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), *value);
    }
    let responses = [json!({"AppAccounts":{"installation_id":"github",
        "credentials":[{"credential":"github","origin":"https://api.github.com","routes":40,"safety":"destructive"}],
        "accounts":[{"account":"work","credential":"github","vault_credential_id":"cpy-1","allow":"write","bound_at_ms":5}]}})];
    for value in &responses {
        let response: LocalDaemonResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), *value);
    }
    let snapshot = json!({"local_protocol":LOCAL_DAEMON_PROTOCOL_VERSION,"requests":requests,"responses":responses});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("app-accounts-1011.json")).unwrap();
    assert_eq!(snapshot, expected);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        ),
        "77377523642eeea3d7f07037487c39068ef314a2172a84b6c1b0a9a9a2bbd3b8"
    );
    // The reply names a Vault entry by id and carries no value field at all.
    for forged in [
        json!({"GrantAppAccount":{"installation_id":"github","account":"work","credential":"github","vault_credential_id":"cpy-1","allow":"write","token":"ghp_x"}}),
        json!({"GrantAppAccount":{"installation_id":"github","account":"work","credential":"github","vault_credential_id":"cpy-1","allow":"root"}}),
        json!({"RevokeAppAccount":{"installation_id":"github","account":"work","owner":"someone-else"}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(forged).is_err());
    }
}
