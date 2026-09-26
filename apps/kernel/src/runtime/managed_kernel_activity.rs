use std::time::Duration;

use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use crate::managed_bootstrap::ConfirmedManagedKernelRegistration;
use crate::runtime::cloud_api_client::post_cloud_json;
use crate::runtime::state::KernelRuntimeState;

const ACTIVITY_ENDPOINT: &str = "/v1/managed-kernels/activity";
const MAX_ACTIVITY_SEQUENCE: u32 = 2_147_483_647;
const MIN_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

pub(crate) struct ManagedKernelActivityReporter {
    binding: ManagedKernelActivityBinding,
    #[cfg(test)]
    confirmation_wait_started: Option<tokio::sync::mpsc::UnboundedSender<Duration>>,
    #[cfg(test)]
    persistence_retry_started: Option<tokio::sync::mpsc::UnboundedSender<Duration>>,
}

struct ManagedKernelActivityBinding {
    api_url: String,
    account_id: String,
    resource_id: String,
    worker: bool,
    machine_id: String,
    kernel_id: String,
    machine_credential: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AcceptedActivity {
    sequence: u32,
    running_agent_count: u8,
    activity_changed_at_ms: u64,
}

#[derive(Debug, Default)]
struct ActivityCursor {
    accepted: Option<AcceptedActivity>,
    pending: Option<AcceptedActivity>,
    requires_confirmation: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignedActivity<'a> {
    account_id: &'a str,
    environment_id: &'a str,
    kernel_id: &'a str,
    machine_id: &'a str,
    activity_changed_at: &'a str,
    running_agent_count: u8,
    sequence: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportActivityResponse {
    accepted_sequence: u32,
    running_agent_count: u8,
}

impl ManagedKernelActivityReporter {
    pub(crate) fn from_runtime(
        config: &DaemonConfig,
        registration: Option<&ConfirmedManagedKernelRegistration>,
    ) -> Result<Option<Self>, DaemonError> {
        let Some(registration) = registration else {
            let Some(path) =
                std::env::var_os(crate::managed_bootstrap::worker::ACTIVITY_RECEIPT_ENV)
            else {
                return Ok(None);
            };
            let profile = config
                .cloud_relay
                .as_ref()
                .ok_or_else(|| activity_error("disposable worker has no Cloud relay profile"))?;
            let allocation_id = crate::managed_bootstrap::worker::activity_allocation(
                std::path::Path::new(&path),
                config,
                profile,
            )?;
            return Ok(Some(Self {
                binding: ManagedKernelActivityBinding {
                    api_url: profile.api_url.trim_end_matches('/').to_string(),
                    account_id: profile.account_id.clone(),
                    resource_id: allocation_id,
                    worker: true,
                    machine_id: config.host_machine_id.clone(),
                    kernel_id: config.daemon_id.clone(),
                    machine_credential: profile
                        .machine_credential
                        .clone()
                        .ok_or_else(|| activity_error("worker Machine credential is missing"))?,
                },
                #[cfg(test)]
                confirmation_wait_started: None,
                #[cfg(test)]
                persistence_retry_started: None,
            }));
        };
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| activity_error("confirmed managed kernel has no Cloud relay profile"))?;
        let binding = ManagedKernelActivityBinding::from_runtime(config, registration, profile)?;
        Ok(Some(Self {
            binding,
            #[cfg(test)]
            confirmation_wait_started: None,
            #[cfg(test)]
            persistence_retry_started: None,
        }))
    }

