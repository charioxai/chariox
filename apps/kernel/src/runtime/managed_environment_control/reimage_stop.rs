//! Cloud owns validation, STOP reservation and retry identity. The kernel only
//! waits for that exact normal lifecycle operation before requesting reimage.
use super::*;
use crate::local::{
    ManagedEnvironmentDesiredState, ManagedEnvironmentObservedState,
    ManagedEnvironmentOperationStatus, ManagedEnvironmentOperationSummary,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StopEnvironment {
    environment_id: String,
    account_id: String,
    runtime_generation: u64,
    desired_state: ManagedEnvironmentDesiredState,
    observed_state: ManagedEnvironmentObservedState,
    desired_revision: u64,
    observed_revision: u64,
}

#[derive(Deserialize)]
struct Admission {
    environment: StopEnvironment,
    operation: super::cloud_contract::OperationSummary,
}

#[derive(Debug, PartialEq, Eq)]
struct StopBinding {
    operation_id: String,
    idempotency_key: String,
    request_digest: String,
    desired_revision: u64,
}

pub(super) async fn prepare(
    cloud: &PersistedCloudRelayProfile,
    token: &str,
    caller_user_id: &str,
    request: &RequestManagedEnvironmentReimageRequest,
    body: &serde_json::Value,
) -> Result<(), DaemonError> {
    tokio::time::timeout(Duration::from_secs(25), async {
        let path = format!(
            "/managed-environments/{}/reimage/stop",
            cloud_url_component(&request.environment_id)
        );
        let mut binding = None;
        loop {
            // No direct lifecycle fallback on 404 or any admission failure.
            let admission: Admission = post_cloud_json_authenticated(
                cloud.api_url.clone(),
                path.clone(),
                token.to_string(),
                body.clone(),
            )
            .await?;
            if validate(
                admission,
                cloud.account_id.trim(),
                caller_user_id,
                request,
                &mut binding,
            )? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(1_500)).await;
        }
    })
    .await
    .map_err(|_| {
        control_error(
            "Managed reimage STOP is still settling; rerun confirm to resume the same operation",
        )
    })?
}

