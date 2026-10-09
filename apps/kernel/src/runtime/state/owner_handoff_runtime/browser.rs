//! MP-08 / MP-10 / MP-11 A07: scoped browser input and protected Vault save.
use super::*;
impl KernelRuntimeState {
    /// Revalidate the bound target under the input barrier, register any
    /// value for observation protection, then perform the one owner action.
    pub(super) async fn handoff_browser_action(
        &self,
        user: &str,
        actor: &str,
        admission: KernelBrowserAdmission,
        handoff: &RuntimeHandoff,
        id: &str,
        action: &HandoffResponseAction,
    ) -> HandoffOutcome {
        let kind = action.kind();
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let Ok(barrier) = protection.barrier(&scope) else {
            return outcome(
                id,
                HandoffStatus::Failed,
                kind,
                Some("protection_unavailable"),
            );
        };
        let _barrier = barrier.write_owned().await;
        let target = &handoff.target;
        if crate::session::unix_epoch_ms() >= handoff.expires_at_ms {
            return outcome(id, HandoffStatus::Expired, kind, Some("timeout"));
        }
        let Ok(source_admission) = self
            .owned
            .kernel_browser_host
            .admit(user, &handoff.agent_id)
        else {
            return outcome(id, HandoffStatus::Cancelled, kind, Some("source_revoked"));
        };
        let admission = self.handoff_action_admission(admission, source_admission.clone(), handoff);
        if self
            .owned
            .kernel_browser_host
            .check_admission(Some(&admission))
            .is_err()
        {
            return outcome(id, HandoffStatus::Cancelled, kind, Some("source_revoked"));
        }
        let fresh = self
            .kernel_browser_bound_operation(
                user,
                Some(&source_admission),
                "host.browser",
                json!({"op":"snapshot","tab_id":target.tab_id,"generation":target.generation}),
                true,
            )
            .await;
        let Some(snapshot) = fresh.ok().and_then(|value| {
            serde_json::from_value::<BrowserControllerStructuredSnapshot>(value["snapshot"].clone())
                .ok()
        }) else {
            return outcome(id, HandoffStatus::Failed, kind, Some("target_changed"));
        };
        let url = snapshot
            .document_url_for_node(&target.node_ref)
            .ok()
            .map(str::to_owned);
        let expected_url = self
            .owned
            .durable_state_store
            .load_subject_events_by_kind(id, "handoff.binding", 1)
            .ok()
            .and_then(|events| {
                events.last().and_then(|event| {
                    event.payload["document_url_digest"]
                        .as_str()
                        .map(str::to_owned)
                })
            });
        let same = expected_url.as_deref().is_some_and(|expected| {
            url.as_deref()
                .is_some_and(|url| document_url_binding(url) == expected)
        }) && snapshot
            .validate(&snapshot.target_id, &target.document_id)
            .is_ok()
            && snapshot
                .dom_nodes
                .iter()
                .find(|n| n.node_ref == target.node_ref)
                .is_some_and(|n| node_accepts(handoff.kind, n))
            && url
                .as_deref()
                .and_then(|url| origin_and_path(url).ok())
                .is_some_and(|(origin, path)| origin == target.origin && path == target.path);
        let (Some(url), true) = (url, same) else {
            return outcome(id, HandoffStatus::Failed, kind, Some("target_changed"));
        };
        let locator = match action {
            HandoffResponseAction::Click => BrowserLocatorAction::Click,
            HandoffResponseAction::EnterValue { value, .. } => BrowserLocatorAction::Fill {
                text: value.expose_secret().to_owned(),
                append: false,
                submit: false,
                expected_document_url: Some(url),
            },
            _ => return outcome(id, HandoffStatus::Failed, kind, Some("invalid_action")),
        };
        let command =
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Action {
                execution_id: format!("owner-{id}"),
                target_id: snapshot.target_id.clone(),
                document_id: target.document_id.clone(),
                node_ref: target.node_ref.clone(),
                action: locator.clone(),
                timeout_ms: 10_000,
            };
        // Observation protection covers the value before the page can echo it.
        if protection.register_command(&scope, &command).is_err() {
            return outcome(
                id,
                HandoffStatus::Failed,
                kind,
                Some("protection_unavailable"),
            );
        }
        match self
            .kernel_browser_bound_operation(
                user,
                Some(&admission),
                "host.secret",
                Self::handoff_input_params(target, actor, locator),
                true,
            )
            .await
        {
            Ok(_) => outcome(id, HandoffStatus::Completed, kind, None),
            Err(DaemonError::UserDomainRefused { .. }) => {
                outcome(id, HandoffStatus::Failed, kind, Some("target_changed"))
            }
            // Dispatch may have reached the page: report, never replay.
            Err(_) => outcome(id, HandoffStatus::Uncertain, kind, Some("input_uncertain")),
        }
    }

