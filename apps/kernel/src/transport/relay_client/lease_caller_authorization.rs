//! Caller binding for a disposable worker's remote lease requests.

use std::collections::BTreeMap;

use chariox_relay::protocol::{EncryptedRelayPayload, RelayCallerIdentity, RelayError};

use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::request_errors::relay_error;
use super::sender_identity::require_bound_kernel_sender;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LeaseCaller {
    home_kernel_id: String,
    realm_id: String,
    owner_user_id: String,
    sender_key_thumbprint: String,
}

#[derive(Debug, Clone)]
struct LeaseRecord {
    caller: LeaseCaller,
    destroyed: bool,
}

#[derive(Debug, Clone)]
struct AgentRecord {
    lease_id: String,
    destroyed: bool,
}

#[derive(Debug, Default)]
pub(super) struct LeaseCallerAuthorization {
    leases: BTreeMap<String, LeaseRecord>,
    agents: BTreeMap<String, AgentRecord>,
}

pub(super) enum LeaseCallerDecision {
    Continue(Option<LeaseCaller>),
    Replay(RelayPeerResponse),
}

enum Target<'a> {
    Ping,
    Reserve {
        home_kernel_id: &'a str,
        owner_user_id: &'a str,
    },
    Lease {
        id: &'a str,
        destroy: bool,
    },
    Agent {
        id: &'a str,
        destroy: bool,
    },
    Forbidden,
}

impl LeaseCallerAuthorization {
    pub(super) fn authorize(
        &self,
        request: &RelayPeerRequest,
        identity: Option<&RelayCallerIdentity>,
        encrypted_request: &EncryptedRelayPayload,
        source_kernel_id: &str,
    ) -> Result<LeaseCallerDecision, RelayError> {
        let target = target(request);
        if matches!(target, Target::Ping) {
            return Ok(LeaseCallerDecision::Continue(None));
        }
        if matches!(target, Target::Forbidden) {
            return Err(unauthorized(
                "lease worker only accepts remote lease operations",
            ));
        }
        let identity = require_bound_kernel_sender(identity, encrypted_request)?;
        let owner_user_id = identity
            .user_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| unauthorized("lease caller must identify its owner user"))?;
        if identity.subject != source_kernel_id || identity.realm_id.trim().is_empty() {
            return Err(unauthorized(
                "lease caller kernel or realm does not match the relay sender",
            ));
        }
        let caller = LeaseCaller {
            home_kernel_id: identity.subject.clone(),
            realm_id: identity.realm_id.clone(),
            owner_user_id: owner_user_id.to_string(),
            sender_key_thumbprint: identity
                .public_key_thumbprint
                .clone()
                .expect("bound kernel sender has a thumbprint"),
        };
        match target {
            Target::Reserve {
                home_kernel_id,
                owner_user_id,
            } => {
                if home_kernel_id != caller.home_kernel_id || owner_user_id != caller.owner_user_id
                {
                    return Err(unauthorized(
                        "lease home kernel or owner does not match its caller",
                    ));
                }
            }
            Target::Lease { id, destroy } => {
                let record = self
                    .leases
                    .get(id)
                    .ok_or_else(|| unauthorized("lease is not bound to this caller"))?;
                if record.caller != caller {
                    return Err(unauthorized("lease is bound to a different caller"));
                }
                if record.destroyed {
                    if destroy {
                        return Ok(LeaseCallerDecision::Replay(
                            RelayPeerResponse::ExecutionLeaseDestroyed {
                                lease_id: id.to_string(),
                            },
                        ));
                    }
                    return Err(unauthorized("lease has been destroyed"));
                }
            }
            Target::Agent { id, destroy } => {
                let agent = self
                    .agents
                    .get(id)
                    .ok_or_else(|| unauthorized("leased agent is not bound to this caller"))?;
                let lease = self
                    .leases
                    .get(&agent.lease_id)
                    .ok_or_else(|| unauthorized("leased agent has no bound lease"))?;
                if lease.caller != caller {
                    return Err(unauthorized("leased agent is bound to a different caller"));
                }
                if agent.destroyed || lease.destroyed {
                    if destroy {
                        return Ok(LeaseCallerDecision::Replay(
                            RelayPeerResponse::LeasedAgentDestroyed {
                                leased_agent_id: id.to_string(),
                            },
                        ));
                    }
                    return Err(unauthorized("leased agent has been destroyed"));
                }
            }
            Target::Ping | Target::Forbidden => unreachable!(),
        }
        Ok(LeaseCallerDecision::Continue(Some(caller)))
    }

    pub(super) fn record_response(&mut self, response: &RelayPeerResponse, caller: LeaseCaller) {
        match response {
            RelayPeerResponse::ExecutionLeaseCreated { lease, .. } => {
                self.leases.insert(
                    lease.id.clone(),
                    LeaseRecord {
                        caller,
                        destroyed: false,
                    },
                );
            }
            RelayPeerResponse::LeasedAgentSpawned { leased_agent } => {
                self.agents.insert(
                    leased_agent.id.clone(),
                    AgentRecord {
                        lease_id: leased_agent.lease_id.clone(),
                        destroyed: false,
                    },
                );
            }
            RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id } => {
                if let Some(record) = self.agents.get_mut(leased_agent_id) {
                    record.destroyed = true;
                }
            }
            RelayPeerResponse::ExecutionLeaseDestroyed { lease_id } => {
                if let Some(record) = self.leases.get_mut(lease_id) {
                    record.destroyed = true;
                }
            }
            _ => {}
        }
    }
}

