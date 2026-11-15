//! MD-5: browser receipts retain mutation deduplication, never old Vault observations.
use super::kernel_browser_runtime::host_error;
use super::KernelRuntimeState;
use crate::{
    error::DaemonError,
    local::{KernelBrowserCommand, LocalDaemonRequest},
    runtime::{command::KernelCommand, kernel_browser_host::KernelBrowserHost},
};
impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn install_kernel_computer_fixture(
        &self,
        user: &str,
        script: &std::path::Path,
        root: &std::path::Path,
    ) {
        self.owned
            .kernel_browser_host
            .install_fixture_backend(user, script, root);
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
    pub(crate) fn register_kernel_room_fixture_value(&self, room: &str, value: &str) {
        self.owned
            .room_secret_observations
            .register(room, value)
            .unwrap();
    }

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
                | KernelBrowserCommand::GrantLeasedComputer { .. }
                | KernelBrowserCommand::RevokeLeasedComputer { .. }
                | KernelBrowserCommand::GrantRoomComputer { .. }
                | KernelBrowserCommand::ListGrants
                | KernelBrowserCommand::SubscribeGrants { .. }
                | KernelBrowserCommand::RevokeGrants { .. }
        ) {
            return Ok(None);
        }
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(&user);
        protection.require(&scope, false)?;
        self.kernel_browser_policy_revision(&user).map(Some)
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
                | KernelBrowserCommand::GrantLeasedComputer { .. }
                | KernelBrowserCommand::RevokeLeasedComputer { .. }
                | KernelBrowserCommand::GrantRoomComputer { .. }
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
        if revision != Some(self.kernel_browser_policy_revision(&user)?) {
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
        self.install_unstarted_kernel_browser_fixture(user, root);
        // MP-11: fixture setup explicitly starts.
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
    /// The controller exists but nothing ran yet: a first read finds it stopped.
    #[cfg(test)]
    pub(crate) fn install_unstarted_kernel_browser_fixture(
        &self,
        user: &str,
        root: &std::path::Path,
    ) {
        let script = root.join("MD5-controller.sh");
        std::fs::write(&script, r##"set -eu
while IFS= read -r request; do
 printf '%s\n' "$request" >> "$1/requests.log"
 id=${request#*:}; id=${id%%,*}
 case "$request" in
  *'"method":"health"'*) printf '{"id":%s,"ok":true,"result":{"state":"ready","process_id":%s,"diagnostic_code":null}}\n' "$id" "$$" ;;
  *'"method":"host.revoke_subscriptions"'*) printf 'revoked\n' > "$1/subscriptions-revoked"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.downloads.cleanup"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.protect"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.computer.retire"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"host.browser.retire_input"'*) printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"op":"state_signal"'*) after=${request#*'"after":'}; after=${after%%,*}; ms=${request#*'"wait_ms":'}; ms=${ms%%[!0-9]*}; ( n=0; while test "$(cat "$1/signal" 2>/dev/null || echo 0)" = "$after" && test "$n" -lt $((ms / 10)); do sleep 0.01; n=$((n+1)); done; printf '{"id":%s,"ok":true,"result":{"signal":%s}}\n' "$id" "$(cat "$1/signal" 2>/dev/null || echo 0)" ) & ;;
  *'"state_wait":true'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-fixture","document_id":"document","title":"%s"}],"_state_signal":%s}}\n' "$id" "$(cat "$1/title" 2>/dev/null || echo first)" "$(cat "$1/signal" 2>/dev/null || echo 0)" ;;
  *'"op":"start"'*) printf 'start\n' >> "$1/starts"; printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-fixture","document_id":"document"}]}}\n' "$id" ;;
  *'"op":"state"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tabs":[{"tab_id":"host-tab-fixture","document_id":"document"}]}}\n' "$id" ;;
  *'"op":"open"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tab_id":"host-tab-new","tabs":[{"tab_id":"host-tab-fixture","document_id":"document"},{"tab_id":"host-tab-new","document_id":"new-document"}]}}\n' "$id" ;;
  *'"op":"snapshot"'*) printf '{"id":%s,"ok":true,"result":{"snapshot":{"browser_generation":1,"target_id":"target","document_id":"document","snapshot_revision":1,"accessibility_nodes":[],"dom_nodes":[{"node_ref":"backend:1","parent_ref":null,"document_index":0,"node_type":3,"node_name":"#text","text":"MD5-sensitive-fixture","attributes":{},"bounds":null}]}}}\n' "$id" ;;
  *'"op":"screenshot"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tab_id":"host-tab-fixture","document_id":"document","mime_type":"image/png","data_base64":"%s","width":2560,"height":1600}}\n' "$id" "$(cat "$1/screenshot.b64" 2>/dev/null || echo cGl4ZWxzLWZpeHR1cmU=)" ;;
  *'"op":"wait"'*) if test -e "$1/matched"; then m=true; else m=false; fi; printf '{"id":%s,"ok":true,"result":{"generation":1,"matched":%s,"waited_ms":0,"tabs":[{"tab_id":"host-tab-fixture","document_id":"document"}]}}\n' "$id" "$m" ;;
  *'"op":"save"'*) printf '{"id":%s,"ok":true,"result":{"generation":1,"tab_id":"host-tab-fixture","document_id":"document","path":"images/logo.png","bytes":4,"mime_type":"image/png","sha256":"00"}}\n' "$id" ;;
  *'"op":"input"'*) printf 'input\n' > "$1/input"; printf '{"id":%s,"ok":true,"result":{}}\n' "$id" ;;
  *'"method":"shutdown"'*) printf '{"id":%s,"ok":true,"result":{"state":"stopped","process_id":null,"diagnostic_code":null}}\n' "$id"; exit 0 ;;
 esac
done
"##).unwrap();
        self.owned
            .kernel_browser_host
            .install_fixture_backend(user, &script, root);
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
