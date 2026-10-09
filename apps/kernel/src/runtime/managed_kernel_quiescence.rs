use std::collections::BTreeMap;
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
const MAX_FAILURE_POLL_INTERVAL: Duration = Duration::from_secs(30);
const FAILURE_WARNING_SUMMARY_INTERVAL: u8 = 5;

#[derive(Clone)]
pub(crate) struct ManagedKernelQuiescenceClient {
    binding: QuiescenceBinding,
}

#[derive(Clone)]
pub(super) struct QuiescenceBinding {
    pub(super) api_url: String,
    pub(super) account_id: String,
    pub(super) environment_id: String,
    pub(super) machine_id: String,
    pub(super) kernel_id: String,
    pub(super) machine_credential: String,
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
        #[serde(rename = "resultSequence")]
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

#[derive(Default)]
struct QuiescencePollSchedule {
    consecutive_failures: u32,
}

impl QuiescencePollSchedule {
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    fn next_poll_delay(&self) -> Duration {
        if self.consecutive_failures == 0 {
            return POLL_INTERVAL;
        }

        let exponent = self.consecutive_failures.saturating_sub(1).min(4);
        let retry_secs = POLL_INTERVAL
            .as_secs()
            .saturating_mul(1_u64 << exponent)
            .min(MAX_FAILURE_POLL_INTERVAL.as_secs());
        Duration::from_secs(retry_secs)
    }
}

#[derive(Default)]
struct QuiescencePollWarningState {
    warned_this_streak: bool,
    failures_since_summary: u8,
}

impl QuiescencePollWarningState {
    fn should_warn(&mut self) -> bool {
        if !self.warned_this_streak {
            self.warned_this_streak = true;
            return true;
        }

        self.failures_since_summary = self.failures_since_summary.saturating_add(1);
        if self.failures_since_summary >= FAILURE_WARNING_SUMMARY_INTERVAL {
            self.failures_since_summary = 0;
            true
        } else {
            false
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PollWaitOutcome {
    PollNow,
    Shutdown,
}

async fn wait_for_poll_or_shutdown(
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
    delay: Duration,
) -> PollWaitOutcome {
    tokio::select! {
        biased;
        changed = shutdown.changed() => {
            if changed.is_err() || *shutdown.borrow() {
                PollWaitOutcome::Shutdown
            } else {
                PollWaitOutcome::PollNow
            }
        }
        _ = tokio::time::sleep(delay) => PollWaitOutcome::PollNow,
    }
}

#[cfg(test)]
#[path = "managed_kernel_quiescence_polling_tests.rs"]
mod polling_tests;

impl ManagedKernelQuiescenceClient {
    pub(crate) fn from_runtime(
        config: &DaemonConfig,
        registration: Option<&ConfirmedManagedKernelRegistration>,
    ) -> Result<Option<Self>, DaemonError> {
        let Some(registration) = registration else {
            return Ok(None);
        };
        let profile = config.cloud_relay.as_ref().ok_or_else(|| {
            quiescence_error("confirmed managed kernel has no Cloud relay profile")
        })?;
        let binding = QuiescenceBinding::from_runtime(config, registration, profile)?;
        Ok(Some(Self { binding }))
    }

    pub(crate) async fn run(
        self,
        runtime: KernelRuntimeState,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), DaemonError> {
        let mut schedule = QuiescencePollSchedule::default();
        let mut warning_state = QuiescencePollWarningState::default();
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }
            let delay = match self.poll_once(&runtime).await {
                Ok(()) => {
                    schedule.record_success();
                    warning_state.reset();
                    schedule.next_poll_delay()
                }
                Err(_) => {
                    schedule.record_failure();
                    let delay = schedule.next_poll_delay();
                    if warning_state.should_warn() {
                        // Keep the retry summary bounded and omit error/request material.
                        crate::logging::warn_with_fields(
                            "managed_kernel.auto_stop_quiescence",
                            "quiescence poll failed; retrying while durable admission state is retained",
                            json!({
                                "consecutive_failures": schedule.consecutive_failures,
                                "retry_in_seconds": delay.as_secs(),
                            }),
                        );
                    }
                    delay
                }
            };
            if wait_for_poll_or_shutdown(&mut shutdown, delay).await == PollWaitOutcome::Shutdown {
                return Ok(());
            }
        }
    }

    async fn poll_once(&self, runtime: &KernelRuntimeState) -> Result<(), DaemonError> {
        let payload = self.signed_poll_request()?;
        let response: PollResponse =
            post_cloud_json(self.binding.api_url.clone(), POLL_ENDPOINT, payload).await?;
        if response.protocol_version != 1 {
            return Err(quiescence_error(
                "Cloud returned an unsupported quiescence protocol version",
            ));
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
                let _: Value =
                    post_cloud_json(self.binding.api_url.clone(), ACK_ENDPOINT, payload).await?;
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
                let _: Value =
                    post_cloud_json(self.binding.api_url.clone(), RELEASE_ACK_ENDPOINT, payload)
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
            return Err(quiescence_error(
                "Cloud quiescence command identity is invalid",
            ));
        }
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(challenge.nonce.as_bytes())
            .map_err(|_| quiescence_error("Cloud quiescence nonce is invalid"))?;
        if nonce.len() != 32
            || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&nonce) != challenge.nonce
        {
            return Err(quiescence_error(
                "Cloud quiescence nonce has an invalid length",
            ));
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
    pub(super) fn from_runtime(
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
            return Err(quiescence_error(
                "managed quiescence identity does not match confirmed kernel registration",
            ));
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

pub(super) fn identity_values(binding: &QuiescenceBinding) -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("accountId", json!(binding.account_id)),
        ("environmentId", json!(binding.environment_id)),
        ("machineId", json!(binding.machine_id)),
        ("kernelId", json!(binding.kernel_id)),
    ])
}

pub(super) fn hmac_signature(
    credential: &str,
    values: &BTreeMap<&'static str, Value>,
) -> Result<String, DaemonError> {
    let canonical = serde_json::to_vec(values).map_err(|error| {
        quiescence_error(format!(
            "could not canonicalize quiescence request: {error}"
        ))
    })?;
    let mut mac = Hmac::<Sha256>::new_from_slice(credential.as_bytes())
        .map_err(|_| quiescence_error("could not initialize quiescence request signature"))?;
    mac.update(&canonical);
    let digest = mac.finalize().into_bytes();
    let signature = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{signature}"))
}

fn validate_result_sequence(sequence: u64) -> Result<(), DaemonError> {
    if sequence == 0 {
        Err(quiescence_error(
            "Cloud release result sequence must be positive",
        ))
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
    fn cloud_poll_and_reservation_requests_match_canonical_v1_payloads_and_hmacs() {
        // Mirrors Cloud's fixed-vector credential without using production secrets.
        let mut binding = binding();
        binding.machine_credential = format!("mcred_{}", "a".repeat(43));
        let client = ManagedKernelQuiescenceClient { binding };

        let poll = client
            .signed_poll_request()
            .expect("poll request should sign");
        assert_eq!(
            serde_json::to_string(&poll).expect("poll payload should serialize"),
            r#"{"accountId":"account-1","action":"auto_stop_quiescence_poll","environmentId":"environment-1","kernelId":"kernel-1","machineCredential":"mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","machineId":"machine-1","protocolVersion":1,"signature":"sha256:2d8160d958ced5fd810ab82baf032d91ff72fff9c1324a04d6acf4aac01b4005"}"#,
        );

        let reserve = client
            .signed_reservation_ack(&challenge(), "fenced")
            .expect("reservation ack should sign");
        assert_eq!(
            serde_json::to_string(&reserve).expect("reservation payload should serialize"),
            r#"{"accountId":"account-1","action":"auto_stop_quiescence_ack","challengeId":"challenge-1","decision":"fenced","desiredRevision":4,"environmentId":"environment-1","idleDeadlineAt":"2026-09-26T00:00:00.000Z","idleSequence":8,"kernelId":"kernel-1","machineCredential":"mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","protocolVersion":1,"signature":"sha256:d1200d83260be01c766316b0f661ce7607969fafd409e540ffa68dd2c5253bd2","stopOperationId":"stop-operation-1"}"#,
        );
    }

    #[test]
    fn cloud_poll_command_golden_response_decodes_as_exact_challenge_tuple() {
        let challenge = challenge();
        let response: PollResponse = serde_json::from_str(
            r#"{"protocolVersion":1,"command":{"kind":"reserve_idle_for_stop","challengeId":"challenge-1","accountId":"account-1","environmentId":"environment-1","machineId":"machine-1","kernelId":"kernel-1","desiredRevision":4,"idleSequence":8,"idleDeadlineAt":"2026-09-26T00:00:00.000Z","stopOperationId":"stop-operation-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}"#,
        )
        .expect("reserve poll response should deserialize");
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
    fn cloud_poll_golden_response_decodes_empty_and_release_command_variants() {
        let empty: PollResponse = serde_json::from_str(r#"{"protocolVersion":1,"command":null}"#)
            .expect("empty poll response should deserialize");
        assert_eq!(empty.protocol_version, 1);
        assert!(empty.command.is_none());

        let challenge = challenge();
        for (payload, expected_outcome) in [
            (
                r#"{"protocolVersion":1,"command":{"kind":"release_admission_fence","challengeId":"challenge-1","accountId":"account-1","environmentId":"environment-1","machineId":"machine-1","kernelId":"kernel-1","desiredRevision":4,"idleSequence":8,"idleDeadlineAt":"2026-09-26T00:00:00.000Z","stopOperationId":"stop-operation-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","outcome":"stopped","resultSequence":1}}"#,
                ManagedKernelQuiescenceOutcome::Stopped,
            ),
            (
                r#"{"protocolVersion":1,"command":{"kind":"release_admission_fence","challengeId":"challenge-1","accountId":"account-1","environmentId":"environment-1","machineId":"machine-1","kernelId":"kernel-1","desiredRevision":4,"idleSequence":8,"idleDeadlineAt":"2026-09-26T00:00:00.000Z","stopOperationId":"stop-operation-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","outcome":"keep_running","resultSequence":1}}"#,
                ManagedKernelQuiescenceOutcome::KeepRunning,
            ),
        ] {
            let response: PollResponse =
                serde_json::from_str(payload).expect("release command response should deserialize");
            assert_eq!(response.protocol_version, 1);
            match response.command.expect("release command") {
                QuiescenceCommand::ReleaseAdmissionFence {
                    challenge: decoded,
                    outcome,
                    result_sequence,
                } => {
                    assert_eq!(decoded, challenge);
                    assert_eq!(outcome, expected_outcome);
                    assert_eq!(result_sequence, 1);
                }
                QuiescenceCommand::ReserveIdleForStop { .. } => {
                    panic!("release command decoded as reserve")
                }
            }
        }
    }

    #[test]
    fn cloud_release_ack_matches_canonical_v1_payload_and_hmac_fixture() {
        // Mirrors Cloud's signedQuiescenceReleaseAck fixture and its
        // mcred_${"a".repeat(43)} machine credential.
        let mut binding = binding();
        binding.machine_credential = format!("mcred_{}", "a".repeat(43));
        let client = ManagedKernelQuiescenceClient { binding };
        let release = client
            .signed_release_ack(&challenge(), ManagedKernelQuiescenceOutcome::KeepRunning, 1)
            .expect("release ack should sign");
        assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 473);
        assert_eq!(
            serde_json::to_string(&release).expect("release payload should serialize"),
            r#"{"accountId":"account-1","action":"auto_stop_quiescence_release_ack","challengeId":"challenge-1","environmentId":"environment-1","kernelId":"kernel-1","machineCredential":"mcred_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","machineId":"machine-1","nonce":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","outcome":"keep_running","protocolVersion":1,"resultSequence":1,"signature":"sha256:d2a9120425277132c10100e4c96ab3ee8172ae79ff82f8296ca6cae8c99502ec","stopOperationId":"stop-operation-1"}"#,
        );
        assert!(release["signature"]
            .as_str()
            .is_some_and(|signature| signature.starts_with("sha256:") && signature.len() == 71));
        assert!(release.get("result_sequence").is_none());
    }
}
