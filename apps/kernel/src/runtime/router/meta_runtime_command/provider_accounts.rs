use super::result::meta_command_error;
use super::*;

pub(super) fn account_copy_request(tokens: &[String]) -> Result<LocalDaemonRequest, DaemonError> {
    let usage = "provider accounts copy-offers | provider accounts <copy-from|copy-to> <provider> <profile> <kernel-id> <key-thumbprint> <operation-id> [--replace]";
    match tokens {
        [_, accounts, verb] if accounts == "accounts" && verb == "copy-offers" => {
            Ok(LocalDaemonRequest::ListProviderAccountCopyOffers(
                crate::local::ListProviderAccountCopyOffersRequest {},
            ))
        }
        [_, accounts, verb, provider, profile, kernel, key, operation, rest @ ..]
            if accounts == "accounts"
                && matches!(verb.as_str(), "copy-from" | "copy-to")
                && (rest.is_empty() || rest == ["--replace"]) =>
        {
            if verb == "copy-from" {
                Ok(LocalDaemonRequest::CopyProviderAccountFromKernel(
                    crate::local::CopyProviderAccountFromKernelRequest {
                        provider: provider.clone(),
                        account_profile: profile.clone(),
                        source_kernel_id: kernel.clone(),
                        source_key_thumbprint: key.clone(),
                        operation_id: operation.clone(),
                        replace: !rest.is_empty(),
                    },
                ))
            } else {
                Ok(LocalDaemonRequest::CopyProviderAccountToKernel(
                    crate::local::CopyProviderAccountToKernelRequest {
                        provider: provider.clone(),
                        account_profile: profile.clone(),
                        target_kernel_id: kernel.clone(),
                        target_key_thumbprint: key.clone(),
                        operation_id: operation.clone(),
                        replace: !rest.is_empty(),
                    },
                ))
            }
        }
        _ => Err(meta_command_error(usage)),
    }
}

impl CommandRouter {
    pub(super) async fn account_copy_targets_command(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
        command: &str,
        tokens: &[String],
    ) -> Option<RuntimeToolResult> {
        if !matches!(tokens, [provider, accounts, verb] if provider == "provider" && accounts == "accounts" && verb == "copy-targets")
        {
            return None;
        }
        let result = async {
            if self.runtime_state.drives_as_room_guest(session, caller)
                || !self.runtime_state.acts_for_kernel_owner(caller.owner_user_id()) {
                return Err(crate::runtime::room_tool_admission::denied("Use an agent with the owner's kernel access to copy accounts"));
            }
            let config = self.runtime_state.config_snapshot().await;
            let peers = crate::managed_context::owner_authority::account_copy_peers(&config).await?;
            Ok(serde_json::json!({"type": "ProviderAccountCopyTargets", "targets": peers.into_iter().map(|peer|
                serde_json::json!({"kernel_id":peer.kernel_id,"machine_id":peer.machine_id,"key_thumbprint":peer.key_thumbprint})
            ).collect::<Vec<_>>()}))
        }.await;
        Some(match result {
            Ok(response) => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"command":command,"response":response}),
            },
            Err(error) => super::result::meta_command_failure_result(command, error),
        })
    }
}
