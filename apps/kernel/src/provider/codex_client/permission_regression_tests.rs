use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::provider::ProviderWriteAccessMode;

use super::{CodexClient, JsonRpcMessage};

struct TemporaryPermissionRoots {
    root: PathBuf,
}

impl TemporaryPermissionRoots {
    fn new(test_name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-pr364-permission-{test_name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("permission regression root should be created");
        Self { root }
    }

    fn selected_project(&self) -> PathBuf {
        self.root.join("selected-project")
    }

    fn provider_account_root(&self) -> PathBuf {
        self.root.join("provider-account")
    }

    fn create_layout(&self) {
        std::fs::create_dir_all(self.selected_project().join("src"))
            .expect("selected project should be created");
        std::fs::create_dir_all(&self.provider_account_root())
            .expect("provider account root should be created");
        std::fs::write(
            self.selected_project().join("src/main.rs"),
            b"fn main() {}\n",
        )
        .expect("selected project file should be created");
        std::fs::write(
            self.provider_account_root().join("credentials.json"),
            b"secret",
        )
        .expect("provider account fixture should be created");
    }
}

impl Drop for TemporaryPermissionRoots {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn discovery_client(selected_project: &Path) -> CodexClient {
    CodexClient::new("pr364-permission-run", "ws://127.0.0.1:43123")
        .expect("client should construct")
        .with_write_access_mode(ProviderWriteAccessMode::WorkspaceLiveSyncTracked)
        .with_workspace_live_sync_roots(&[selected_project.to_path_buf()])
        .with_read_only_discovery_permissions()
}

fn permission_request(network: Option<Value>, reads: Vec<String>) -> JsonRpcMessage {
    let mut permissions = serde_json::Map::new();
    if let Some(network) = network {
        permissions.insert("network".to_string(), network);
    }
    permissions.insert("fileSystem".to_string(), json!({ "read": reads }));
    JsonRpcMessage {
        id: Some(json!(1)),
        method: Some("item/permissions/requestApproval".to_string()),
        params: Some(json!({ "permissions": Value::Object(permissions) })),
        result: None,
        error: None,
    }
}

#[test]
fn pr364_read_only_discovery_denies_provider_account_root_read() {
    let roots = TemporaryPermissionRoots::new("account-root");
    roots.create_layout();
    let selected_project = roots.selected_project();
    let provider_account_root = roots.provider_account_root();
    let client = discovery_client(&selected_project);
    let message = permission_request(
        None,
        vec![
            selected_project.join("src/main.rs").display().to_string(),
            provider_account_root
                .join("credentials.json")
                .display()
                .to_string(),
        ],
    );

    assert_eq!(
        client.permissions_approval_response(&message),
        json!({
            "permissions": {
                "fileSystem": {
                    "read": [selected_project.join("src/main.rs").display().to_string()]
                }
            },
            "scope": "turn"
        })
    );
}

#[test]
fn pr364_read_only_discovery_denies_network_permission() {
    let roots = TemporaryPermissionRoots::new("network");
    roots.create_layout();
    let selected_project = roots.selected_project();
    let client = discovery_client(&selected_project);
    let selected_file = selected_project.join("src/main.rs");
    let message = permission_request(Some(json!(true)), vec![selected_file.display().to_string()]);

    assert_eq!(
        client.permissions_approval_response(&message),
        json!({
            "permissions": {
                "fileSystem": {
                    "read": [selected_file.display().to_string()]
                }
            },
            "scope": "turn"
        })
    );
}

#[cfg(unix)]
#[test]
fn pr364_read_only_discovery_denies_symlink_escape_but_preserves_selected_project_reads() {
    let roots = TemporaryPermissionRoots::new("symlink");
    roots.create_layout();
    let selected_project = roots.selected_project();
    let provider_account_root = roots.provider_account_root();
    let linked_account = selected_project.join("linked-account");
    std::os::unix::fs::symlink(&provider_account_root, &linked_account)
        .expect("permission regression symlink should be created");
    let selected_file = selected_project.join("src/main.rs");
    let escaped_file = linked_account.join("credentials.json");
    let client = discovery_client(&selected_project);
    let message = permission_request(
        None,
        vec![
            selected_file.display().to_string(),
            escaped_file.display().to_string(),
        ],
    );

    assert_eq!(
        client.permissions_approval_response(&message),
        json!({
            "permissions": {
                "fileSystem": {
                    "read": [selected_file.display().to_string()]
                }
            },
            "scope": "turn"
        })
    );
}
