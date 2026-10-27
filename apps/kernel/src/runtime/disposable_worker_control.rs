//! Cloud allocation control through the normal, owner-authorized home kernel.
use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use crate::local::{DisposableWorkerAllocation, LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::cloud_api_client::{
    cloud_url_component, delete_cloud_json_authenticated, get_cloud_json_authenticated,
    post_cloud_json_authenticated,
};
use crate::runtime::managed_environment_control::{
    authorized_cloud_profile,
    cloud_contract::{ContextTransferTicket, EnvironmentDetailsResponse},
    preflight_provider_account_exports,
};

pub(crate) async fn execute_disposable_worker_control_request(
    config: DaemonConfig,
    profiles: crate::account_profile::ProviderAccountProfileRegistry,
    caller_user_id: &str,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let cloud = authorized_cloud_profile(&config, caller_user_id)?;
    let token = cloud
        .cloud_session_token
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| error("Cloud session is unavailable"))?;
    if cloud.account_id.trim().is_empty() {
        return Err(error("Cloud account is unavailable"));
    }
    if let LocalDaemonRequest::KeepManagedEnvironmentRunning(request) = &request {
        identifier(&request.environment_id)?;
        let result = post_cloud_json_authenticated::<EnvironmentDetailsResponse>(
            cloud.api_url.clone(),
            format!(
                "/managed-environments/{}/auto-stop/keep-running",
                cloud_url_component(&request.environment_id)
            ),
            token.to_string(),
            serde_json::json!({"accountId":cloud.account_id}),
        )
        .await?;
        let environment: crate::local::ManagedEnvironmentSummary = result.environment.into();
        if environment.environment_id != request.environment_id {
            return Err(error("Cloud returned another managed environment"));
        }
        return Ok(LocalDaemonResponse::ManagedEnvironmentKeptRunning { environment });
    }
    if let LocalDaemonRequest::CreateDisposableWorker(create) = &request {
        home(
            &config,
            cloud,
            &create.home_kernel_id,
            &create.home_relay_realm_id,
        )?;
        identifier(&create.client_request_id)?;
        identifier(&create.compute_class)?;
        identifier(&create.architecture)?;
        if create.region.is_empty()
            || create.region.len() > 128
            || !create
                .region
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            || !(60..=86400).contains(&create.maximum_lifetime_seconds)
            || create.auto_stop_policy.minimum_runtime_seconds > 2_592_000
            || create
                .auto_stop_policy
                .idle_delay_seconds
                .is_some_and(|v| v > 2_592_000)
        {
            return Err(error("Invalid disposable worker placement or lifetime"));
        }
        if let Some(plan) = &create.context_plan {
            preflight_provider_account_exports(&config, cloud, &profiles, &plan.provider_accounts)?;
        }
        let mut body =
            serde_json::to_value(create).map_err(|_| error("Invalid disposable worker request"))?;
        body.as_object_mut()
            .ok_or_else(|| error("Invalid disposable worker request"))?
            .insert("accountId".into(), cloud.account_id.clone().into());
        // The client request ID is preserved; Cloud owns create idempotency. Never retry here.
        let allocation = post_cloud_json_authenticated::<DisposableWorkerAllocation>(
            cloud.api_url.clone(),
            "/disposable-workers".into(),
            token.to_string(),
            body,
        )
        .await?;
        bound(&config, cloud, &allocation, None)?;
        return Ok(LocalDaemonResponse::DisposableWorker { allocation });
    }
    let selection = match &request {
        LocalDaemonRequest::GetDisposableWorker(r)
        | LocalDaemonRequest::ReleaseDisposableWorker(r)
        | LocalDaemonRequest::KeepDisposableWorkerRunning(r)
        | LocalDaemonRequest::PrepareDisposableWorkerContextTransfer(r) => r,
        _ => return Err(error("Unsupported disposable worker control")),
    };
    home(
        &config,
        cloud,
        &selection.home_kernel_id,
        &selection.home_relay_realm_id,
    )?;
    identifier(&selection.allocation_id)?;
    let base = format!(
        "/disposable-workers/{}",
        cloud_url_component(&selection.allocation_id)
    );
    let query = format!("?accountId={}", cloud_url_component(&cloud.account_id));
    // Read authority before a mutation: an account may contain allocations for other homes.
    let allocation = get_cloud_json_authenticated::<DisposableWorkerAllocation>(
        cloud.api_url.clone(),
        format!("{base}{query}"),
        token.to_string(),
    )
    .await?;
    bound(&config, cloud, &allocation, Some(&selection.allocation_id))?;
    let allocation = match &request {
        LocalDaemonRequest::GetDisposableWorker(_) => allocation,
        LocalDaemonRequest::ReleaseDisposableWorker(_) => {
            delete_cloud_json_authenticated::<DisposableWorkerAllocation>(
                cloud.api_url.clone(),
                format!("{base}{query}"),
                token.to_string(),
            )
            .await?
        }
        LocalDaemonRequest::KeepDisposableWorkerRunning(_) => {
            post_cloud_json_authenticated::<DisposableWorkerAllocation>(
                cloud.api_url.clone(),
                format!("{base}/keep-running{query}"),
                token.to_string(),
                serde_json::json!({}),
            )
            .await?
        }
        LocalDaemonRequest::PrepareDisposableWorkerContextTransfer(_) => {
            let ticket = get_cloud_json_authenticated::<ContextTransferTicket>(
                cloud.api_url.clone(),
                format!("{base}/context-transfer{query}"),
                token.to_string(),
            )
            .await?
            .into_ticket()?;
            if ticket.environment_id != selection.allocation_id
                || allocation.runtime_kernel_id.as_deref() != Some(ticket.target.kernel_id.as_str())
                || allocation.runtime_machine_id.as_deref()
                    != Some(ticket.target.machine_id.as_str())
                || ticket.target.relay_realm_id != cloud.realm_id
            {
                return Err(error(
                    "Cloud returned a context ticket for another disposable worker",
                ));
            }
            crate::managed_context::outbound_service::validate_ticket(&config, &ticket)?;
            return Ok(LocalDaemonResponse::DisposableWorkerContextTransferPrepared { ticket });
        }
        _ => unreachable!(),
    };
    bound(&config, cloud, &allocation, Some(&selection.allocation_id))?;
    Ok(LocalDaemonResponse::DisposableWorker { allocation })
}

fn identifier(value: &str) -> Result<(), DaemonError> {
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_lowercase() && !value.as_bytes()[0].is_ascii_digit()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:-".contains(&b))
    {
        return Err(error("Invalid Cloud control identifier"));
    }
    Ok(())
}

fn home(
    config: &DaemonConfig,
    cloud: &PersistedCloudRelayProfile,
    kernel: &str,
    realm: &str,
) -> Result<(), DaemonError> {
    identifier(kernel)?;
    identifier(realm)?;
    if kernel != config.daemon_id || realm != cloud.realm_id {
        return Err(error(
            "Disposable worker belongs to another home kernel or realm",
        ));
    }
    Ok(())
}

fn bound(
    config: &DaemonConfig,
    cloud: &PersistedCloudRelayProfile,
    allocation: &DisposableWorkerAllocation,
    id: Option<&str>,
) -> Result<(), DaemonError> {
    identifier(&allocation.allocation_id)?;
    if id.is_some_and(|id| id != allocation.allocation_id) {
        return Err(error("Cloud returned another disposable worker allocation"));
    }
    home(
        config,
        cloud,
        allocation.home_kernel_id.as_deref().unwrap_or(""),
        allocation.home_relay_realm_id.as_deref().unwrap_or(""),
    )
}

fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "disposable worker control",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
