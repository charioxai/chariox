use crate::config::DaemonConfig;
use crate::error::DaemonError;

// MP-08 / MP-10 / MP-11: hosted metadata reads use the client-scoped grant
// (owner-visible targets); peer registration keeps the kernel grant. A
// self-hosted relay keeps its own token.
pub(crate) async fn relay_metadata_discovery_config(
    config: &DaemonConfig,
) -> Result<DaemonConfig, DaemonError> {
    let hosted = config
        .relay_url
        .as_deref()
        .is_some_and(|url| config.relay_url_uses_cloud_profile(url));
    scoped_metadata_config(config, hosted).await
}

// The returned config carries the issued grant and no Cloud profile, so relay
// discovery reads it as-is rather than issuing a second grant.
pub(crate) async fn scoped_metadata_config(
    config: &DaemonConfig,
    issue_cloud_grant: bool,
) -> Result<DaemonConfig, DaemonError> {
    let mut discovery = config.clone();
    if let Some(profile) = config.cloud_relay.as_ref().filter(|_| issue_cloud_grant) {
        let targets =
            crate::transport::relay_discovery::authorization::visible_targets(profile).await?;
        // MP-08 / MP-10 / MP-11: defer large inventories to the query's bounded
        // grants. A single API grant can authorize at most 64 targets.
        if targets.as_ref().is_some_and(|targets| targets.len() > 64) {
            return Ok(discovery);
        }
        discovery.relay_token = Some(
            crate::transport::relay_discovery::authorization::cloud_metadata_token_for_targets(
                config, targets,
            )
            .await?,
        );
    }
    discovery.cloud_relay = None;
    Ok(discovery)
}

// A managed slice worker is not in the owner's Cloud kernel directory, so a
// lookup that only carries the directory grant never finds it. The owner asks
// Cloud for a grant scoped to exactly that worker, as slice launch already does.
pub(crate) async fn relay_target_discovery_config(
    config: &DaemonConfig,
    target_ref: &str,
) -> Result<DaemonConfig, DaemonError> {
    let hosted_slice_profile = config
        .relay_url
        .as_deref()
        .filter(|url| config.relay_url_uses_cloud_profile(url))
        .and(config.cloud_relay.as_ref())
        .filter(|_| target_ref.starts_with("slice:"));
    let Some(profile) = hosted_slice_profile else {
        return relay_metadata_discovery_config(config).await;
    };
    let grant =
        super::issue_cloud_slice_discovery_token(profile, &config.daemon_id, target_ref).await?;
    Ok(crate::transport::relay_discovery::with_metadata_token(
        config,
        grant.token,
    ))
}
