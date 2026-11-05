use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::cloud_api_client::{
    post_cloud_json_with_timeout, CLOUD_PRESENCE_REQUEST_TIMEOUT,
};
use crate::runtime::cloud_relay_connection_executor::{
    ensure_cloud_relay_connection as ensure_cloud_relay_connection_with_executor,
    ensure_cloud_relay_connection_replacing as ensure_cloud_relay_connection_replacing_with_executor,
};
use crate::runtime::cloud_relay_control::{
    cloud_kernel_presence_body, cloud_relay_token_refresh_due,
};

impl CommandRouter {
    pub(crate) fn relay_config_snapshot(&self) -> crate::config::DaemonConfig {
        self.config_projection.snapshot()
    }

    pub(crate) fn cloud_relay_token_refresh_due(&self) -> bool {
        let config = self.config_projection.snapshot();
        cloud_relay_token_refresh_due(&config, crate::session::unix_epoch_ms())
    }

    pub(crate) async fn ensure_cloud_relay_connection(&self) -> Result<(), DaemonError> {
        ensure_cloud_relay_connection_with_executor(&self.runtime_state, &self.config_projection)
            .await
    }

    pub(crate) async fn ensure_cloud_relay_connection_replacing(
        &self,
        refused_token: Option<&str>,
    ) -> Result<(), DaemonError> {
        ensure_cloud_relay_connection_replacing_with_executor(
            &self.runtime_state,
            &self.config_projection,
            refused_token,
        )
        .await
    }

    pub(crate) async fn publish_cloud_kernel_presence(
        &self,
        online: bool,
    ) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let Some(profile) = config.cloud_relay.as_ref() else {
            return Ok(());
        };
        let registration = self.relay_registration().await;
        let Some(body) = cloud_kernel_presence_body(&config, profile, online, Some(&registration))
        else {
            return Ok(());
        };
        let _: serde_json::Value = post_cloud_json_with_timeout(
            profile.api_url.clone(),
            "/kernels/presence",
            body,
            CLOUD_PRESENCE_REQUEST_TIMEOUT,
        )
        .await?;
        Ok(())
    }
}