fn target(request: &RelayPeerRequest) -> Target<'_> {
    match request {
        RelayPeerRequest::Ping { .. } => Target::Ping,
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id,
            owner_user_id,
            ..
        } => Target::Reserve {
            home_kernel_id,
            owner_user_id,
        },
        RelayPeerRequest::DestroyExecutionLease { lease_id } => Target::Lease {
            id: lease_id,
            destroy: true,
        },
        RelayPeerRequest::SpawnLeasedAgent { lease_id, .. } => Target::Lease {
            id: lease_id,
            destroy: false,
        },
        RelayPeerRequest::EnsureRemoteProviderAccount { context, .. } => Target::Lease {
            id: &context.execution_lease_id,
            destroy: false,
        },
        RelayPeerRequest::DestroyLeasedAgent { leased_agent_id } => Target::Agent {
            id: leased_agent_id,
            destroy: true,
        },
        RelayPeerRequest::UpdateLeasedAgentConfig {
            leased_agent_id, ..
        }
        | RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id, ..
        }
        | RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id, ..
        }
        | RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
            leased_agent_id, ..
        }
        | RelayPeerRequest::LaunchLeasedNativeProviderRun {
            leased_agent_id, ..
        }
        | RelayPeerRequest::SendLeasedNativeProviderInput {
            leased_agent_id, ..
        }
        | RelayPeerRequest::ResizeLeasedProviderTerminal {
            leased_agent_id, ..
        }
        | RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id, ..
        }
        | RelayPeerRequest::SteerLeasedPrompt {
            leased_agent_id, ..
        }
        | RelayPeerRequest::DrainLeasedRuntimeProjection {
            leased_agent_id, ..
        }
        | RelayPeerRequest::CompleteLeasedPrompt { leased_agent_id }
        | RelayPeerRequest::ObserveLeasedGitAfter {
            leased_agent_id, ..
        }
        | RelayPeerRequest::CancelLeasedPrompt { leased_agent_id } => Target::Agent {
            id: leased_agent_id,
            destroy: false,
        },
        RelayPeerRequest::EnsureRemoteSkillPackages { context, .. } => Target::Agent {
            id: &context.leased_agent_id,
            destroy: false,
        },
        RelayPeerRequest::CheckRemoteMcpAvailability { context, .. } => Target::Agent {
            id: &context.leased_agent_id,
            destroy: false,
        },
        _ => Target::Forbidden,
    }
}

