//! MD-5: browser receipts retain mutation deduplication, never old Vault observations.
use super::kernel_browser_runtime::host_error;
use super::KernelRuntimeState;
use crate::{
    error::DaemonError,
    local::{KernelBrowserCommand, LocalDaemonRequest},
    runtime::{command::KernelCommand, kernel_browser_host::KernelBrowserHost},
};
impl KernelRuntimeState {
    // Do not wait for a barrier in the socket reader: it must keep detecting disconnect.
    pub(crate) fn kernel_browser_receipt_revision(
        &self,
        caller: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<Option<u64>, DaemonError> {
        let LocalDaemonRequest::KernelBrowser(request) = request else {
            return Ok(None);
        };
        let (user, _) = self.kernel_browser_terminal_context(caller)?;
        if matches!(
            request.command,
            KernelBrowserCommand::Stop
                | KernelBrowserCommand::ListGrants
                | KernelBrowserCommand::SubscribeGrants { .. }
                | KernelBrowserCommand::RevokeGrants { .. }
        ) {
            return Ok(None);
        }
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(&user);
        protection.require(&scope, false)?;
        protection.revision(&scope).map(Some)
    }
    pub(crate) async fn validate_kernel_browser_receipt(
        &self,
        caller: &KernelCommand,
        request: &LocalDaemonRequest,
        revision: Option<u64>,
    ) -> Result<(), DaemonError> {
        let LocalDaemonRequest::KernelBrowser(request) = request else {
            return Ok(());
        };
        let (user, _) = self.kernel_browser_terminal_context(caller)?;
        if matches!(
            request.command,
            KernelBrowserCommand::Stop
                | KernelBrowserCommand::ListGrants
                | KernelBrowserCommand::SubscribeGrants { .. }
                | KernelBrowserCommand::RevokeGrants { .. }
        ) {
            return Ok(());
        }
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(&user);
        let _barrier = protection.barrier(&scope)?.read_owned().await;
        protection.require(&scope, false)?;
        if revision != Some(protection.revision(&scope)?) {
            return Err(host_error(
                "MD-5: browser receipt protection changed; fetch a fresh observation".into(),
            ));
        }
        // The caller may have disconnected while waiting for secret input to finish.
        self.kernel_browser_terminal_context(caller)?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn install_kernel_browser_fixture(&self, user: &str, root: &std::path::Path) {
        let script = root.join("MD5-controller.sh");
        std::fs::write(&script, r#"set -eu
while IFS= read -r request; do
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.revoke_subscriptions"'*) printf 'revoked\n' > "$1/subscriptions-revoked"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"op":"state"'*|*'"op":"start"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-fixture","document_id":"document"}]}}\n' "$id" ;;
  *'"op":"open"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tab_id":"host-tab-new","tabs":[{"tab_id":"host-tab-fixture","document_id":"document"},{"tab_id":"host-tab-new","document_id":"new-document"}]}}\n' "$id" ;;
  *'"op":"snapshot"'*) printf '{"id":%s,"ok":true,"result":{"snapshot":{"text":"MD5-sensitive-fixture"}}}\n' "$id" ;;
  *'"op":"subscribe"'*) printf '{"id":%s,"ok":true,"result":{"subscription_id":"fixture-stream"}}\n' "$id" ;;
  *'"op":"poll"'*) printf '{"id":%s,"ok":true,"result":{"frame":null}}\n' "$id" ;;
  *'"op":"screenshot"'*) printf '{"id":%s,"ok":true,"result":{"data_base64":"cGl4ZWxzLWZpeHR1cmU="}}\n' "$id" ;;
  *'"op":"input"'*) printf 'input\n' > "$1/input"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"#).unwrap();
        self.owned
            .kernel_browser_host
            .install_fixture_backend(user, &script, root);
        // MP-11: fixture setup explicitly starts; observation reads never do.
        self.owned
            .kernel_browser_host
            .protected_request(
                user,
                None,
                "host.browser",
                serde_json::json!({"op":"start"}),
                serde_json::json!({"unknown":false,"values":[],"targets":[]}),
            )
            .unwrap();
    }
    #[cfg(test)]
    pub(crate) fn kernel_browser_fixture_actors(&self, user: &str) -> serde_json::Value {
        self.owned.kernel_browser_host.actor_snapshot(user).unwrap()
    }
    #[cfg(test)]
    pub(crate) fn kernel_browser_fixture_barrier(
        &self,
        user: &str,
    ) -> std::sync::Arc<tokio::sync::RwLock<()>> {
        self.owned
            .kernel_browser_secret_observations
            .barrier(&KernelBrowserHost::profile_key(user))
            .unwrap()
    }
    #[cfg(test)]
    pub(crate) fn register_kernel_browser_fixture_value(&self, user: &str, value: &str) {
        self.owned
            .kernel_browser_secret_observations
            .register(&KernelBrowserHost::profile_key(user), value)
            .unwrap();
    }
    #[cfg(test)]
    pub(crate) fn fence_kernel_browser_fixture(&self, user: &str) {
        assert!(self
            .owned
            .kernel_browser_secret_observations
            .register(&KernelBrowserHost::profile_key(user), "")
            .is_err());
    }
}