    /// MP-11 A07: carry the authenticated terminal identity into the existing
    /// browser input ledger, including an already-held same-terminal takeover.
    pub(in crate::runtime) fn handoff_input_params(
        target: &crate::session::HandoffTarget,
        actor: &str,
        action: BrowserLocatorAction,
    ) -> serde_json::Value {
        json!({"tab_id":target.tab_id,"generation":target.generation,
            "document_id":target.document_id,"node_ref":target.node_ref,
            "action":action,"observed_by":actor})
    }

    /// MP-11: preserve task, expiry and source grant authority through physical dispatch.
    pub(super) fn handoff_action_admission(
        &self,
        admission: KernelBrowserAdmission,
        source_admission: KernelBrowserAdmission,
        handoff: &RuntimeHandoff,
    ) -> KernelBrowserAdmission {
        let task_authority = self.owned.clone();
        let binding = handoff.clone();
        let browser_host = self.owned.kernel_browser_host.clone();
        let current_grant_identity = browser_host.grant_identity(&source_admission).ok();
        let source_fence = source_admission;
        let retained_grant_identity = self
            .owned
            .durable_state_store
            .load_subject_events_by_kind(
                &RuntimeHandoff::interaction_id(&handoff.obligation_id),
                "handoff.binding",
                1,
            )
            .ok()
            .and_then(|events| {
                events
                    .last()
                    .and_then(|event| event.payload["source_grant_identity"].as_str())
                    .map(str::to_owned)
            });
        admission.with_authority(move || {
            crate::session::unix_epoch_ms() < binding.expires_at_ms
                && retained_grant_identity
                    .as_ref()
                    .zip(current_grant_identity.as_ref())
                    .is_some_and(|(retained, current)| retained == current)
                && browser_host.check_admission(Some(&source_fence)).is_ok()
                && task_authority
                    .durable_state_store
                    .agent_tasks(None, Some(&binding.agent_id))
                    .ok()
                    .is_some_and(|tasks| {
                        tasks.iter().any(|task| {
                            task.task_id == binding.task_id
                                && !matches!(
                                task.state,
                                crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
                                    | crate::durable_state::agent_lifecycle::ExecutionState::Done
                            ) && task
                                .obligations
                                .iter()
                                .any(|o| o.id == binding.obligation_id && o.status == "open")
                        })
                    })
        })
    }

    pub(super) async fn save_handoff_secret(
        &self,
        handoff: &RuntimeHandoff,
        key: &str,
        value: &str,
    ) -> Result<(), DaemonError> {
        let host = url::Url::parse(&handoff.target.origin)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .ok_or_else(|| handoff_error("invalid hand-off origin"))?;
        let credential = crate::config::UserCredentialConfig {
            id: key.to_owned(),
            description: Some(format!("Saved by the owner during a hand-off on {host}")),
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: key.to_owned(),
            },
            allowed_hosts: vec![host],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: crate::config::UserCredentialInjectionConfig::Browser,
            metadata: None,
        };
        let service = self.home_runtime_secret_service()?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        self.upsert_observed_vault_credential(&service, &registry, credential, value, false)
            .await
            .map(|_| ())
    }
}