fn unauthorized(message: &str) -> RelayError {
    relay_error("unauthorized", message, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_relay::auth::RelaySubjectKind;

    use crate::execution_lease::{ExecutionLease, LeasedAgent};
    use crate::runtime::terminal_pairings::public_key_thumbprint;
    use crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;

    fn encrypted_request() -> EncryptedRelayPayload {
        EncryptedRelayPayload {
            sender_public_key: "home-public-key".to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        }
    }

    fn identity() -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".to_string(),
            subject: "home-kernel".to_string(),
            subject_kind: RelaySubjectKind::Kernel,
            expires_at_ms: u64::MAX,
            token_id: Some("token-1".to_string()),
            user_id: Some("owner-1".to_string()),
            public_key_thumbprint: Some(public_key_thumbprint("home-public-key")),
        }
    }

    fn reserve_request() -> RelayPeerRequest {
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id: "home-kernel".to_string(),
            home_session_id: "home-session".to_string(),
            home_agent_id: "home-agent".to_string(),
            home_agent_metaagent: false,
            owner_user_id: "owner-1".to_string(),
        }
    }

    fn bound_authorization() -> LeaseCallerAuthorization {
        let mut authorization = LeaseCallerAuthorization::default();
        let caller = match authorization
            .authorize(
                &reserve_request(),
                Some(&identity()),
                &encrypted_request(),
                "home-kernel",
            )
            .expect("valid home kernel should reserve")
        {
            LeaseCallerDecision::Continue(Some(caller)) => caller,
            _ => panic!("reservation must bind a caller"),
        };
        let lease = ExecutionLease::new(
            "lease-1".to_string(),
            "home-kernel".to_string(),
            "home-session".to_string(),
            "home-agent".to_string(),
            false,
            "owner-1".to_string(),
            "worker-kernel".to_string(),
            "worker-machine".to_string(),
        );
        authorization.record_response(
            &RelayPeerResponse::ExecutionLeaseCreated {
                lease,
                relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            },
            caller,
        );
        authorization
    }

    fn spawn_request() -> RelayPeerRequest {
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id: "lease-1".to_string(),
            provider: "codex".to_string(),
            account_profile: "codex-1".to_string(),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            workspace_live_sync_mode: None,
            worktree_id: None,
            worktree_placement: None,
        }
    }

    fn record_agent(authorization: &mut LeaseCallerAuthorization) {
        let caller = match authorization
            .authorize(
                &spawn_request(),
                Some(&identity()),
                &encrypted_request(),
                "home-kernel",
            )
            .expect("bound home kernel should spawn")
        {
            LeaseCallerDecision::Continue(Some(caller)) => caller,
            _ => panic!("spawn must retain caller binding"),
        };
        let agent = LeasedAgent::new(
            "leased-agent-1".to_string(),
            "lease-1".to_string(),
            "home-agent".to_string(),
            "codex".to_string(),
            "codex-1".to_string(),
            None,
            None,
            None,
            None,
            "backing-session".to_string(),
            "backing-agent".to_string(),
            "backing-attachment".to_string(),
        );
        authorization.record_response(
            &RelayPeerResponse::LeasedAgentSpawned {
                leased_agent: agent,
            },
            caller,
        );
    }

    #[test]
    fn reservation_requires_a_live_bound_home_kernel_and_matching_owner() {
        let authorization = LeaseCallerAuthorization::default();
        let request = reserve_request();
        let encrypted = encrypted_request();
        assert!(authorization
            .authorize(&request, None, &encrypted, "home-kernel")
            .is_err());
        let mut caller = identity();
        caller.user_id = Some("other-user".to_string());
        assert!(authorization
            .authorize(&request, Some(&caller), &encrypted, "home-kernel")
            .is_err());
        let mut caller = identity();
        caller.expires_at_ms = 1;
        assert!(authorization
            .authorize(&request, Some(&caller), &encrypted, "home-kernel")
            .is_err());
        let mut caller = identity();
        caller.public_key_thumbprint = Some(public_key_thumbprint("different-key"));
        assert!(authorization
            .authorize(&request, Some(&caller), &encrypted, "home-kernel")
            .is_err());
        assert!(authorization
            .authorize(&request, Some(&identity()), &encrypted, "other-kernel")
            .is_err());
    }

    #[test]
    fn lease_and_agent_ids_do_not_authorize_a_different_caller() {
        let mut authorization = bound_authorization();
        record_agent(&mut authorization);
        let requests = [
            spawn_request(),
            RelayPeerRequest::CompleteLeasedPrompt {
                leased_agent_id: "leased-agent-1".to_string(),
            },
            RelayPeerRequest::ObserveLeasedGitAfter {
                leased_agent_id: "leased-agent-1".to_string(),
                provider_run_id: "run-1".to_string(),
            },
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: "leased-agent-1".to_string(),
            },
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: "lease-1".to_string(),
            },
        ];
        for request in &requests {
            let mut other_user = identity();
            other_user.user_id = Some("other-user".to_string());
            assert!(authorization
                .authorize(
                    request,
                    Some(&other_user),
                    &encrypted_request(),
                    "home-kernel"
                )
                .is_err());
            let mut other_realm = identity();
            other_realm.realm_id = "other-realm".to_string();
            assert!(authorization
                .authorize(
                    request,
                    Some(&other_realm),
                    &encrypted_request(),
                    "home-kernel"
                )
                .is_err());
            let mut other_kernel = identity();
            other_kernel.subject = "other-kernel".to_string();
            assert!(authorization
                .authorize(
                    request,
                    Some(&other_kernel),
                    &encrypted_request(),
                    "other-kernel"
                )
                .is_err());
            let mut other_key = identity();
            other_key.public_key_thumbprint = Some(public_key_thumbprint("other-home-key"));
            let mut other_encrypted = encrypted_request();
            other_encrypted.sender_public_key = "other-home-key".to_string();
            assert!(authorization
                .authorize(request, Some(&other_key), &other_encrypted, "home-kernel")
                .is_err());
        }
    }

    #[test]
    fn successful_deletion_leaves_caller_bound_tombstones() {
        let mut authorization = bound_authorization();
        record_agent(&mut authorization);
        let caller = match authorization
            .authorize(
                &spawn_request(),
                Some(&identity()),
                &encrypted_request(),
                "home-kernel",
            )
            .expect("bound caller should be accepted")
        {
            LeaseCallerDecision::Continue(Some(caller)) => caller,
            _ => panic!("spawn should continue"),
        };
        authorization.record_response(
            &RelayPeerResponse::LeasedAgentDestroyed {
                leased_agent_id: "leased-agent-1".to_string(),
            },
            caller.clone(),
        );
        assert!(matches!(
            authorization.authorize(
                &RelayPeerRequest::DestroyLeasedAgent {
                    leased_agent_id: "leased-agent-1".to_string(),
                },
                Some(&identity()),
                &encrypted_request(),
                "home-kernel",
            ),
            Ok(LeaseCallerDecision::Replay(
                RelayPeerResponse::LeasedAgentDestroyed { .. }
            ))
        ));
        authorization.record_response(
            &RelayPeerResponse::ExecutionLeaseDestroyed {
                lease_id: "lease-1".to_string(),
            },
            caller,
        );
        assert!(matches!(
            authorization.authorize(
                &RelayPeerRequest::DestroyExecutionLease {
                    lease_id: "lease-1".to_string(),
                },
                Some(&identity()),
                &encrypted_request(),
                "home-kernel",
            ),
            Ok(LeaseCallerDecision::Replay(
                RelayPeerResponse::ExecutionLeaseDestroyed { .. }
            ))
        ));
        assert!(authorization
            .authorize(
                &spawn_request(),
                Some(&identity()),
                &encrypted_request(),
                "home-kernel"
            )
            .is_err());
    }
}
