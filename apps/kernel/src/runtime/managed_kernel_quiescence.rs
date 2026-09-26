use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::runtime::cloud_api_client::post_cloud_json;
use crate::runtime::state::{
    KernelRuntimeState, ManagedKernelQuiescenceChallenge, ManagedKernelQuiescenceOutcome,
};

const POLL_ENDPOINT: &str = "/v1/managed-kernels/auto-stop/quiescence/poll";
const ACK_ENDPOINT: &str = "/v1/managed-kernels/auto-stop/quiescence/ack";
const RELEASE_ACK_ENDPOINT: &str = "/v1/managed-kernels/auto-stop/quiescence/release-ack";
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(crate) struct ManagedKernelQuiescenceClient {
    binding: QuiescenceBinding,
}

#[derive(Clone)]
struct QuiescenceBinding {
    api_url: String,
    account_id: String,
    environment_id: String,
    machine_id: String,
    kernel_id: String,
    machine_credential: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PollResponse {
    protocol_version: u8,
    command: Option<QuiescenceCommand>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QuiescenceCommand {
    ReserveIdleForStop {
        #[serde(flatten)]
        challenge: ManagedKernelQuiescenceChallenge,
    },
    ReleaseAdmissionFence {
        #[serde(flatten)]
        challenge: ManagedKernelQuiescenceChallenge,
        outcome: ManagedKernelQuiescenceOutcome,
        result_sequence: u64,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PollRequest<'a> {
    protocol_version: u8,
    action: &'static str,
    account_id: &'a str,
    environment_id: &'a str,
    machine_id: &'a str,
    kernel_id: &'a str,
    machine_credential: &'a str,
    signature: String,
}

impl ManagedKernelQuiescenceClient {
    pub(crate) fn from_runtime(
        config: &DaemonConfig,
        registration: Option<&ConfirmedManagedKernelRegistration>,
    ) -> Result<Option<Self>, DaemonError> {
        let Some(registration) = registration else {
            return Ok(None);
        };
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| quiescence_error("confirmed managed kernel has no Cloud relay profile"))?;
        let binding = QuiescenceBinding::from_runtime(config, registration, profile)?;
        Ok(Some(Self { binding }))
    }

    pub(crate) async fn run(
        self,
        runtime: KernelRuntimeState,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), DaemonError> {
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            if self.poll_once(&runtime).await.is_err() {
                // Do not include request material, challenge values, nonce or credential.
                crate::logging::warn_with_fields(
                    "managed_kernel.auto_stop_quiescence",
                    "quiescence poll did not complete; durable admission state is retained",
                    json!({}),
                );
            }
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }
        }
    }

    async fn poll_once(&self, runtime: &KernelRuntimeState) -> Result<(), DaemonError> {
        let payload = self.signed_poll_request()?;
        let response: PollResponse = post_cloud_json(self.binding.api_url.clone(), POLL_ENDPOINT, payload).await?;
        if response.protocol_version != 1 {
            return Err(quiescence_error("Cloud returned an unsupported quiescence protocol version"));
        }
        match response.command {
            None => Ok(()),
            Some(QuiescenceCommand::ReserveIdleForStop { challenge }) => {
                self.validate_challenge(&challenge)?;
                let decision = match runtime.reserve_managed_kernel_for_stop(challenge.clone()) {
                    Ok(true) => "fenced",
                    Ok(false) | Err(_) => "busy",
                };
                let payload = self.signed_reservation_ack(&challenge, decision)?;
                let _: Value = post_cloud_json(self.binding.api_url.clone(), ACK_ENDPOINT, payload).await?;
                Ok(())
            }
            Some(QuiescenceCommand::ReleaseAdmissionFence {
                challenge,
                outcome,
                result_sequence,
            }) => {
                self.validate_challenge(&challenge)?;
                validate_result_sequence(result_sequence)?;
                runtime.apply_managed_kernel_stop_release(&challenge, outcome, result_sequence)?;
                let payload = self.signed_release_ack(&challenge, outcome, result_sequence)?;
                let _: Value = post_cloud_json(
                    self.binding.api_url.clone(),
                    RELEASE_ACK_ENDPOINT,
                    payload,
                )
                .await?;
                Ok(())
            }
        }
    }

