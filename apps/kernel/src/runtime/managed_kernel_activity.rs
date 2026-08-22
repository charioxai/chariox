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
}

struct ManagedKernelActivityBinding {
    api_url: String,
    account_id: String,
    environment_id: String,
    machine_id: String,
    kernel_id: String,
    machine_credential: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AcceptedActivity {
    sequence: u32,
    running_agent_count: u8,
}

#[derive(Debug, Default)]
struct ActivityCursor {
    accepted: Option<AcceptedActivity>,
    pending: Option<AcceptedActivity>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignedActivity<'a> {
    account_id: &'a str,
    environment_id: &'a str,
    kernel_id: &'a str,
    machine_id: &'a str,
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
            return Ok(None);
        };
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| activity_error("confirmed managed kernel has no Cloud relay profile"))?;
        let binding = ManagedKernelActivityBinding::from_runtime(config, registration, profile)?;
        Ok(Some(Self { binding }))
    }

    pub(crate) async fn run(
        self,
        runtime: KernelRuntimeState,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), DaemonError> {
        let mut cursor = ActivityCursor::default();
        let mut change_sequence = runtime.managed_activity_change_sequence();
        let mut running_agent_count = runtime.managed_running_agent_count();
        let mut retry_delay = MIN_RETRY_DELAY;

        loop {
            if *shutdown.borrow() {
                return Ok(());
            }

            if let Some(report) = cursor.next_report(running_agent_count)? {
                match self
                    .report(report.sequence, report.running_agent_count)
                    .await
                {
                    Ok(response) => {
                        let accepted = cursor.accept_response(response)?;
                        crate::logging::info_with_fields(
                            "managed_kernel.activity",
                            "managed kernel activity accepted",
                            serde_json::json!({
                                "environment_id": self.binding.environment_id,
                                "machine_id": self.binding.machine_id,
                                "kernel_id": self.binding.kernel_id,
                                "sequence": accepted.sequence,
                                "running_agent_count": accepted.running_agent_count,
                            }),
                        );
                        retry_delay = MIN_RETRY_DELAY;
                        continue;
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "managed_kernel.activity",
                            "managed kernel activity report failed; reporter will retry",
                            serde_json::json!({
                                "environment_id": self.binding.environment_id,
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
                                running_agent_count,
                            ) => {
                                (change_sequence, running_agent_count) = transition;
                                retry_delay = MIN_RETRY_DELAY;
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
                    running_agent_count,
                ) => {
                    (change_sequence, running_agent_count) = transition;
                    retry_delay = MIN_RETRY_DELAY;
                }
            }
        }
    }

    async fn report(
        &self,
        sequence: u32,
        running_agent_count: u8,
    ) -> Result<ReportActivityResponse, DaemonError> {
        let signature = activity_signature(&self.binding, sequence, running_agent_count)?;
        post_cloud_json(
            self.binding.api_url.clone(),
            ACTIVITY_ENDPOINT,
            serde_json::json!({
                "accountId": self.binding.account_id,
                "environmentId": self.binding.environment_id,
                "machineId": self.binding.machine_id,
                "kernelId": self.binding.kernel_id,
                "machineCredential": self.binding.machine_credential,
                "sequence": sequence,
                "runningAgentCount": running_agent_count,
                "signature": signature,
            }),
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
            environment_id: registration.environment_id.clone(),
            machine_id: machine_id.to_string(),
            kernel_id: registration.kernel_id.clone(),
            machine_credential: machine_credential.to_string(),
        })
    }
}

impl ActivityCursor {
    fn next_report(
        &mut self,
        running_agent_count: u8,
    ) -> Result<Option<AcceptedActivity>, DaemonError> {
        if let Some(pending) = self.pending {
            return Ok(Some(pending));
        }
        let report = match self.accepted {
            None => AcceptedActivity {
                sequence: 1,
                running_agent_count,
            },
            Some(accepted) if accepted.running_agent_count == running_agent_count => {
                return Ok(None);
            }
            Some(accepted) if accepted.sequence < MAX_ACTIVITY_SEQUENCE => AcceptedActivity {
                sequence: accepted.sequence + 1,
                running_agent_count,
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
        };
        self.accepted = Some(accepted);
        self.pending = None;
        Ok(accepted)
    }
}

fn activity_signature(
    binding: &ManagedKernelActivityBinding,
    sequence: u32,
    running_agent_count: u8,
) -> Result<String, DaemonError> {
    let canonical = serde_json::to_string(&SignedActivity {
        account_id: &binding.account_id,
        environment_id: &binding.environment_id,
        kernel_id: &binding.kernel_id,
        machine_id: &binding.machine_id,
        running_agent_count,
        sequence,
    })
    .map_err(|error| activity_error(format!("could not encode managed activity: {error}")))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(binding.machine_credential.as_bytes())
        .map_err(|error| activity_error(format!("could not sign managed activity: {error}")))?;
    mac.update(canonical.as_bytes());
    Ok(format!("sha256:{:x}", mac.finalize().into_bytes()))
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

    fn binding() -> ManagedKernelActivityBinding {
        ManagedKernelActivityBinding {
            api_url: "https://cloud.example.test".to_string(),
            account_id: "acct-1".to_string(),
            environment_id: "env-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            machine_credential: "mcred_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMN".to_string(),
        }
    }

    #[test]
    fn activity_signature_matches_cloud_canonical_json_vector() {
        assert_eq!(
            serde_json::to_string(&SignedActivity {
                account_id: "acct-1",
                environment_id: "env-1",
                kernel_id: "kernel-1",
                machine_id: "machine-1",
                running_agent_count: 1,
                sequence: 7,
            })
            .expect("activity should serialize"),
            "{\"accountId\":\"acct-1\",\"environmentId\":\"env-1\",\"kernelId\":\"kernel-1\",\"machineId\":\"machine-1\",\"runningAgentCount\":1,\"sequence\":7}"
        );
        assert_eq!(
            activity_signature(&binding(), 7, 1).expect("activity should sign"),
            "sha256:5bb7f722ce8a9f9e3086fe2255f0d6483b04e1e1905e767d613cecf92ea45b47"
        );
    }

    #[test]
    fn cursor_resynchronizes_from_cloud_before_sending_a_transition() {
        let mut cursor = ActivityCursor::default();
        assert_eq!(
            cursor.next_report(1).expect("report"),
            Some(AcceptedActivity {
                sequence: 1,
                running_agent_count: 1,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 9,
                running_agent_count: 0,
            })
            .expect("stale response should synchronize");
        assert_eq!(
            cursor.next_report(1).expect("report"),
            Some(AcceptedActivity {
                sequence: 10,
                running_agent_count: 1,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 10,
                running_agent_count: 1,
            })
            .expect("transition should synchronize");
        assert_eq!(cursor.next_report(1).expect("report"), None);
    }

    #[test]
    fn cursor_resynchronizes_equal_first_sequence_after_restart() {
        let mut cursor = ActivityCursor::default();
        assert_eq!(
            cursor.next_report(1).expect("restart report"),
            Some(AcceptedActivity {
                sequence: 1,
                running_agent_count: 1,
            })
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 0,
            })
            .expect("stored Cloud cursor should synchronize");
        assert_eq!(
            cursor.next_report(1).expect("corrective report"),
            Some(AcceptedActivity {
                sequence: 2,
                running_agent_count: 1,
            })
        );
    }

    #[test]
    fn cursor_replays_initial_report_after_lost_acknowledgement() {
        let mut cursor = ActivityCursor::default();
        let initial = AcceptedActivity {
            sequence: 1,
            running_agent_count: 0,
        };
        assert_eq!(
            cursor.next_report(0).expect("initial report"),
            Some(initial)
        );
        assert_eq!(
            cursor.next_report(1).expect("retry after local transition"),
            Some(initial),
            "the pending report must not change before acknowledgement"
        );
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 0,
            })
            .expect("initial report should be acknowledged");
        assert_eq!(
            cursor.next_report(1).expect("new transition"),
            Some(AcceptedActivity {
                sequence: 2,
                running_agent_count: 1,
            })
        );
    }

    #[test]
    fn cursor_preserves_later_pending_report_across_aba_change() {
        let mut cursor = ActivityCursor::default();
        cursor.next_report(1).expect("initial report");
        cursor
            .accept_response(ReportActivityResponse {
                accepted_sequence: 1,
                running_agent_count: 1,
            })
            .expect("initial report should be acknowledged");

        let idle = AcceptedActivity {
            sequence: 2,
            running_agent_count: 0,
        };
        assert_eq!(cursor.next_report(0).expect("idle report"), Some(idle));
        assert_eq!(
            cursor.next_report(1).expect("retry after ABA change"),
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
            cursor.next_report(1).expect("active correction"),
            Some(AcceptedActivity {
                sequence: 3,
                running_agent_count: 1,
            })
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