    pub(crate) async fn run(
        self,
        runtime: KernelRuntimeState,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), DaemonError> {
        let mut cursor = ActivityCursor::default();
        runtime.ensure_managed_activity_tracking(&self.binding.kernel_id)?;
        let mut persistence_retry_delay = MIN_RETRY_DELAY;
        let Some((mut change_sequence, mut observation)) = self
            .retry_activity_snapshot(&runtime, &mut shutdown, &mut persistence_retry_delay, None)
            .await
        else {
            return Ok(());
        };
        let mut retry_delay = MIN_RETRY_DELAY;
        let mut confirmation_delay = MIN_RETRY_DELAY;

        loop {
            if *shutdown.borrow() {
                return Ok(());
            }

            if let Some(report) = cursor.next_report(observation)? {
                match self.report(report).await {
                    Ok(response) => {
                        let accepted = cursor.accept_response(response)?;
                        crate::logging::info_with_fields(
                            "managed_kernel.activity",
                            "managed kernel activity accepted",
                            serde_json::json!({
                                "resource_id": self.binding.resource_id,
                                "disposable_worker": self.binding.worker,
                                "machine_id": self.binding.machine_id,
                                "kernel_id": self.binding.kernel_id,
                                "sequence": accepted.sequence,
                                "running_agent_count": accepted.running_agent_count,
                            }),
                        );
                        let Some(snapshot) = self
                            .retry_activity_snapshot(
                                &runtime,
                                &mut shutdown,
                                &mut persistence_retry_delay,
                                None,
                            )
                            .await
                        else {
                            return Ok(());
                        };
                        (change_sequence, observation) = snapshot;
                        retry_delay = MIN_RETRY_DELAY;
                        if cursor.requires_confirmation {
                            crate::logging::warn_with_fields(
                                "managed_kernel.activity",
                                "Cloud activity cursor remains ahead; confirmation will be delayed",
                                serde_json::json!({
                                    "resource_id": self.binding.resource_id,
                                    "disposable_worker": self.binding.worker,
                                    "machine_id": self.binding.machine_id,
                                    "kernel_id": self.binding.kernel_id,
                                    "sequence": accepted.sequence,
                                    "confirmation_delay_ms": confirmation_delay.as_millis(),
                                }),
                            );
                            #[cfg(test)]
                            if let Some(wait_started) = &self.confirmation_wait_started {
                                let _ = wait_started.send(confirmation_delay);
                            }
                            let sleep = tokio::time::sleep(jittered(confirmation_delay));
                            tokio::pin!(sleep);
                            let snapshot_error = loop {
                                tokio::select! {
                                    changed = shutdown.changed() => {
                                        if changed.is_err() || *shutdown.borrow() {
                                            return Ok(());
                                        }
                                    }
                                    transition = runtime.wait_for_managed_activity_transition_after(
                                        change_sequence,
                                        observation,
                                    ) => {
                                        confirmation_delay = MIN_RETRY_DELAY;
                                        break transition.err();
                                    }
                                    _ = &mut sleep => {
                                        confirmation_delay = confirmation_delay
                                            .saturating_mul(2)
                                            .min(MAX_RETRY_DELAY);
                                        break None;
                                    }
                                }
                            };
                            let Some(snapshot) = self
                                .retry_activity_snapshot(
                                    &runtime,
                                    &mut shutdown,
                                    &mut persistence_retry_delay,
                                    snapshot_error,
                                )
                                .await
                            else {
                                return Ok(());
                            };
                            (change_sequence, observation) = snapshot;
                        } else {
                            confirmation_delay = MIN_RETRY_DELAY;
                        }
                        continue;
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "managed_kernel.activity",
                            "managed kernel activity report failed; reporter will retry",
                            serde_json::json!({
                                "resource_id": self.binding.resource_id,
                                "disposable_worker": self.binding.worker,
                                "machine_id": self.binding.machine_id,
                                "kernel_id": self.binding.kernel_id,
                                "sequence": report.sequence,
                                "running_agent_count": report.running_agent_count,
                                "retry_delay_ms": retry_delay.as_millis(),
                                "error": error.to_string(),
                            }),
                        );
                        let sleep = tokio::time::sleep(jittered(retry_delay));
                        tokio::pin!(sleep);
                        tokio::select! {
                            changed = shutdown.changed() => {
                                if changed.is_err() || *shutdown.borrow() {
                                    return Ok(());
                                }
                            }
                            transition = runtime.wait_for_managed_activity_transition_after(
                                change_sequence,
                                observation,
                            ) => {
                                let snapshot = match transition {
                                    Ok(snapshot) => Some(snapshot),
                                    Err(error) => self
                                        .retry_activity_snapshot(
                                            &runtime,
                                            &mut shutdown,
                                            &mut persistence_retry_delay,
                                            Some(error),
                                        )
                                        .await,
                                };
                                let Some(snapshot) = snapshot else {
                                    return Ok(());
                                };
                                (change_sequence, observation) = snapshot;
                                retry_delay = MIN_RETRY_DELAY;
                                confirmation_delay = MIN_RETRY_DELAY;
                            }
                            _ = &mut sleep => {
                                retry_delay = retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
                            }
                        }
                        continue;
                    }
                }
            }

            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                transition = runtime.wait_for_managed_activity_transition_after(
                    change_sequence,
                    observation,
                ) => {
                    let snapshot = match transition {
                        Ok(snapshot) => Some(snapshot),
                        Err(error) => self
                            .retry_activity_snapshot(
                                &runtime,
                                &mut shutdown,
                                &mut persistence_retry_delay,
                                Some(error),
                            )
                            .await,
                    };
                    let Some(snapshot) = snapshot else {
                        return Ok(());
                    };
                    (change_sequence, observation) = snapshot;
                    retry_delay = MIN_RETRY_DELAY;
                    confirmation_delay = MIN_RETRY_DELAY;
                }
            }
        }
    }

    async fn retry_activity_snapshot(
        &self,
        runtime: &KernelRuntimeState,
        shutdown: &mut tokio::sync::watch::Receiver<bool>,
        retry_delay: &mut Duration,
        mut first_error: Option<DaemonError>,
    ) -> Option<(u64, crate::runtime::state::ManagedActivityObservation)> {
        loop {
            if *shutdown.borrow() {
                return None;
            }
            let snapshot = match first_error.take() {
                Some(error) => Err(error),
                None => runtime.managed_activity_report_snapshot(),
            };
            match snapshot {
                Ok(snapshot) => {
                    *retry_delay = MIN_RETRY_DELAY;
                    return Some(snapshot);
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "managed_kernel.activity",
                        "managed activity persistence unavailable; reporter will retry",
                        serde_json::json!({
                            "resource_id": self.binding.resource_id,
                            "disposable_worker": self.binding.worker,
                            "machine_id": self.binding.machine_id,
                            "kernel_id": self.binding.kernel_id,
                            "retry_delay_ms": (*retry_delay).as_millis(),
                            "error": error.to_string(),
                        }),
                    );
                    #[cfg(test)]
                    if let Some(retry_started) = &self.persistence_retry_started {
                        let _ = retry_started.send(*retry_delay);
                    }
                    let sleep = tokio::time::sleep(jittered(*retry_delay));
                    tokio::pin!(sleep);
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                return None;
                            }
                        }
                        _ = &mut sleep => {
                            *retry_delay = (*retry_delay)
                                .saturating_mul(2)
                                .min(MAX_RETRY_DELAY);
                        }
                    }
                }
            }
        }
    }

    async fn report(
        &self,
        report: AcceptedActivity,
    ) -> Result<ReportActivityResponse, DaemonError> {
        let activity_changed_at = canonical_activity_timestamp(report.activity_changed_at_ms)?;
        let signature = activity_signature(
            &self.binding,
            report.sequence,
            report.running_agent_count,
            &activity_changed_at,
        )?;
        let mut payload = signed_activity_value(
            &self.binding,
            report.sequence,
            report.running_agent_count,
            &activity_changed_at,
        )?;
        payload["machineCredential"] = self.binding.machine_credential.clone().into();
        payload["signature"] = signature.into();
        post_cloud_json(
            self.binding.api_url.clone(),
            if self.binding.worker {
                "/v1/disposable-workers/activity"
            } else {
                ACTIVITY_ENDPOINT
            },
            payload,
        )
        .await
    }
}