    fn validate_challenge(
        &self,
        challenge: &ManagedKernelQuiescenceChallenge,
    ) -> Result<(), DaemonError> {
        if challenge.account_id != self.binding.account_id
            || challenge.environment_id != self.binding.environment_id
            || challenge.machine_id != self.binding.machine_id
            || challenge.kernel_id != self.binding.kernel_id
            || !is_canonical_nonempty(&challenge.challenge_id)
            || !is_canonical_nonempty(&challenge.stop_operation_id)
            || challenge.desired_revision == 0
            || challenge.idle_sequence == 0
            || !is_utc_millisecond_timestamp(&challenge.idle_deadline_at)
        {
            return Err(quiescence_error("Cloud quiescence command identity is invalid"));
        }
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(challenge.nonce.as_bytes())
            .map_err(|_| quiescence_error("Cloud quiescence nonce is invalid"))?;
        if nonce.len() != 32
            || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&nonce) != challenge.nonce
        {
            return Err(quiescence_error("Cloud quiescence nonce has an invalid length"));
        }
        Ok(())
    }

    fn signed_poll_request(&self) -> Result<Value, DaemonError> {
        let values = BTreeMap::from([
            ("protocolVersion", json!(1)),
            ("action", json!("auto_stop_quiescence_poll")),
            ("accountId", json!(self.binding.account_id)),
            ("environmentId", json!(self.binding.environment_id)),
            ("machineId", json!(self.binding.machine_id)),
            ("kernelId", json!(self.binding.kernel_id)),
        ]);
        let signature = hmac_signature(&self.binding.machine_credential, &values)?;
        let request = PollRequest {
            protocol_version: 1,
            action: "auto_stop_quiescence_poll",
            account_id: &self.binding.account_id,
            environment_id: &self.binding.environment_id,
            machine_id: &self.binding.machine_id,
            kernel_id: &self.binding.kernel_id,
            machine_credential: &self.binding.machine_credential,
            signature,
        };
        serde_json::to_value(request).map_err(|error| quiescence_error(error.to_string()))
    }

    fn signed_reservation_ack(
        &self,
        challenge: &ManagedKernelQuiescenceChallenge,
        decision: &str,
    ) -> Result<Value, DaemonError> {
        let mut values = challenge_values(challenge);
        values.insert("protocolVersion", json!(1));
        values.insert("action", json!("auto_stop_quiescence_ack"));
        values.insert("decision", json!(decision));
        self.signed_body(values)
    }

    fn signed_release_ack(
        &self,
        challenge: &ManagedKernelQuiescenceChallenge,
        outcome: ManagedKernelQuiescenceOutcome,
        result_sequence: u64,
    ) -> Result<Value, DaemonError> {
        let mut values = identity_values(&self.binding);
        values.insert("protocolVersion", json!(1));
        values.insert("action", json!("auto_stop_quiescence_release_ack"));
        values.insert("challengeId", json!(challenge.challenge_id));
        values.insert("stopOperationId", json!(challenge.stop_operation_id));
        values.insert("nonce", json!(challenge.nonce));
        values.insert(
            "outcome",
            json!(match outcome {
                ManagedKernelQuiescenceOutcome::Stopped => "stopped",
                ManagedKernelQuiescenceOutcome::KeepRunning => "keep_running",
            }),
        );
        values.insert("resultSequence", json!(result_sequence));
        self.signed_body(values)
    }

    fn signed_body(&self, mut values: BTreeMap<&'static str, Value>) -> Result<Value, DaemonError> {
        let signature = hmac_signature(&self.binding.machine_credential, &values)?;
        values.insert("machineCredential", json!(self.binding.machine_credential));
        values.insert("signature", json!(signature));
        serde_json::to_value(values).map_err(|error| quiescence_error(error.to_string()))
    }
}

