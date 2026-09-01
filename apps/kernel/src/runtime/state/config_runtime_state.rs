use super::*;

impl KernelRuntimeState {
    pub(crate) async fn set_user_config_value(
        &self,
        path: String,
        value: String,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_app_side_effect(move |app| {
                app.set_user_config_value(path, value)?;
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config.clone());
        Ok(config)
    }

    pub(crate) async fn unset_user_config_value(
        &self,
        path: String,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_app_side_effect(move |app| {
                app.unset_user_config_value(path)?;
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config.clone());
        Ok(config)
    }

    pub(crate) async fn configure_relay(
        &self,
        relay_url: Option<String>,
        relay_token: Option<String>,
        invalidate_provider_catalog: bool,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_app_side_effect(move |app| {
                app.configure_relay(relay_url, relay_token)?;
                if invalidate_provider_catalog {
                    app.invalidate_provider_catalog_cache();
                }
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config.clone());
        Ok(config)
    }

    pub(crate) async fn configure_managed_slice_relay(
        &self,
        relay_url: String,
        relay_token: String,
        recovery_token: String,
        owner_public_key: String,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_app_side_effect(move |app| {
                app.configure_managed_slice_relay(
                    relay_url,
                    relay_token,
                    recovery_token,
                    owner_public_key,
                )?;
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config.clone());
        Ok(config)
    }

    pub(crate) async fn persist_cloud_relay_profile(
        &self,
        profile: Option<crate::config::PersistedCloudRelayProfile>,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_app_side_effect(move |app| {
                app.persist_cloud_relay_profile(profile)?;
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config.clone());
        Ok(config)
    }

    pub(crate) async fn configure_relay_with_cloud_profile(
        &self,
        relay_url: Option<String>,
        relay_token: Option<String>,
        profile: Option<crate::config::PersistedCloudRelayProfile>,
        invalidate_provider_catalog: bool,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_app_side_effect(move |app| {
                app.configure_relay(relay_url, relay_token)?;
                app.persist_cloud_relay_profile(profile)?;
                if invalidate_provider_catalog {
                    app.invalidate_provider_catalog_cache();
                }
                Ok(app.config().clone())
            })
            .await?;
        self.owned.config_projection.update(config.clone());
        Ok(config)
    }
}