impl ManagedKernelActivityBinding {
    fn from_runtime(
        config: &DaemonConfig,
        registration: &ConfirmedManagedKernelRegistration,
        profile: &PersistedCloudRelayProfile,
    ) -> Result<Self, DaemonError> {
        let machine_id = profile.machine_id.as_deref().ok_or_else(|| {
            activity_error("confirmed managed kernel has no Cloud Machine identity")
        })?;
        let machine_credential = profile.machine_credential.as_deref().ok_or_else(|| {
            activity_error("confirmed managed kernel has no Cloud Machine credential")
        })?;
        if profile.api_url.trim().is_empty()
            || profile.account_id.trim().is_empty()
            || machine_credential.trim().is_empty()
            || registration.environment_id.trim().is_empty()
            || registration.machine_id != machine_id
            || registration.machine_id != config.host_machine_id
            || registration.kernel_id != config.daemon_id
        {
            return Err(activity_error(
                "managed activity identity does not match the confirmed kernel registration",
            ));
        }
        Ok(Self {
            api_url: profile.api_url.trim_end_matches('/').to_string(),
            account_id: profile.account_id.clone(),
            resource_id: registration.environment_id.clone(),
            worker: false,
            machine_id: machine_id.to_string(),
            kernel_id: registration.kernel_id.clone(),
            machine_credential: machine_credential.to_string(),
        })
    }
}

impl ActivityCursor {
    fn next_report(
        &mut self,
        observation: crate::runtime::state::ManagedActivityObservation,
    ) -> Result<Option<AcceptedActivity>, DaemonError> {
        if let Some(pending) = self.pending {
            return Ok(Some(pending));
        }
        let report = match self.accepted {
            None => AcceptedActivity {
                sequence: 1,
                running_agent_count: observation.running_agent_count,
                activity_changed_at_ms: observation.changed_at_ms,
            },
            Some(accepted)
                if accepted.running_agent_count == observation.running_agent_count
                    && accepted.activity_changed_at_ms == observation.changed_at_ms
                    && !self.requires_confirmation =>
            {
                return Ok(None);
            }
            Some(accepted) if accepted.sequence < MAX_ACTIVITY_SEQUENCE => AcceptedActivity {
                sequence: accepted.sequence + 1,
                running_agent_count: observation.running_agent_count,
                activity_changed_at_ms: observation.changed_at_ms,
            },
            Some(_) => return Err(activity_error("managed activity sequence is exhausted")),
        };
        self.pending = Some(report);
        Ok(Some(report))
    }

    fn accept_response(
        &mut self,
        response: ReportActivityResponse,
    ) -> Result<AcceptedActivity, DaemonError> {
        let pending = self.pending.ok_or_else(|| {
            activity_error("Cloud returned managed activity without a pending report")
        })?;
        if response.accepted_sequence < pending.sequence
            || response.accepted_sequence > MAX_ACTIVITY_SEQUENCE
            || response.running_agent_count > 1
        {
            return Err(activity_error(
                "Cloud returned an invalid managed activity result",
            ));
        }
        let accepted = AcceptedActivity {
            sequence: response.accepted_sequence,
            running_agent_count: response.running_agent_count,
            activity_changed_at_ms: pending.activity_changed_at_ms,
        };
        self.requires_confirmation = response.accepted_sequence > pending.sequence;
        self.accepted = Some(accepted);
        self.pending = None;
        Ok(accepted)
    }
}

fn signed_activity_value(
    binding: &ManagedKernelActivityBinding,
    sequence: u32,
    running_agent_count: u8,
    activity_changed_at: &str,
) -> Result<serde_json::Value, DaemonError> {
    let mut value = serde_json::to_value(&SignedActivity {
        account_id: &binding.account_id,
        environment_id: &binding.resource_id,
        kernel_id: &binding.kernel_id,
        machine_id: &binding.machine_id,
        activity_changed_at,
        running_agent_count,
        sequence,
    })
    .map_err(|error| activity_error(format!("could not encode managed activity: {error}")))?;
    if binding.worker {
        let fields = value
            .as_object_mut()
            .expect("activity serializes as an object");
        let id = fields
            .remove("environmentId")
            .expect("activity contains environmentId");
        fields.insert("allocationId".to_string(), id);
    }
    Ok(value)
}

fn activity_signature(
    binding: &ManagedKernelActivityBinding,
    sequence: u32,
    running_agent_count: u8,
    activity_changed_at: &str,
) -> Result<String, DaemonError> {
    let value = signed_activity_value(binding, sequence, running_agent_count, activity_changed_at)?;
    // Sort explicitly so the signature does not depend on serde_json's map feature flags.
    let fields: std::collections::BTreeMap<_, _> = value
        .as_object()
        .expect("activity is an object")
        .iter()
        .collect();
    let canonical = serde_json::to_string(&fields)
        .map_err(|error| activity_error(format!("could not encode managed activity: {error}")))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(binding.machine_credential.as_bytes())
        .map_err(|error| activity_error(format!("could not sign managed activity: {error}")))?;
    mac.update(canonical.as_bytes());
    Ok(format!("sha256:{:x}", mac.finalize().into_bytes()))
}

