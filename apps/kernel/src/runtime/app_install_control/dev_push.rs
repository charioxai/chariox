//! Development enters the same durable reservation and pump as file installs.
use super::*;
use crate::{
    durable_state::app_installation_operations::{InstallInput, UpdateTarget},
    local::*,
};
use chariox_app_runtime::installation::channel::{AcquisitionSource, InstallationChannel};

impl AppInstallControl {
    pub(crate) async fn dev_push_status(
        &self,
        owner: &str,
        session: &str,
        id: &str,
    ) -> Result<Option<LocalDaemonResponse>, AppRequestErrorCode> {
        let permit = self
            .0
            .shared
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppRequestErrorCode::Busy)?;
        let store = self.0.shared.store.clone();
        let (owner, session, id) = (owner.to_owned(), session.to_owned(), id.to_owned());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match store.first_app_install_status(&owner, &id) {
                Ok(value) => {
                    if !value.input.as_ref().is_some_and(|input| {
                        input.session_id == session && input.source == AcquisitionSource::DevPush
                    }) {
                        return Err(AppRequestErrorCode::NotFound);
                    }
                    Ok(Some(requests::projection(value)))
                }
                Err(InstallOperationError::NotFound) => Ok(None),
                Err(error) => Err(requests::code(error)),
            }
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
    }

    pub(crate) async fn begin_built_dev_push(
        &self,
        owner: String,
        session: String,
        id: String,
        upload: String,
        digest: String,
        app_id: String,
    ) -> Result<(), AppRequestErrorCode> {
        let permit = self
            .0
            .shared
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppRequestErrorCode::Busy)?;
        let store = self.0.shared.store.clone();
        let key = (owner.clone(), id.clone());
        let stopped = self.0.stopped.clone();
        let budget = AppOperationBudget::from_supervisor(move || stopped.load(Ordering::Acquire));
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut after = None;
            let mut target = None;
            loop {
                let page = store
                    .list_app_installations(&owner, after.as_deref(), 100)
                    .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
                for installation in page.installations {
                    if installation.app_id == app_id
                        && installation.channel == InstallationChannel::Dev
                        && installation.dev_room_id.as_deref() == Some(session.as_str())
                        && installation.data_release().is_some()
                    {
                        if target.is_some() {
                            return Err(AppRequestErrorCode::Conflict);
                        }
                        target = Some(UpdateTarget {
                            installation_id: installation.installation_id,
                            expected_generation: installation.generation,
                        });
                    }
                }
                after = page.next_cursor;
                if after.is_none() {
                    break;
                }
            }
            let input = InstallInput {
                source: AcquisitionSource::DevPush,
                expected_upload_digest: Some(digest.clone()),
                session_id: session,
                upload_handle: upload,
                update: target,
                deployment: None,
            };
            store
                .reserve_app_install(&owner, &id, input, &digest, budget)
                .map_err(requests::code)?;
            Ok(())
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)??;
        self.notify(key);
        Ok(())
    }
}