impl QuiescenceBinding {
    fn from_runtime(
        config: &DaemonConfig,
        registration: &ConfirmedManagedKernelRegistration,
        profile: &PersistedCloudRelayProfile,
    ) -> Result<Self, DaemonError> {
        let machine_id = profile.machine_id.as_deref().ok_or_else(|| {
            quiescence_error("confirmed managed kernel has no Cloud Machine identity")
        })?;
        let machine_credential = profile.machine_credential.as_deref().ok_or_else(|| {
            quiescence_error("confirmed managed kernel has no Cloud Machine credential")
        })?;
        if profile.api_url.trim().is_empty()
            || profile.account_id.trim().is_empty()
            || machine_credential.trim().is_empty()
            || registration.environment_id.trim().is_empty()
            || registration.machine_id != machine_id
            || registration.machine_id != config.host_machine_id
            || registration.kernel_id != config.daemon_id
        {
            return Err(quiescence_error("managed quiescence identity does not match confirmed kernel registration"));
        }
        Ok(Self {
            api_url: profile.api_url.trim_end_matches('/').to_string(),
            account_id: profile.account_id.clone(),
            environment_id: registration.environment_id.clone(),
            machine_id: machine_id.to_string(),
            kernel_id: registration.kernel_id.clone(),
            machine_credential: machine_credential.to_string(),
        })
    }
}

fn challenge_values(challenge: &ManagedKernelQuiescenceChallenge) -> BTreeMap<&'static str, Value> {
    let mut values = identity_values(&QuiescenceBinding {
        api_url: String::new(),
        account_id: challenge.account_id.clone(),
        environment_id: challenge.environment_id.clone(),
        machine_id: challenge.machine_id.clone(),
        kernel_id: challenge.kernel_id.clone(),
        machine_credential: String::new(),
    });
    values.insert("challengeId", json!(challenge.challenge_id));
    values.insert("desiredRevision", json!(challenge.desired_revision));
    values.insert("idleSequence", json!(challenge.idle_sequence));
    values.insert("idleDeadlineAt", json!(challenge.idle_deadline_at));
    values.insert("stopOperationId", json!(challenge.stop_operation_id));
    values.insert("nonce", json!(challenge.nonce));
    values
}

fn identity_values(binding: &QuiescenceBinding) -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("accountId", json!(binding.account_id)),
        ("environmentId", json!(binding.environment_id)),
        ("machineId", json!(binding.machine_id)),
        ("kernelId", json!(binding.kernel_id)),
    ])
}

fn hmac_signature(
    credential: &str,
    values: &BTreeMap<&'static str, Value>,
) -> Result<String, DaemonError> {
    let canonical = serde_json::to_vec(values)
        .map_err(|error| quiescence_error(format!("could not canonicalize quiescence request: {error}")))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(credential.as_bytes())
        .map_err(|_| quiescence_error("could not initialize quiescence request signature"))?;
    mac.update(&canonical);
    Ok(format!("sha256:{}", hex::encode(mac.finalize().into_bytes())))
}

fn validate_result_sequence(sequence: u64) -> Result<(), DaemonError> {
    if sequence == 0 {
        Err(quiescence_error("Cloud release result sequence must be positive"))
    } else {
        Ok(())
    }
}

fn is_canonical_nonempty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

fn is_utc_millisecond_timestamp(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok_and(|timestamp| {
        timestamp.offset().local_minus_utc() == 0
            && timestamp.timestamp_subsec_nanos() % 1_000_000 == 0
            && timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true) == value
    })
}

