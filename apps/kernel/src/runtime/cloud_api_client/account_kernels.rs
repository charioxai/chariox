use super::http::{decode_cloud_response, post_with_kernel_enrollment};
use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::local::{AccountKernel, AccountKernelStatus};
use serde::Deserialize;

#[derive(Deserialize)]
struct AccountKernelDirectory {
    targets: Vec<DirectoryEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntry {
    id: String,
    daemon_id: String,
    name: Option<String>,
    status: DirectoryStatus,
    last_heartbeat_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum DirectoryStatus {
    Online,
    Offline,
    Stale,
    Draining,
}

pub(crate) async fn list_account_kernels(
    profile: PersistedCloudRelayProfile,
) -> Result<Vec<AccountKernel>, DaemonError> {
    tokio::task::spawn_blocking(move || {
        let response = post_with_kernel_enrollment(
            &profile,
            "/kernels/account/list",
            &serde_json::json!({"accountId": profile.account_id}),
        )?;
        let directory: AccountKernelDirectory = decode_cloud_response(response)?;
        Ok(directory
            .targets
            .into_iter()
            .map(|target| AccountKernel {
                target_id: target.id,
                kernel_id: target.daemon_id,
                name: target.name,
                status: match target.status {
                    DirectoryStatus::Online => AccountKernelStatus::Online,
                    DirectoryStatus::Offline => AccountKernelStatus::Offline,
                    DirectoryStatus::Stale => AccountKernelStatus::Stale,
                    DirectoryStatus::Draining => AccountKernelStatus::Draining,
                },
                last_seen_at: target.last_heartbeat_at,
            })
            .collect())
    })
    .await
    .map_err(join_error)?
}

pub(crate) async fn unlink_account_kernel(
    profile: PersistedCloudRelayProfile,
    target_id: String,
    agent_id: String,
) -> Result<(), DaemonError> {
    tokio::task::spawn_blocking(move || {
        let response = post_with_kernel_enrollment(
            &profile,
            "/kernels/account/unlink",
            &serde_json::json!({"accountId": profile.account_id, "targetId": target_id, "agentId": agent_id}),
        )?;
        if response.status() != 204 {
            return Err(DaemonError::LocalTransport {
                operation: "unlink account kernel",
                message: "The connected service did not acknowledge the unlink; list the account's kernels before retrying".into(),
            });
        }
        Ok(())
    })
    .await
    .map_err(join_error)?
}

fn join_error(error: tokio::task::JoinError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "manage linked kernels",
        message: error.to_string(),
    }
}