fn validate(
    admission: Admission,
    account_id: &str,
    caller_user_id: &str,
    request: &RequestManagedEnvironmentReimageRequest,
    binding: &mut Option<StopBinding>,
) -> Result<bool, DaemonError> {
    let environment = admission.environment;
    let operation: ManagedEnvironmentOperationSummary = admission.operation.into();
    if environment.environment_id != request.environment_id
        || environment.account_id != account_id
        || operation.environment_id != request.environment_id
        || operation.requested_by_user_id != caller_user_id
        || operation.operation_id.trim().is_empty()
        || operation.idempotency_key.trim().is_empty()
        || !is_sha256_digest(&operation.request_digest)
    {
        return Err(control_error(
            "Cloud returned an unrelated reimage STOP admission",
        ));
    }
    if operation.kind == ManagedEnvironmentOperationKind::Reimage {
        if binding
            .as_ref()
            .is_some_and(|stop| stop.request_digest != operation.request_digest)
            || operation.idempotency_key != request.idempotency_key
            || Some(environment.runtime_generation) != request.expected_generation.checked_add(1)
        {
            return Err(control_error(
                "Cloud returned an unrelated reimage replay admission",
            ));
        }
        // Original reimage POST verifies its existing request digest and receipt.
        return Ok(true);
    }
    if operation.kind != ManagedEnvironmentOperationKind::Stop
        || operation.idempotency_key
            != format!(
                "reimage-stop:{:x}",
                Sha256::digest(request.idempotency_key.as_bytes())
            )
        || environment.runtime_generation != request.expected_generation
        || environment.desired_state != ManagedEnvironmentDesiredState::Stopped
        || environment.desired_revision != operation.desired_revision
    {
        return Err(control_error(
            "Cloud returned a stale or unrelated reimage STOP operation",
        ));
    }
    let received = StopBinding {
        operation_id: operation.operation_id,
        idempotency_key: operation.idempotency_key,
        request_digest: operation.request_digest,
        desired_revision: operation.desired_revision,
    };
    if binding
        .as_ref()
        .is_some_and(|expected| expected != &received)
    {
        return Err(control_error(
            "Cloud changed the admitted reimage STOP operation",
        ));
    }
    *binding = Some(received);
    if operation.status == ManagedEnvironmentOperationStatus::Failed {
        return Err(control_error(
            "Managed reimage STOP failed; inspect the retained lifecycle operation",
        ));
    }
    if operation.status != ManagedEnvironmentOperationStatus::Succeeded {
        return Ok(false);
    }
    if operation
        .completed_at
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
        || environment.observed_state != ManagedEnvironmentObservedState::Stopped
        || environment.observed_revision != environment.desired_revision
    {
        return Err(control_error(
            "Cloud returned an unconverged completed reimage STOP",
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> RequestManagedEnvironmentReimageRequest {
        serde_json::from_value(serde_json::json!({
            "environmentId":"env-1","expectedGeneration":3,"expectedProviderServerId":"123",
            "expectedProviderImageId":"456","expectedProviderProfileId":"path1",
            "expectedProviderProfileDigest":format!("sha256:{}","a".repeat(64)),
            "expectedRuntimeReleaseDigest":format!("sha256:{}","b".repeat(64)),
            "expectedRuntimeSourceCommit":"c".repeat(40),"expectedRuntimeSourceTree":"d".repeat(40),
            "contextPlan":{"sourceTargetId":null,"kernelContext":"empty","developmentSetup":{"kind":"empty"},"providerAccounts":{"kind":"none"},"gitCredentials":{"kind":"none"}},
            "idempotencyKey":"reimage-1"
        })).unwrap()
    }
    fn admission() -> serde_json::Value {
        serde_json::json!({"environment":{"environmentId":"env-1","accountId":"account-1","runtimeGeneration":3,
            "desiredState":"stopped","observedState":"stopped","desiredRevision":8,"observedRevision":8},
            "operation":{"operationId":"stop-1","environmentId":"env-1","requestedByUserId":"owner-1","kind":"stop",
            "idempotencyKey":format!("reimage-stop:{:x}",Sha256::digest(b"reimage-1")),"requestDigest":format!("sha256:{}","a".repeat(64)),
            "desiredRevision":8,"status":"succeeded","attempt":0,"retryable":false,"failureCode":null,"failureMessage":null,
            "completedAt":"2026-09-28T00:00:00Z","createdAt":"2026-09-28T00:00:00Z","updatedAt":"2026-09-28T00:00:00Z"}})
    }
    fn check(
        value: serde_json::Value,
        binding: &mut Option<StopBinding>,
    ) -> Result<bool, DaemonError> {
        validate(
            serde_json::from_value(value).unwrap(),
            "account-1",
            "owner-1",
            &request(),
            binding,
        )
    }
    #[test]
    fn reimage_stop_requires_exact_completed_same_generation_receipt() {
        let mut binding = None;
        let mut pending = admission();
        pending["operation"]["status"] = serde_json::json!("running");
        pending["environment"]["observedState"] = serde_json::json!("stopping");
        assert!(!check(pending, &mut binding).unwrap());
        assert!(check(admission(), &mut binding).unwrap());
        for (section, field, value) in [
            ("environment", "runtimeGeneration", serde_json::json!(4)),
            ("environment", "observedRevision", serde_json::json!(7)),
            ("environment", "accountId", serde_json::json!("other")),
            ("operation", "operationId", serde_json::json!("replacement")),
            ("operation", "idempotencyKey", serde_json::json!("wrong")),
            ("operation", "requestedByUserId", serde_json::json!("other")),
            (
                "operation",
                "requestDigest",
                serde_json::json!(format!("sha256:{}", "b".repeat(64))),
            ),
            ("operation", "status", serde_json::json!("failed")),
            ("operation", "completedAt", serde_json::Value::Null),
        ] {
            let mut changed = admission();
            changed[section][field] = value;
            assert!(check(changed, &mut binding).is_err(), "{section}.{field}");
        }
    }
    #[test]
    fn reimage_replay_skips_stop_only_for_original_key_and_next_generation() {
        let mut replay = admission();
        replay["operation"]["kind"] = serde_json::json!("reimage");
        replay["operation"]["idempotencyKey"] = serde_json::json!("reimage-1");
        replay["environment"]["runtimeGeneration"] = serde_json::json!(4);
        assert!(check(replay.clone(), &mut None).unwrap());
        replay["operation"]["idempotencyKey"] = serde_json::json!("foreign");
        assert!(check(replay, &mut None).is_err());
    }
    #[test]
    fn reimage_stop_accepts_concurrent_same_request_replay_but_not_changed_digest() {
        let mut binding = None;
        assert!(check(admission(), &mut binding).unwrap());
        let mut replay = admission();
        replay["operation"]["operationId"] = serde_json::json!("reimage-operation-1");
        replay["operation"]["kind"] = serde_json::json!("reimage");
        replay["operation"]["idempotencyKey"] = serde_json::json!("reimage-1");
        replay["environment"]["runtimeGeneration"] = serde_json::json!(4);
        assert!(check(replay.clone(), &mut binding).unwrap());
        replay["operation"]["requestDigest"] =
            serde_json::json!(format!("sha256:{}", "b".repeat(64)));
        assert!(check(replay, &mut binding).is_err());
    }
    #[tokio::test]
    async fn reimage_stop_missing_cloud_endpoint_fails_without_lifecycle_fallback() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0_u8; 8192];
            let count = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..count])
                .starts_with("POST /managed-environments/env-1/reimage/stop "));
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .unwrap();
        });
        let cloud = PersistedCloudRelayProfile {
            kernel_id: None,
            kernel_credential: None,
            kernel_public_key_thumbprint: None,
            api_url: format!("http://{address}"),
            account_id: "account-1".into(),
            ..Default::default()
        };
        assert!(prepare(
            &cloud,
            "fixture-token",
            "owner-1",
            &request(),
            &serde_json::json!({})
        )
        .await
        .is_err());
        worker.join().unwrap();
    }
}