fn quiescence_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "manage kernel auto-stop quiescence",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> QuiescenceBinding {
        QuiescenceBinding {
            api_url: "https://cloud.example".into(),
            account_id: "account-1".into(),
            environment_id: "environment-1".into(),
            machine_id: "machine-1".into(),
            kernel_id: "kernel-1".into(),
            machine_credential: "credential-never-logged".into(),
        }
    }

    fn challenge() -> ManagedKernelQuiescenceChallenge {
        ManagedKernelQuiescenceChallenge {
            challenge_id: "challenge-1".into(),
            account_id: "account-1".into(),
            environment_id: "environment-1".into(),
            machine_id: "machine-1".into(),
            kernel_id: "kernel-1".into(),
            desired_revision: 4,
            idle_sequence: 8,
            idle_deadline_at: "2026-09-26T00:00:00.000Z".into(),
            stop_operation_id: "stop-operation-1".into(),
            nonce: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
        }
    }

    #[test]
    fn signed_ack_bodies_use_exact_protocol_fields_and_hmac_prefix() {
        let client = ManagedKernelQuiescenceClient { binding: binding() };
        let challenge = challenge();
        let reserve = client
            .signed_reservation_ack(&challenge, "fenced")
            .expect("reservation ack should sign");
        assert_eq!(reserve["protocolVersion"], 1);
        assert_eq!(reserve["action"], "auto_stop_quiescence_ack");
        assert_eq!(reserve["decision"], "fenced");
        assert_eq!(reserve["idleSequence"], 8);
        assert_eq!(reserve["nonce"], challenge.nonce);
        assert!(reserve["signature"]
            .as_str()
            .is_some_and(|signature| signature.starts_with("sha256:") && signature.len() == 71));
        let reserve_fields = reserve.as_object().expect("object");
        assert_eq!(reserve_fields.len(), 15);

        let release = client
            .signed_release_ack(
                &challenge,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                2,
            )
            .expect("release ack should sign");
        assert_eq!(release["action"], "auto_stop_quiescence_release_ack");
        assert_eq!(release["outcome"], "keep_running");
        assert_eq!(release["resultSequence"], 2);
        assert!(release.get("desiredRevision").is_none());
        assert!(release.get("idleSequence").is_none());
        assert!(release.get("idleDeadlineAt").is_none());
    }

    #[test]
    fn cloud_poll_commands_decode_as_exact_challenge_tuples() {
        let challenge = challenge();
        let response: PollResponse = serde_json::from_value(json!({
            "protocolVersion": 1,
            "command": {
                "kind": "reserve_idle_for_stop",
                "challengeId": challenge.challenge_id.clone(),
                "accountId": challenge.account_id.clone(),
                "environmentId": challenge.environment_id.clone(),
                "machineId": challenge.machine_id.clone(),
                "kernelId": challenge.kernel_id.clone(),
                "desiredRevision": challenge.desired_revision,
                "idleSequence": challenge.idle_sequence,
                "idleDeadlineAt": challenge.idle_deadline_at.clone(),
                "stopOperationId": challenge.stop_operation_id.clone(),
                "nonce": challenge.nonce.clone(),
            }
        }))
        .expect("reserve poll response should decode");
        assert_eq!(response.protocol_version, 1);
        match response.command.expect("reserve command") {
            QuiescenceCommand::ReserveIdleForStop { challenge: decoded } => {
                assert_eq!(decoded, challenge);
            }
            QuiescenceCommand::ReleaseAdmissionFence { .. } => {
                panic!("reserve command decoded as release")
            }
        }
    }

    #[test]
    fn canonical_hmac_is_stable_for_same_sorted_contract_fields() {
        let first = BTreeMap::from([
            ("protocolVersion", json!(1)),
            ("action", json!("auto_stop_quiescence_poll")),
            ("accountId", json!("account-1")),
        ]);
        let second = BTreeMap::from([
            ("accountId", json!("account-1")),
            ("action", json!("auto_stop_quiescence_poll")),
            ("protocolVersion", json!(1)),
        ]);
        assert_eq!(
            hmac_signature("credential", &first).expect("first signature"),
            hmac_signature("credential", &second).expect("same canonical signature"),
        );
    }
}