fn canonical_activity_timestamp(timestamp_ms: u64) -> Result<String, DaemonError> {
    let timestamp_ms = i64::try_from(timestamp_ms)
        .map_err(|_| activity_error("managed activity timestamp overflows i64"))?;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| activity_error("managed activity timestamp is invalid"))
        .map(|timestamp| timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn jittered(delay: Duration) -> Duration {
    let maximum_jitter_ms = (delay.as_millis() / 4).min(u64::MAX as u128) as u64;
    let jitter_ms = rand::thread_rng().gen_range(0..=maximum_jitter_ms);
    delay.saturating_add(Duration::from_millis(jitter_ms))
}

fn activity_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "report managed kernel activity",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use crate::runtime::router::CommandRouter;
    use crate::DaemonApp;

    fn binding() -> ManagedKernelActivityBinding {
        ManagedKernelActivityBinding {
            api_url: "https://cloud.example.test".to_string(),
            account_id: "acct-1".to_string(),
            resource_id: "env-1".to_string(),
            worker: false,
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            machine_credential: "mcred_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN".to_string(),
        }
    }

    fn observation(
        running_agent_count: u8,
        changed_at_ms: u64,
    ) -> crate::runtime::state::ManagedActivityObservation {
        crate::runtime::state::ManagedActivityObservation {
            running_agent_count,
            changed_at_ms,
        }
    }

    #[test]
    fn worker_activity_uses_allocation_without_managed_environment_identity() {
        let mut worker = binding();
        worker.worker = true;
        let payload = signed_activity_value(&worker, 7, 1, "1970-01-01T00:00:01.000Z").unwrap();
        assert_eq!(payload["allocationId"], "env-1");
        assert!(payload.get("environmentId").is_none());
        assert!(payload.get("machineCredential").is_none());
        assert_eq!(
            activity_signature(&worker, 7, 1, "1970-01-01T00:00:01.000Z").unwrap(),
            "sha256:42b9ef2e9414baf065431cd5fc7df85e3cef537a6ba697ba43fe752348dbf1b9"
        );
        assert_ne!(
            activity_signature(&worker, 7, 1, "1970-01-01T00:00:01.000Z").unwrap(),
            activity_signature(&binding(), 7, 1, "1970-01-01T00:00:01.000Z").unwrap()
        );
    }

    #[tokio::test]
    async fn worker_report_posts_signed_allocation_to_worker_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.starts_with("POST /v1/disposable-workers/activity HTTP/1.1"));
            let payload = http_request_body(&request);
            assert_eq!(payload["allocationId"], "worker-1");
            assert!(payload.get("environmentId").is_none());
            let mut expected = binding();
            expected.worker = true;
            expected.resource_id = "worker-1".to_string();
            assert_eq!(
                payload["signature"],
                activity_signature(&expected, 7, 1, "1970-01-01T00:00:01.000Z").unwrap()
            );
            assert_eq!(payload["machineCredential"], expected.machine_credential);
            write_http_response(
                &mut stream,
                &serde_json::json!({
                    "acceptedSequence": 7, "runningAgentCount": 1,
                }),
            );
        });
        let mut worker = binding();
        worker.worker = true;
        worker.resource_id = "worker-1".to_string();
        worker.api_url = format!("http://{address}");
        let reporter = ManagedKernelActivityReporter {
            binding: worker,
            confirmation_wait_started: None,
            persistence_retry_started: None,
        };
        let response = reporter
            .report(AcceptedActivity {
                sequence: 7,
                running_agent_count: 1,
                activity_changed_at_ms: 1_000,
            })
            .await
            .unwrap();
        assert_eq!(response.accepted_sequence, 7);
        assert_eq!(response.running_agent_count, 1);
        fixture.join().unwrap();
    }

    #[test]
    fn activity_signature_matches_cloud_canonical_json_vector() {
        assert_eq!(
            canonical_activity_timestamp(1_000).expect("timestamp should format"),
            "1970-01-01T00:00:01.000Z"
        );
        assert_eq!(
            serde_json::to_string(&SignedActivity {
                account_id: "acct-1",
                environment_id: "env-1",
                kernel_id: "kernel-1",
                machine_id: "machine-1",
                activity_changed_at: "1970-01-01T00:00:01.000Z",
                running_agent_count: 1,
                sequence: 7,
            })
            .expect("activity should serialize"),
            "{\"accountId\":\"acct-1\",\"environmentId\":\"env-1\",\"kernelId\":\"kernel-1\",\"machineId\":\"machine-1\",\"activityChangedAt\":\"1970-01-01T00:00:01.000Z\",\"runningAgentCount\":1,\"sequence\":7}"
        );
        assert_eq!(
            activity_signature(&binding(), 7, 1, "1970-01-01T00:00:01.000Z")
                .expect("activity should sign"),
            "sha256:2e4cc0ff504b4222281a8bc3e08349cc78b9591fec4d0abd08ff14f1a1598351"
        );
    }

    #[test]
    fn cursor_resynchronizes_from_cloud_before_sending_a_transition() {
        let mut cursor = ActivityCursor::default();
        assert_eq!(
            cursor.next_report(observation(1, 1_000)).expect("report"),
            Some(AcceptedActivity {
                sequence: 1,
                running_agent_count: 1,
                activity_changed_at_ms: 1_000,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 9,
                running_agent_count: 0,
            })
            .expect("stale response should synchronize");
        assert_eq!(
            cursor.next_report(observation(1, 1_000)).expect("report"),
            Some(AcceptedActivity {
                sequence: 10,
                running_agent_count: 1,
                activity_changed_at_ms: 1_000,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 10,
                running_agent_count: 1,
            })
            .expect("transition should synchronize");
        assert_eq!(
            cursor.next_report(observation(1, 1_000)).expect("report"),
            None
        );
    }

    #[test]
    fn cursor_resynchronizes_equal_first_sequence_after_restart() {
        let mut cursor = ActivityCursor::default();
        assert_eq!(
            cursor
                .next_report(observation(1, 1_000))
                .expect("restart report"),
            Some(AcceptedActivity {
                sequence: 1,
                running_agent_count: 1,
                activity_changed_at_ms: 1_000,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 0,
            })
            .expect("stored Cloud cursor should synchronize");
        assert_eq!(
            cursor
                .next_report(observation(1, 1_000))
                .expect("corrective report"),
            Some(AcceptedActivity {
                sequence: 2,
                running_agent_count: 1,
                activity_changed_at_ms: 1_000,
            })
        );
    }

    #[test]
    fn cursor_confirms_same_count_after_cloud_resynchronization() {
        let mut cursor = ActivityCursor::default();
        assert_eq!(
            cursor
                .next_report(observation(0, 1_000))
                .expect("restart report"),
            Some(AcceptedActivity {
                sequence: 1,
                running_agent_count: 0,
                activity_changed_at_ms: 1_000,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 51,
                running_agent_count: 0,
            })
            .expect("stored Cloud cursor should synchronize");
        assert_eq!(
            cursor
                .next_report(observation(0, 1_000))
                .expect("post-start confirmation"),
            Some(AcceptedActivity {
                sequence: 52,
                running_agent_count: 0,
                activity_changed_at_ms: 1_000,
            }),
            "a replayed restart report must be followed by a fresh report even when the count is unchanged"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reporter_refreshes_activity_changed_while_resynchronization_is_in_flight() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
        let address = listener.local_addr().expect("activity fixture address");
        let (first_request_tx, first_request_rx) = tokio::sync::oneshot::channel();
        let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
        let (second_request_tx, second_request_rx) = tokio::sync::oneshot::channel();
        let fixture = std::thread::spawn(move || {
            let (mut first_stream, _) = listener.accept().expect("accept first activity report");
            let first_request = read_http_request(&mut first_stream);
            first_request_tx
                .send(first_request)
                .expect("publish first activity report");
            release_first_rx
                .recv()
                .expect("release first activity response");
            write_http_response(
                &mut first_stream,
                &serde_json::json!({
                    "acceptedSequence": 51,
                    "runningAgentCount": 0,
                }),
            );

            let (mut second_stream, _) = listener.accept().expect("accept confirmation report");
            let second_request = read_http_request(&mut second_stream);
            write_http_response(
                &mut second_stream,
                &serde_json::json!({
                    "acceptedSequence": 52,
                    "runningAgentCount": 1,
                }),
            );
            second_request_tx
                .send(second_request)
                .expect("publish confirmation report");
        });

        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let mut activity_binding = binding();
        activity_binding.api_url = format!("http://{address}");
        let reporter = ManagedKernelActivityReporter {
            binding: activity_binding,
            confirmation_wait_started: None,
            persistence_retry_started: None,
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_runtime = runtime.clone();
        let reporter_task =
            tokio::spawn(async move { reporter.run(reporter_runtime, shutdown_rx).await });

        let first_request = tokio::time::timeout(Duration::from_secs(2), first_request_rx)
            .await
            .expect("first activity report should arrive")
            .expect("first activity fixture should stay available");
        let first_body = http_request_body(&first_request);
        assert_eq!(first_body["sequence"], 1);
        assert_eq!(first_body["runningAgentCount"], 0);

        runtime.start_active_turn_with_trace_id(
            "session-1",
            "agent-1",
            "prompt-1",
            "provider-run-1",
            "trace-1",
        );
        runtime.record_waiting_room_change();
        runtime.record_managed_activity_transition_for_test();
        release_first_tx
            .send(())
            .expect("release first activity response");

        let second_request = tokio::time::timeout(Duration::from_secs(2), second_request_rx)
            .await
            .expect("confirmation activity report should arrive")
            .expect("activity fixture should stay available");
        let second_body = http_request_body(&second_request);
        assert_eq!(second_body["sequence"], 52);
        assert_eq!(
            second_body["runningAgentCount"], 1,
            "the confirmation must use activity observed while the restart report was in flight"
        );

        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_secs(2), reporter_task)
            .await
            .expect("activity reporter should stop")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        fixture.join().expect("activity fixture should stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_http_does_not_move_finished_agent_transition_time() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
        let address = listener.local_addr().expect("activity fixture address");
        let (first_request_tx, first_request_rx) = tokio::sync::oneshot::channel();
        let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
        let (idle_request_tx, idle_request_rx) = tokio::sync::oneshot::channel();
        let fixture = std::thread::spawn(move || {
            let (mut first_stream, _) = listener.accept().expect("accept active report");
            first_request_tx
                .send(http_request_body(&read_http_request(&mut first_stream)))
                .expect("publish active report");
            release_first_rx.recv().expect("release active response");
            write_http_response(
                &mut first_stream,
                &serde_json::json!({
                    "acceptedSequence": 1,
                    "runningAgentCount": 1,
                }),
            );

            let (mut idle_stream, _) = listener.accept().expect("accept idle report");
            let idle_request = http_request_body(&read_http_request(&mut idle_stream));
            write_http_response(
                &mut idle_stream,
                &serde_json::json!({
                    "acceptedSequence": 2,
                    "runningAgentCount": 0,
                }),
            );
            idle_request_tx
                .send(idle_request)
                .expect("publish idle report");
        });

        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        runtime
            .ensure_managed_activity_tracking("kernel-1")
            .expect("activate test tracking before activity");
        runtime.start_active_turn_with_trace_id(
            "session-1",
            "agent-1",
            "prompt-1",
            "provider-run-1",
            "trace-1",
        );
        runtime.record_waiting_room_change();
        runtime.record_managed_activity_transition_for_test();

        let mut activity_binding = binding();
        activity_binding.api_url = format!("http://{address}");
        let reporter = ManagedKernelActivityReporter {
            binding: activity_binding,
            confirmation_wait_started: None,
            persistence_retry_started: None,
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_runtime = runtime.clone();
        let reporter_task =
            tokio::spawn(async move { reporter.run(reporter_runtime, shutdown_rx).await });

        let first_request = tokio::time::timeout(Duration::from_secs(2), first_request_rx)
            .await
            .expect("active report should arrive")
            .expect("activity fixture should stay available");
        assert_eq!(first_request["runningAgentCount"], 1);

        runtime.clear_prompt_activity_for_managed_activity_test("provider-run-1");
        let (_, idle_observation) = runtime
            .managed_activity_report_snapshot()
            .expect("idle transition should already be durable");
        assert_eq!(idle_observation.running_agent_count, 0);
        let expected_changed_at =
            canonical_activity_timestamp(idle_observation.changed_at_ms).expect("canonical T0");
        release_first_tx
            .send(())
            .expect("release blocked active response");

        let idle_request = tokio::time::timeout(Duration::from_secs(2), idle_request_rx)
            .await
            .expect("idle report should arrive")
            .expect("activity fixture should stay available");
        assert_eq!(idle_request["sequence"], 2);
        assert_eq!(idle_request["runningAgentCount"], 0);
        assert_eq!(idle_request["activityChangedAt"], expected_changed_at);

        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_secs(2), reporter_task)
            .await
            .expect("activity reporter should stop")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        fixture.join().expect("activity fixture should stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transient_activity_persistence_failure_retries_original_transition() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
        let address = listener.local_addr().expect("activity fixture address");
        let (initial_request_tx, initial_request_rx) = tokio::sync::oneshot::channel();
        let (recovered_request_tx, recovered_request_rx) = tokio::sync::oneshot::channel();
        let fixture = std::thread::spawn(move || {
            let (mut initial_stream, _) = listener.accept().expect("accept initial report");
            let initial_request = http_request_body(&read_http_request(&mut initial_stream));
            write_http_response(
                &mut initial_stream,
                &serde_json::json!({
                    "acceptedSequence": 1,
                    "runningAgentCount": 0,
                }),
            );
            initial_request_tx
                .send(initial_request)
                .expect("publish initial report");

            let (mut recovered_stream, _) = listener.accept().expect("accept recovered report");
            let recovered_request = http_request_body(&read_http_request(&mut recovered_stream));
            write_http_response(
                &mut recovered_stream,
                &serde_json::json!({
                    "acceptedSequence": 2,
                    "runningAgentCount": 1,
                }),
            );
            recovered_request_tx
                .send(recovered_request)
                .expect("publish recovered report");
        });

        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let state_path = app.durable_state_store().path().to_path_buf();
        let app = Arc::new(Mutex::new(app));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let mut activity_binding = binding();
        activity_binding.api_url = format!("http://{address}");
        let (retry_started_tx, mut retry_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let reporter = ManagedKernelActivityReporter {
            binding: activity_binding,
            confirmation_wait_started: None,
            persistence_retry_started: Some(retry_started_tx),
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_runtime = runtime.clone();
        let reporter_task =
            tokio::spawn(async move { reporter.run(reporter_runtime, shutdown_rx).await });

        let initial_request = tokio::time::timeout(Duration::from_secs(2), initial_request_rx)
            .await
            .expect("initial report should arrive")
            .expect("activity fixture should stay available");
        assert_eq!(initial_request["runningAgentCount"], 0);

        let database = rusqlite::Connection::open(state_path)
            .expect("durable database should open for failure injection");
        database
            .execute_batch(
                "CREATE TRIGGER fail_managed_activity_append
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'managed_kernel.activity.changed'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected managed activity persistence failure');
                 END;",
            )
            .expect("activity persistence failure trigger should install");
        runtime.start_active_turn_with_trace_id(
            "session-1",
            "agent-1",
            "prompt-1",
            "provider-run-1",
            "trace-1",
        );
        runtime.record_managed_activity_transition_for_test();
        let transition_recorded_by = crate::session::unix_epoch_ms();
        runtime.record_waiting_room_change();

        let retry_delay = tokio::time::timeout(Duration::from_secs(2), retry_started_rx.recv())
            .await
            .expect("persistence retry should start")
            .expect("activity reporter should stay available");
        assert_eq!(retry_delay, MIN_RETRY_DELAY);
        assert!(!reporter_task.is_finished());
        database
            .execute_batch("DROP TRIGGER fail_managed_activity_append;")
            .expect("activity persistence failure trigger should be removed");

        let recovered_request = tokio::time::timeout(Duration::from_secs(3), recovered_request_rx)
            .await
            .expect("recovered report should arrive")
            .expect("activity fixture should stay available");
        assert_eq!(recovered_request["runningAgentCount"], 1);
        let recovered_changed_at = chrono::DateTime::parse_from_rfc3339(
            recovered_request["activityChangedAt"]
                .as_str()
                .expect("activity timestamp should be a string"),
        )
        .expect("activity timestamp should be canonical RFC3339")
        .timestamp_millis() as u64;
        assert!(recovered_changed_at <= transition_recorded_by);

        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_secs(2), reporter_task)
            .await
            .expect("activity reporter should stop")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        fixture.join().expect("activity fixture should stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activity_persistence_retry_wait_is_shutdown_responsive() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let state_path = app.durable_state_store().path().to_path_buf();
        let database = rusqlite::Connection::open(state_path)
            .expect("durable database should open for failure injection");
        database
            .execute_batch(
                "CREATE TRIGGER fail_managed_activity_append_on_start
                 BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'managed_kernel.activity.changed'
                 BEGIN
                   SELECT RAISE(FAIL, 'injected initial managed activity persistence failure');
                 END;",
            )
            .expect("initial activity failure trigger should install");
        let app = Arc::new(Mutex::new(app));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let (retry_started_tx, mut retry_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let reporter = ManagedKernelActivityReporter {
            binding: binding(),
            confirmation_wait_started: None,
            persistence_retry_started: Some(retry_started_tx),
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_task = tokio::spawn(async move { reporter.run(runtime, shutdown_rx).await });

        let retry_delay = tokio::time::timeout(Duration::from_secs(2), retry_started_rx.recv())
            .await
            .expect("initial persistence retry should start")
            .expect("activity reporter should stay available");
        assert_eq!(retry_delay, MIN_RETRY_DELAY);
        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_millis(250), reporter_task)
            .await
            .expect("persistence backoff should be shutdown responsive")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        database
            .execute_batch("DROP TRIGGER fail_managed_activity_append_on_start;")
            .expect("initial activity failure trigger should be removed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reporter_backs_off_when_cloud_cursor_stays_ahead() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
        let address = listener.local_addr().expect("activity fixture address");
        let (unexpected_third_tx, unexpected_third_rx) = tokio::sync::oneshot::channel();
        let fixture = std::thread::spawn(move || {
            for accepted_sequence in [51, 60] {
                let (mut stream, _) = listener.accept().expect("accept activity report");
                let _request = read_http_request(&mut stream);
                write_http_response(
                    &mut stream,
                    &serde_json::json!({
                        "acceptedSequence": accepted_sequence,
                        "runningAgentCount": 0,
                    }),
                );
            }

            listener
                .set_nonblocking(true)
                .expect("observe confirmation backoff");
            let deadline = std::time::Instant::now() + Duration::from_millis(750);
            let mut saw_third_request = false;
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((_stream, _)) => {
                        saw_third_request = true;
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("observe activity confirmation: {error}"),
                }
            }
            unexpected_third_tx
                .send(saw_third_request)
                .expect("publish confirmation observation");
        });

        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let mut activity_binding = binding();
        activity_binding.api_url = format!("http://{address}");
        let reporter = ManagedKernelActivityReporter {
            binding: activity_binding,
            confirmation_wait_started: None,
            persistence_retry_started: None,
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_task = tokio::spawn(async move { reporter.run(runtime, shutdown_rx).await });

        let saw_third_request = tokio::time::timeout(Duration::from_secs(4), unexpected_third_rx)
            .await
            .expect("confirmation observation should finish")
            .expect("activity fixture should stay available");
        assert!(
            !saw_third_request,
            "repeated ahead responses must not create an unthrottled confirmation loop"
        );

        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_secs(2), reporter_task)
            .await
            .expect("activity reporter should stop")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        fixture.join().expect("activity fixture should stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn successful_ahead_response_uses_confirmation_backoff_not_transport_backoff() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
        let address = listener.local_addr().expect("activity fixture address");
        let (confirmation_tx, confirmation_rx) = tokio::sync::oneshot::channel();
        let fixture = std::thread::spawn(move || {
            let (mut failed_stream, _) = listener.accept().expect("accept failed activity report");
            let _request = read_http_request(&mut failed_stream);
            drop(failed_stream);

            let (mut recovered_stream, _) =
                listener.accept().expect("accept recovered activity report");
            let _request = read_http_request(&mut recovered_stream);
            write_http_response(
                &mut recovered_stream,
                &serde_json::json!({
                    "acceptedSequence": 51,
                    "runningAgentCount": 0,
                }),
            );
            let accepted_at = std::time::Instant::now();

            let (mut confirmation_stream, _) = listener.accept().expect("accept confirmation");
            let confirmation = http_request_body(&read_http_request(&mut confirmation_stream));
            write_http_response(
                &mut confirmation_stream,
                &serde_json::json!({
                    "acceptedSequence": 52,
                    "runningAgentCount": 0,
                }),
            );
            confirmation_tx
                .send((accepted_at.elapsed(), confirmation))
                .expect("publish recovered confirmation");
        });

        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let mut activity_binding = binding();
        activity_binding.api_url = format!("http://{address}");
        let reporter = ManagedKernelActivityReporter {
            binding: activity_binding,
            confirmation_wait_started: None,
            persistence_retry_started: None,
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_task = tokio::spawn(async move { reporter.run(runtime, shutdown_rx).await });

        let (confirmation_delay, confirmation) =
            tokio::time::timeout(Duration::from_secs(5), confirmation_rx)
                .await
                .expect("recovered confirmation should arrive")
                .expect("activity fixture should stay available");
        assert_eq!(confirmation["sequence"], 52);
        assert_eq!(confirmation["runningAgentCount"], 0);
        assert!(
            confirmation_delay < Duration::from_millis(1_800),
            "transport failures must not inflate the first successful confirmation delay: {confirmation_delay:?}"
        );

        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_secs(2), reporter_task)
            .await
            .expect("activity reporter should stop")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        fixture.join().expect("activity fixture should stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_activity_transition_wakes_confirmation_backoff() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind activity fixture");
        let address = listener.local_addr().expect("activity fixture address");
        let (wait_started_tx, mut wait_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (transition_confirmation_tx, transition_confirmation_rx) =
            tokio::sync::oneshot::channel();
        let fixture = std::thread::spawn(move || {
            for accepted_sequence in [51, 60, 70] {
                let (mut stream, _) = listener.accept().expect("accept activity report");
                let _request = read_http_request(&mut stream);
                write_http_response(
                    &mut stream,
                    &serde_json::json!({
                        "acceptedSequence": accepted_sequence,
                        "runningAgentCount": 0,
                    }),
                );
            }
            let (mut stream, _) = listener.accept().expect("accept transition confirmation");
            let confirmation = http_request_body(&read_http_request(&mut stream));
            write_http_response(
                &mut stream,
                &serde_json::json!({
                    "acceptedSequence": 71,
                    "runningAgentCount": 1,
                }),
            );
            transition_confirmation_tx
                .send(confirmation)
                .expect("publish transition confirmation");
        });

        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let runtime = CommandRouter::with_interactive_capacity(app, 1).runtime_state();
        let mut activity_binding = binding();
        activity_binding.api_url = format!("http://{address}");
        let reporter = ManagedKernelActivityReporter {
            binding: activity_binding,
            confirmation_wait_started: Some(wait_started_tx),
            persistence_retry_started: None,
        };
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let reporter_runtime = runtime.clone();
        let reporter_task =
            tokio::spawn(async move { reporter.run(reporter_runtime, shutdown_rx).await });

        for expected_delay in [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ] {
            let actual_delay = tokio::time::timeout(Duration::from_secs(6), wait_started_rx.recv())
                .await
                .expect("confirmation wait should start")
                .expect("activity reporter should stay available");
            assert_eq!(actual_delay, expected_delay);
        }
        runtime.start_active_turn_with_trace_id(
            "session-1",
            "agent-1",
            "prompt-1",
            "provider-run-1",
            "trace-1",
        );
        runtime.record_waiting_room_change();
        runtime.record_managed_activity_transition_for_test();

        let confirmation = tokio::time::timeout(Duration::from_secs(1), transition_confirmation_rx)
            .await
            .expect("real activity transition should wake confirmation backoff")
            .expect("activity fixture should stay available");
        assert_eq!(confirmation["sequence"], 71);
        assert_eq!(confirmation["runningAgentCount"], 1);

        shutdown_tx.send(true).expect("stop activity reporter");
        tokio::time::timeout(Duration::from_secs(2), reporter_task)
            .await
            .expect("activity reporter should stop")
            .expect("activity reporter task should not panic")
            .expect("activity reporter should succeed");
        fixture.join().expect("activity fixture should stop");
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("activity fixture timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).expect("read activity request");
            assert!(read > 0, "activity request ended before its body arrived");
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                return String::from_utf8(request).expect("activity request UTF-8");
            }
        }
    }

    fn http_request_body(request: &str) -> serde_json::Value {
        let (_, body) = request
            .split_once("\r\n\r\n")
            .expect("activity request body");
        serde_json::from_str(body).expect("activity request JSON")
    }

    fn write_http_response(stream: &mut TcpStream, body: &serde_json::Value) {
        let body = body.to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        stream
            .write_all(response.as_bytes())
            .expect("write activity response");
    }

    #[test]
    fn cursor_replays_initial_report_after_lost_acknowledgement() {
        let mut cursor = ActivityCursor::default();
        let initial = AcceptedActivity {
            sequence: 1,
            running_agent_count: 0,
            activity_changed_at_ms: 1_000,
        };
        assert_eq!(
            cursor
                .next_report(observation(0, 1_000))
                .expect("initial report"),
            Some(initial)
        );
        assert_eq!(
            cursor
                .next_report(observation(1, 2_000))
                .expect("retry after local transition"),
            Some(initial),
            "the pending report and its transition timestamp must not change before acknowledgement"
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 0,
            })
            .expect("initial report should be acknowledged");
        assert_eq!(
            cursor
                .next_report(observation(1, 2_000))
                .expect("new transition"),
            Some(AcceptedActivity {
                sequence: 2,
                running_agent_count: 1,
                activity_changed_at_ms: 2_000,
            })
        );
    }

    #[test]
    fn cursor_preserves_later_pending_report_across_aba_change() {
        let mut cursor = ActivityCursor::default();
        cursor
            .next_report(observation(1, 1_000))
            .expect("initial report");
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 1,
            })
            .expect("initial report should be acknowledged");

        let idle = AcceptedActivity {
            sequence: 2,
            running_agent_count: 0,
            activity_changed_at_ms: 2_000,
        };
        assert_eq!(
            cursor
                .next_report(observation(0, 2_000))
                .expect("idle report"),
            Some(idle)
        );
        assert_eq!(
            cursor
                .next_report(observation(1, 3_000))
                .expect("retry after ABA change"),
            Some(idle),
            "the acknowledged Cloud state must be repaired before the latest local state"
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 2,
                running_agent_count: 0,
            })
            .expect("idle report should be acknowledged");
        assert_eq!(
            cursor
                .next_report(observation(1, 3_000))
                .expect("active correction"),
            Some(AcceptedActivity {
                sequence: 3,
                running_agent_count: 1,
                activity_changed_at_ms: 3_000,
            })
        );
    }

    #[test]
    fn cursor_reports_later_idle_transition_after_unobserved_busy_cycle() {
        let mut cursor = ActivityCursor::default();
        let first_idle = cursor
            .next_report(observation(0, 1_000))
            .expect("initial idle report")
            .expect("initial idle report should exist");
        assert_eq!(
            cursor
                .next_report(observation(1, 2_000))
                .expect("busy transition while request is pending"),
            Some(first_idle)
        );
        assert_eq!(
            cursor
                .next_report(observation(0, 3_000))
                .expect("later idle while request is pending"),
            Some(first_idle)
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 0,
            })
            .expect("first idle should be acknowledged");
        assert_eq!(
            cursor
                .next_report(observation(0, 3_000))
                .expect("later idle transition"),
            Some(AcceptedActivity {
                sequence: 2,
                running_agent_count: 0,
                activity_changed_at_ms: 3_000,
            }),
            "the intervening busy cycle must reset the idle transition clock"
        );
    }

    #[test]
    fn reporter_starts_only_for_an_exact_confirmed_managed_kernel() {
        let mut config = DaemonConfig::for_tests();
        config.host_machine_id = "machine-1".to_string();
        config.daemon_id = "kernel-1".to_string();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            account_id: "acct-1".to_string(),
            machine_id: Some("machine-1".to_string()),
            machine_credential: Some("mcred_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN".to_string()),
            ..PersistedCloudRelayProfile::default()
        });
        let registration = ConfirmedManagedKernelRegistration {
            environment_id: "env-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            context_plan: None,
        };
        assert!(ManagedKernelActivityReporter::from_runtime(&config, None)
            .expect("ordinary kernel should not fail")
            .is_none());
        assert!(
            ManagedKernelActivityReporter::from_runtime(&config, Some(&registration))
                .expect("managed kernel should configure")
                .is_some()
        );

        config.host_machine_id = "different-machine".to_string();
        assert!(ManagedKernelActivityReporter::from_runtime(&config, Some(&registration)).is_err());
    }
}
