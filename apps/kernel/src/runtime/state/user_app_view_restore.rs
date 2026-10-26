//! MP-08/MP-10/MP-11: an owner's App Tabs come back after its kernel browser
//! or the kernel restarts, like Room App views after a cold start. Each reopens
//! as a new instance of the installation's current verified release; an old
//! instance id or document never regains authority. Explicit closes end intent.
use super::KernelRuntimeState;
use crate::local::AppRequestErrorCode;

const FILE: &str = "app-tabs.json";
/// One read-modify-write of an `app-tabs.json` at a time: a user's aliases share it.
static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Saved {
    apps: Vec<SavedApp>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedApp {
    owner: String,
    installation_id: String,
}

impl KernelRuntimeState {
    fn app_tabs_path(&self, owner: &str) -> std::path::PathBuf {
        self.owned
            .kernel_browser_host
            .profile_root(&self.provider_account_authority_owner_user_id(owner))
            .join(FILE)
    }

    fn saved_app_tabs(&self, owner: &str) -> Saved {
        std::fs::read(self.app_tabs_path(owner))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Load earlier intent once, settling any Tabs already explicitly bound.
    pub(super) fn seed_user_app_tabs(&self, owner: &str) {
        let views = self.app_control().user_views();
        if views.has_restore(owner).is_some() {
            return;
        }
        let saved = self.saved_app_tabs(owner).apps.into_iter();
        views.seed_restore(
            owner,
            saved
                .filter(|app| app.owner == owner)
                .map(|app| app.installation_id)
                .collect(),
        );
    }

    /// Record which Apps the owner keeps open, beside its browser profile.
    pub(super) fn remember_user_app_tabs(&self, owner: &str) {
        self.seed_user_app_tabs(owner);
        let _saving = SAVING.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.app_tabs_path(owner);
        self.app_control()
            .user_views()
            .save_restore_intent(owner, |intent| {
                let mut saved = self.saved_app_tabs(owner);
                saved.apps.retain(|app| app.owner != owner);
                saved
                    .apps
                    .extend(intent.into_iter().map(|installation_id| SavedApp {
                        owner: owner.into(),
                        installation_id,
                    }));
                let Some(parent) = path.parent() else { return };
                let temporary = path.with_extension(format!("{:016x}.tmp", rand::random::<u64>()));
                let written = std::fs::create_dir_all(parent)
                    .and_then(|()| {
                        std::fs::write(&temporary, serde_json::to_vec(&saved).unwrap_or_default())
                    })
                    .and_then(|()| std::fs::rename(&temporary, &path));
                if written.is_err() {
                    let _ = std::fs::remove_file(&temporary);
                    tracing::debug!("MP-08: App Tab restore intent was not saved");
                }
            });
    }

    /// After the owner's browser served a request, reopen Apps it left open.
    pub(super) fn restore_user_app_tabs(&self, user: &str) {
        let views = self.app_control().user_views();
        for owner in self.user_domain_owner_aliases(user) {
            self.seed_user_app_tabs(&owner);
            if views.has_restore(&owner) != Some(true) || !views.begin_restore(&owner) {
                continue;
            }
            let state = self.clone();
            tokio::spawn(async move {
                let views = state.app_control().user_views();
                for installation in views.restore_batch(&owner) {
                    // Explicit opens may have settled this intent since the pass
                    // began, and shutdown stops a pass before its next open.
                    if !views.restore_batch(&owner).contains(&installation) {
                        continue;
                    }
                    match state.restore_user_app_view(&owner, &installation).await {
                        // Uninstalled or no longer the owner's: nothing to restore.
                        Ok(Ok(_)) => {}
                        Ok(Err(
                            AppRequestErrorCode::NotFound
                            | AppRequestErrorCode::Unauthorized
                            | AppRequestErrorCode::InvalidRequest,
                        )) => {
                            views.settle_restore(&owner, &installation);
                            state.remember_user_app_tabs(&owner);
                        }
                        _ => {}
                    }
                }
                state.remember_user_app_tabs(&owner);
                views.end_restore(&owner);
            });
        }
    }
}
