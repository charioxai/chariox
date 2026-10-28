use super::*;

impl KernelRuntimeState {
    pub(crate) async fn wait_for_kernel_access_holder_exit(&self) {
        use futures_util::StreamExt;
        let changed = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .holder_changed
            .clone();
        let admitted = changed.notified();
        tokio::pin!(admitted);
        admitted.as_mut().enable();
        let holders = {
            let state = self
                .owned
                .kernel_access
                .lock()
                .expect("access state poisoned");
            let mut holders = Vec::new();
            for grant in state.grants.values() {
                if !holders.contains(&grant.holder) {
                    holders.push(grant.holder.clone());
                }
            }
            holders
        };
        let mut exits: futures_util::stream::FuturesUnordered<_> =
            holders.iter().map(ProcessIdentity::wait_for_exit).collect();
        tokio::select! {
            _ = admitted => {}
            _ = exits.next(), if !exits.is_empty() => {}
        }
    }
}
