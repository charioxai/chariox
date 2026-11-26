use super::*;

impl KernelRuntimeState {
    pub(crate) async fn mark_cloud_kernel_removal(
        &self,
        profile: crate::config::PersistedCloudRelayProfile,
        removal: crate::config::CloudKernelRemoval,
    ) -> Result<(), DaemonError> {
        self.with_app_side_effect(move |app| app.mark_cloud_kernel_removal(&profile, removal))
            .await
    }

    pub(crate) async fn finish_cloud_relay_renewal(
        &self,
        expected: crate::config::PersistedCloudRelayProfile,
        renewal: Option<(crate::config::PersistedCloudRelayProfile, String)>,
    ) -> Result<(), DaemonError> {
        self.with_authorized_app_side_effect(move |app| {
            // A renewal installed in the projection may have failed its own save, so the app's
            // profile can carry an older expiry than the `expected` one: same enrollment is enough.
            let current = app.config().cloud_relay.clone();
            let expected_matches = match (&renewal, &current) {
                (Some(_), Some(current)) => {
                    crate::runtime::projection::same_cloud_enrollment(current, &expected)
                }
                _ => current.as_ref() == Some(&expected),
            };
            if expected_matches
                && crate::runtime::cloud_relay_control::cloud_relay_transport_selected(app.config())
            {
                match renewal {
                    // Saves run concurrently: an older renewal never replaces a newer one.
                    Some((updated, _))
                        if current.as_ref().is_some_and(|current| {
                            current.token_expires_at_ms > updated.token_expires_at_ms
                        }) => {}
                    Some((updated, token)) => {
                        app.persist_cloud_relay_token(updated, token)?;
                    }
                    None => app.persist_cloud_relay_profile(None)?,
                }
            }
            Ok(())
        })
        .await
    }

    pub(crate) async fn connect_cloud_relay(
        &self,
        expected: crate::config::PersistedCloudRelayProfile,
        profile: crate::config::PersistedCloudRelayProfile,
        token: String,
    ) -> Result<(), DaemonError> {
        self.with_authorized_app_side_effect(move |app| {
            if app.config().cloud_relay.as_ref() != Some(&expected) {
                if let Some(removal) = app
                    .config()
                    .cloud_relay
                    .as_ref()
                    .and_then(|profile| profile.kernel_removal())
                {
                    return Err(
                        crate::runtime::cloud_relay_control::cloud_kernel_removal_error(
                            "connect Cloud relay",
                            removal,
                        ),
                    );
                }
                return Err(DaemonError::LocalTransport {
                    operation: "connect Cloud relay",
                    message: "Cloud enrollment changed during connection. Try connecting again."
                        .into(),
                });
            }
            app.persist_cloud_relay_token(profile, token)?;
            app.invalidate_provider_catalog_cache();
            Ok(())
        })
        .await
    }

    pub(crate) async fn set_user_config_value(
        &self,
        path: String,
        value: String,
    ) -> Result<crate::config::DaemonConfig, DaemonError> {
        let config = self
            .with_authorized_app_side_effect(move |app| {
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
            .with_authorized_app_side_effect(move |app| {
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
            .with_authorized_app_side_effect(move |app| {
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
            .with_authorized_app_side_effect(move |app| {
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
            .with_authorized_app_side_effect(move |app| {
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
            .with_authorized_app_side_effect(move |app| {
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

impl KernelRuntimeOwnedState {
    pub(super) fn require_linked_workflow_owner(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        if let Some(profile) = config.cloud_relay {
            if let Some(removal) = profile.kernel_removal().filter(|_| {
                self.session_store
                    .read()
                    .session_owner_user_id(session_id)
                    .as_deref()
                    == Some(profile.user_id.as_str())
            }) {
                return Err(
                    crate::runtime::cloud_relay_control::cloud_kernel_removal_error(
                        "start owner workflow",
                        removal,
                    ),
                );
            }
        }
        Ok(())
    }
}
