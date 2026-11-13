//! MD-5: user-domain scope over the Room Vault/observation implementation.
use super::kernel_browser_runtime::{host_error, PASTE};
use super::*;
use crate::runtime::browser_controller_action::BrowserLocatorAction;
use crate::runtime::kernel_browser_host::{KernelBrowserAdmission, KernelBrowserHost};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PasteArgs {
    credential_id: String,
    tab_id: String,
    generation: u64,
    document_id: String,
    node_ref: String,
    #[serde(default)]
    submit: bool,
    /// A saved payment card fills one checkout field per insertion.
    #[serde(default)]
    card_field: Option<crate::config::PaymentCardField>,
    /// The payment as the agent states it, shown on the owner's approval.
    #[serde(default)]
    payment: Option<crate::local::VaultPaymentClaim>,
}
pub(super) fn paste_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: PASTE.into(),
        description: "MD-5: insert a Vault credential into an observed editable text field in a user-domain tab. Use tab/generation/document_id/node_ref from a fresh snapshot. Secret bytes never enter tool arguments/results. TOTP handles compute a current code at insertion; backup_codes consumes an unused code. A saved payment card fills one checkout field per call: set card_field and describe the payment in payment {merchant, amount, currency} on every call. The owner approves each payment once; then every field of that card for that payment at that site fills as often as the checkout needs (retries, reloads) until your turn ends or 15 minutes pass. Another site, merchant, amount or currency asks the owner again. For saved cards submit must be false: use the observed checkout button to pay after filling. A payment submits once; if its result is uncertain, inspect the receipt before starting another payment. Default submit=false; use ordinary input to click a separate sign-in button. With submit=true the field's native form is submitted and the result carries submitted:{navigated,url,title}: a form that navigates is a successful submit, so continue from that page and do not paste again.".into(),
        input_schema: json!({"type":"object","properties":{"credential_id":{"type":"string"},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"document_id":{"type":"string"},"node_ref":{"type":"string"},"submit":{"type":"boolean","default":false},"card_field":{"type":"string","enum":["number","expiry","expiry_month","expiry_year","security_code","name"]},"payment":{"type":"object","required":["merchant","amount","currency"],"properties":{"merchant":{"type":"string","minLength":1,"maxLength":100},"amount":{"type":"string","pattern":"^[0-9]{1,12}(\\.[0-9]{1,4})?$"},"currency":{"type":"string","pattern":"^[A-Z]{3}$"}},"additionalProperties":false}},"required":["credential_id","tab_id","generation","document_id","node_ref"],"additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_operation(
        &self,
        user: &str,
        agent: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, DaemonError> {
        if self.slice_kernel_id().is_some() {
            return Err(host_error(
                "MD-2: the home kernel owns the user-domain browser".into(),
            ));
        }
        let admission = agent
            .map(|agent| self.owned.kernel_browser_host.admit(user, agent))
            .transpose()
            .map_err(host_error)?;
        self.kernel_browser_operation_admitted(user, admission, method, params)
            .await
    }
    pub(super) async fn kernel_browser_operation_admitted(
        &self,
        user: &str,
        admission: Option<KernelBrowserAdmission>,
        method: &str,
        params: Value,
    ) -> Result<Value, DaemonError> {
        // Stop must remain available even when observation storage is fenced.
        if method == "host.browser" && params["op"] == "stop" {
            return self
                .kernel_browser_bound_operation(user, admission.as_ref(), method, params, false)
                .await;
        }
        let gate = self.owned.kernel_browser_host.display_gate(user);
        let _input = (method == "host.secret" || params["op"] == "input").then(|| gate.input());
        let _capture = if let Some(subscription) = params["display_subscription_id"].as_str() {
            Some(gate.capture(subscription).await.map_err(host_error)?)
        } else {
            None
        };
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let at = std::time::Instant::now();
        let _barrier = protection.barrier(&scope)?.read_owned().await;
        crate::transport::kernel_browser_display::timing("barrier", at);
        let released = params["op"] == "release";
        let result = self
            .kernel_browser_bound_operation(user, admission.as_ref(), method, params, true)
            .await;
        if released && result.is_ok() {
            self.schedule_agent_lifecycle_sweep();
        }
        result
    }

    // Caller holds the scope's capture/input barrier for every protected operation.
    pub(super) async fn kernel_browser_bound_operation(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        method: &str,
        params: Value,
        protect: bool,
    ) -> Result<Value, DaemonError> {
        if method == "host.browser" {
            self.owned
                .kernel_browser_host
                .set_attention(user, self.pending_handoff_tabs(user));
        }
        let mut params = params;
        if method == "host.browser" && params["op"] == "input" {
            if let Some(agent) = admission.and_then(KernelBrowserAdmission::agent_id) {
                params["_vault_payment_approvals"] =
                    self.vault_browser_payment_approvals(agent).into();
            }
        }
        let model_mutation = admission.is_some_and(KernelBrowserAdmission::is_agent)
            && (method == "host.secret"
                || (method == "host.browser"
                    && matches!(
                        params["op"].as_str(),
                        Some(
                            "stop"
                                | "close"
                                | "navigate"
                                | "resize"
                                | "input"
                                | "mirror_input"
                                | "display_input"
                        )
                    )));
        let capture = method == "host.secret" && params["op"] == "capture";
        let chosen_clipboard = method == "host.computer" && params["op"] == "clipboard_read";
        let tab = params["tab_id"].as_str().map(str::to_owned);
        if model_mutation && self.handoff_model_write_blocked(user, tab.as_deref()) {
            return Err(host_error("owner_step_pending: wait for the owner's hand-off completion before changing this tab".into()));
        }
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let policy = if protect {
            protection.require(&scope, false)?;
            self.kernel_browser_capture_policy(user)?
        } else {
            Value::Null
        };
        let host = self.owned.kernel_browser_host.clone();
        let admission = admission.cloned().map(|admission| {
            if model_mutation {
                let state = self.clone();
                let owner = user.to_owned();
                admission.with_authority(move || {
                    !state.handoff_model_write_blocked(&owner, tab.as_deref())
                })
            } else {
                admission
            }
        });
        let mut approval_ids: Vec<String> = params["_vault_payment_approvals"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|approval| approval["id"].as_str().map(str::to_owned))
            .collect();
        if let Some(id) = params["_vault_payment"]["id"].as_str() {
            approval_ids.push(id.to_owned());
        }
        let admission = admission.map(|admission| {
            let Some(actor) = admission.agent_id().map(str::to_owned) else {
                return admission;
            };
            if approval_ids.is_empty() {
                return admission;
            }
            let state = self.clone();
            admission.with_authority(move || {
                let live = state.vault_browser_payment_approvals(&actor);
                approval_ids.iter().all(|expected| {
                    live.iter()
                        .any(|approval| approval["id"] == expected.as_str())
                })
            })
        });
        let (user, method) = (user.to_string(), method.to_string());
        let pixels = matches!(method.as_str(), "host.browser" | "host.computer")
            && params["op"] == "screenshot";
        let display = method == "host.browser"
            && ((params["op"] == "screenshot" && params["display_subscription_id"].is_string())
                || params["op"] == "display_subscribe");
        let mirror = method == "host.browser" && params["op"] == "mirror_next";
        let snapshot = method == "host.browser" && params["op"] == "snapshot";
        let card_region = method == "host.browser" && params["op"] == "popup_region";
        let browser_state = method == "host.browser"
            && matches!(
                params["op"].as_str(),
                Some(
                    "start"
                        | "state"
                        | "open"
                        | "close"
                        | "navigate"
                        | "resize"
                        | "pointer_cursor"
                        | "input"
                        | "dialog"
                        | "cancel_download"
                        | "wait"
                        | "stop"
                )
            );
        let input =
            matches!(method.as_str(), "host.browser" | "host.computer") && params["op"] == "input";
        let at = std::time::Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            // MP-08/MP-10/MP-11: numeric queue attribution, never request data.
            if input {
                crate::transport::kernel_browser_display::timing("input_blocking_queue", at);
            }
            host.protected_request_admitted(&user, admission.as_ref(), &method, params, policy)
        })
        .await
        .map_err(|_| host_error("MD-5: browser task failed".into()))?
        .map_err(|message| protection.scrub_error(&scope, message.into_daemon("kernel_browser")))?;
        crate::transport::kernel_browser_display::timing("protected_host_ipc", at);
        // The capture reply is the value itself, headed for the Vault and never for the model:
        // scrubbing it would store the redaction marker when the value is already protected.
        if !protect || capture {
            return Ok(result);
        }
        // The agent explicitly chose to read its copied text. Field observations
        // remain masked; masks do not follow the value into the clipboard.
        if chosen_clipboard {
            return Ok(result);
        }
        if card_region {
            // The controller returns only binding IDs, geometry and fixed result
            // labels. Scrub the site origin without corrupting keys or IDs when
            // a protected value happens to be a short protocol substring.
            let mut result = result;
            if let Some(origin) = result.pointer_mut("/region/origin") {
                *origin = protection.scrub(&scope, origin.take())?;
            }
            return Ok(result);
        }
        if browser_state {
            return protection.scrub_kernel_browser_state(&scope, result);
        }
        scrub_browser_result(
            protection, &scope, result, pixels, display, mirror, snapshot,
        )
    }

    pub(super) async fn revoke_kernel_browser_observation_values(
        &self,
        key: &str,
    ) -> Result<(), DaemonError> {
        let protection = &self.owned.kernel_browser_secret_observations;
        // Profile-directory identity survives restart, so revocation discovers
        // dormant browsers too. Never enumerate or read their profile contents.
        for scope in self
            .owned
            .kernel_browser_host
            .profile_keys()
            .map_err(host_error)?
        {
            let _barrier = protection.barrier(&scope)?.write_owned().await;
            if protection.uses_vault_key(&scope, key)? {
                protection.forget(&scope)?;
            }
        }
        Ok(())
    }

    // `plain` is VAUDIT S3 fill_field: an unmasked target (an env var value box)
    // the kernel masks in model-bound observations instead of a password field.
    pub(super) async fn kernel_browser_paste_secret(
        &self,
        session: &str,
        agent: &crate::agent::AgentInstance,
        arguments: Value,
        admission: KernelBrowserAdmission,
        plain: bool,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args: PasteArgs = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-5: invalid Vault input arguments".into()))?;
        let target = SecretTarget::of(&args);
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let user = user.as_str();
        require_vault_owner(user)?;
        let check_authority = || {
            self.owned
                .kernel_browser_host
                .check_admission(Some(&admission))
                .map_err(host_error)
        };
        check_authority()?;
        // Validate the observed document before prompting for unlock. Recheck
        // metadata, focus and target afterward, under the actual input barrier.
        let url = self
            .kernel_browser_secret_target_url(user, &admission, &target, true)
            .await?;
        check_authority()?;
        let service = self.home_runtime_secret_service()?;
        let credential = service.credential(&args.credential_id)?.clone();
        if let Err(error) =
            service.validate_browser_secret_input_for_target_url(&args.credential_id, &url)
        {
            self.record_vault_refusal(
                &args.credential_id,
                &service.browser_vault_target(&args.credential_id, &url)?,
                agent.id(),
            )?;
            return Err(error);
        }
        if args.submit
            && credential
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.use_binding.as_ref())
                .is_some_and(|binding| {
                    binding.value_kind == crate::config::VaultValueKind::PaymentCard
                })
        {
            return Err(host_error("Fill a saved card with submit=false, then use the observed checkout button to pay.".into()));
        }
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        // Discovery fixes the protection epoch before either unlock or card consent.
        let epoch = protection.revision(&scope)?;
        let _unlock = self
            .ensure_vault_unlocked_for_agent(session, agent.id(), "kernel_browser_paste_secret")
            .await?;
        let signing_material = self
            .web_bot_auth_browser_material(session, agent.id())
            .await?;
        check_authority()?;
        let approval = if credential
            .metadata
            .as_ref()
            .is_some_and(|m| m.use_binding.is_some())
        {
            Some(
                self.authorize_vault_use(
                    agent.id(),
                    &credential,
                    &service.browser_vault_target(&args.credential_id, &url)?,
                    super::vault_use::VaultUseAsk {
                        agent_id: Some(agent.id()),
                        session_id: Some(session),
                        card_field: args.card_field,
                        claim: args.payment.as_ref(),
                    },
                )
                .await?,
            )
        } else if args.card_field.is_some() || args.payment.is_some() {
            return Err(host_error(
                "card_field and payment are only for payment cards".into(),
            ));
        } else {
            None
        };
        check_authority()?;
        let (service, _vault_guard) = self
            .scoped_secret_input_service(protection, &scope, &args.credential_id)
            .await?;
        let service = match approval {
            Some(approval) => service.with_vault_use_approval(approval),
            None => service,
        };
        check_authority()?;
        let _barrier = protection.barrier(&scope)?.write_owned().await;
        check_authority()?;
        let current = self.owned.agent_store.get_agent(agent.id())?;
        if current.session_id() != session || current.owner_user_id() != agent.owner_user_id() {
            return Err(host_error(
                "MD-3: the agent changed session or owner during the Vault wait".into(),
            ));
        }
        require_vault_owner(
            &self.provider_account_authority_owner_user_id(current.owner_user_id()),
        )?;
        if protection.revision(&scope)? != epoch {
            return Err(host_error(
                "MP-11: observation protection changed during the Vault wait; observe the page again".into(),
            ));
        }
        let url = self
            .kernel_browser_secret_target_url_bound(user, &admission, &target, true)
            .await?;
        check_authority()?;
        service.validate_browser_secret_input_for_target_url(&args.credential_id, &url)?;
        let attempt = service.begin_vault_use(
            &args.credential_id,
            &format!("Use attempted in browser by {}", agent.id()),
        )?;
        let inserted = self
            .insert_kernel_browser_secret(
                user,
                &admission,
                &args,
                &service,
                url,
                SecretFillContext {
                    actor: None,
                    signing_material,
                    plain,
                },
            )
            .await?;
        attempt.finish(&format!("Used in browser by {}", agent.id()))?;
        // A submit that left the page is the submit working: say where it went.
        let mut payload = json!({"inserted":true});
        if let Some(submitted) = inserted.get("submitted") {
            payload["submitted"] = submitted.clone();
        }
        Ok(RuntimeToolResult { ok: true, payload })
    }

    pub(super) async fn kernel_browser_terminal_paste_secret(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        credential_id: String,
        tab_id: String,
        generation: u64,
    ) -> Result<Value, DaemonError> {
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        require_vault_owner(&user)?;
        let admission = self
            .owned
            .kernel_browser_host
            .admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
        let snapshot = self.kernel_browser_operation_admitted(&user, Some(admission.clone()),
            "host.browser", json!({"op":"snapshot","tab_id":tab_id,"generation":generation,"observed_by":actor})).await?;
        let snapshot: crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot =
            serde_json::from_value(snapshot["snapshot"].clone()).map_err(|_| host_error("MD-5: invalid browser snapshot".into()))?;
        let node_ref = snapshot
            .accessibility_nodes
            .iter()
            .find(|node| {
                node.focused
                    && snapshot.dom_nodes.iter().any(|dom| {
                        dom.node_ref == node.node_ref
                            && (dom.node_name.eq_ignore_ascii_case("input")
                                || dom.node_name.eq_ignore_ascii_case("textarea")
                                || dom.attributes.contains_key("contenteditable"))
                    })
            })
            .map(|node| node.node_ref.clone())
            .ok_or_else(|| {
                host_error("Focus the page field to fill, then choose Fill from Vault.".into())
            })?;
        let args = PasteArgs {
            credential_id,
            tab_id,
            generation,
            document_id: snapshot.document_id,
            node_ref,
            submit: false,
            card_field: None,
            payment: None,
        };
        let url = self
            .kernel_browser_secret_target_url(&user, &admission, &SecretTarget::of(&args), true)
            .await?;
        let credential = self
            .home_runtime_secret_service()?
            .credential(&args.credential_id)?
            .clone();
        let approval = if credential
            .metadata
            .as_ref()
            .is_some_and(|m| m.use_binding.is_some())
        {
            // Only the owner's terminal fills a bound entry here.
            self.owner_client_user_id(caller)?;
            Some(
                self.authorize_vault_use(
                    &actor,
                    &credential,
                    &self
                        .home_runtime_secret_service()?
                        .browser_vault_target(&args.credential_id, &url)?,
                    Default::default(),
                )
                .await?,
            )
        } else {
            None
        };
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(&user);
        let epoch = protection.revision(&scope)?;
        let (service, _vault_guard) = self
            .scoped_secret_input_service(protection, &scope, &args.credential_id)
            .await?;
        let service = match approval {
            Some(approval) => service.with_vault_use_approval(approval),
            None => service,
        };
        let _barrier = protection.barrier(&scope)?.write_owned().await;
        if protection.revision(&scope)? != epoch {
            return Err(host_error(
                "The page protection changed. Focus the field and try again.".into(),
            ));
        }
        let url = self
            .kernel_browser_secret_target_url_bound(
                &user,
                &admission,
                &SecretTarget::of(&args),
                true,
            )
            .await?;
        let attempt =
            service.begin_vault_use(&args.credential_id, "Use attempted in browser by owner")?;
        self.insert_kernel_browser_secret(
            &user,
            &admission,
            &args,
            &service,
            url,
            SecretFillContext {
                actor: Some(&actor),
                signing_material: Value::Null,
                plain: false,
            },
        )
        .await?;
        attempt.finish("Used in browser by owner")?;
        Ok(json!({"inserted":true}))
    }

    async fn insert_kernel_browser_secret(
        &self,
        user: &str,
        admission: &KernelBrowserAdmission,
        args: &PasteArgs,
        service: &crate::secret::RuntimeSecretService,
        url: String,
        context: SecretFillContext<'_>,
    ) -> Result<Value, DaemonError> {
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        // Reuse Room registration before any input handler, including failures.
        let snapshot = self
            .kernel_browser_bound_operation(
                user,
                Some(admission),
                "host.browser",
                json!({"op":"snapshot","tab_id":args.tab_id,"generation":args.generation}),
                true,
            )
            .await?;
        let target = snapshot["snapshot"]["target_id"]
            .as_str()
            .ok_or_else(|| host_error("MD-5: missing target identity".into()))?;
        let secret = zeroize::Zeroizing::new(
            service.browser_secret_input_for_target_url(&args.credential_id, &url)?,
        );
        let action = BrowserLocatorAction::Fill {
            text: secret.to_string(),
            append: false,
            submit: args.submit,
            expected_document_url: Some(url.clone()),
        };
        let command =
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Action {
                agent_id: None,
                execution_id: "kernel-browser-secret".into(),
                target_id: target.into(),
                document_id: args.document_id.clone(),
                node_ref: args.node_ref.clone(),
                action: action.clone(),
                timeout_ms: 10_000,
            };
        protection.register_command(&scope, &command)?;
        let mut params = json!({"tab_id":args.tab_id,"generation":args.generation,"document_id":args.document_id,"node_ref":args.node_ref,"action":action});
        if args.card_field.is_some() {
            let origin = url::Url::parse(&url)
                .map_err(|_| {
                    host_error("The checkout URL is invalid. Observe the page again.".into())
                })?
                .origin()
                .ascii_serialization();
            let claim = json!(args.payment);
            let approval = admission
                .agent_id()
                .and_then(|agent| {
                    self.vault_browser_payment_approvals(agent)
                        .into_iter()
                        .find(|approval| {
                            approval["credential_id"] == args.credential_id
                                && approval["claim"] == claim
                                && approval["origin"] == origin
                        })
                })
                .ok_or_else(|| {
                    host_error(
                        "This payment approval ended. Fill the saved card again to ask.".into(),
                    )
                })?;
            params["_vault_payment"] = approval;
        }

        // OTP and recovery forms commonly use plain inputs. The registered
        // target protects model observations without changing the owner's field.
        params["vault_factor_input"] = matches!(
            service
                .validate_browser_secret_input_for_target_url(&args.credential_id, &url)?
                .injection,
            crate::config::UserCredentialInjectionConfig::Totp(_)
                | crate::config::UserCredentialInjectionConfig::BackupCodes
        )
        .into();
        params["_web_bot_auth"] = context.signing_material;
        if context.plain {
            params["plain_target"] = json!(true);
        }
        // MP-08/MP-11: the fill is the agent's input to an App Tab too.
        if admission.is_agent() {
            self.claim_app_view_tab(user, admission, &mut params)?;
        }
        if let Some(actor) = context.actor {
            params["observed_by"] = json!(actor);
        }
        self.kernel_browser_bound_operation(user, Some(admission), "host.secret", params, true)
            .await
    }

    pub(super) async fn kernel_browser_secret_target_url(
        &self,
        user: &str,
        admission: &KernelBrowserAdmission,
        target: &SecretTarget<'_>,
        editable: bool,
    ) -> Result<String, DaemonError> {
        let scope = KernelBrowserHost::profile_key(user);
        let _barrier = self
            .owned
            .kernel_browser_secret_observations
            .barrier(&scope)?
            .read_owned()
            .await;
        self.kernel_browser_secret_target_url_bound(user, admission, target, editable)
            .await
    }
    // Caller holds the scope's barrier. Every node must be in the observed
    // document; the URL is that of the first one.
    pub(super) async fn kernel_browser_secret_target_url_bound(
        &self,
        user: &str,
        admission: &KernelBrowserAdmission,
        target: &SecretTarget<'_>,
        editable: bool,
    ) -> Result<String, DaemonError> {
        let result = self
            .kernel_browser_bound_operation(
                user,
                Some(admission),
                "host.browser",
                json!({"op":"snapshot","tab_id":target.tab_id,"generation":target.generation}),
                true,
            )
            .await?;
        let snapshot: crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot = serde_json::from_value(result["snapshot"].clone()).map_err(|_| host_error("MD-5: invalid browser snapshot".into()))?;
        snapshot
            .validate(&snapshot.target_id, target.document_id)
            .map_err(host_error)?;
        let node = snapshot
            .dom_nodes
            .iter()
            .find(|node| node.node_ref == target.node_ref)
            .ok_or_else(|| host_error("MD-5: rediscover the fill field".into()))?;
        let url = snapshot
            .document_url_for_node(target.node_ref)
            .map(str::to_string)
            .map_err(host_error)?;
        if let Some(reveal) = target.reveal_node_ref {
            if snapshot.document_url_for_node(reveal).ok() != Some(url.as_str()) {
                return Err(host_error(
                    "VAUDIT: rediscover the reveal control; it must be in the same document as the value".into(),
                ));
            }
        }
        if !editable {
            return Ok(url);
        }
        let node_editable = node.node_name.eq_ignore_ascii_case("input")
            || node.node_name.eq_ignore_ascii_case("textarea")
            || node.attributes.get("contenteditable").is_some_and(|value| {
                value.is_empty()
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("plaintext-only")
            });
        if !node_editable
            || node.attributes.contains_key("disabled")
            || node.attributes.contains_key("readonly")
        {
            return Err(host_error(
                "MD-5: Vault input requires an editable text field".into(),
            ));
        }
        Ok(url)
    }
}

/// The observed page node a Vault operation binds to.
pub(super) struct SecretTarget<'a> {
    pub(super) tab_id: &'a str,
    pub(super) generation: u64,
    pub(super) document_id: &'a str,
    pub(super) node_ref: &'a str,
    pub(super) reveal_node_ref: Option<&'a str>,
}

impl<'a> SecretTarget<'a> {
    fn of(args: &'a PasteArgs) -> Self {
        Self {
            tab_id: &args.tab_id,
            generation: args.generation,
            document_id: &args.document_id,
            node_ref: &args.node_ref,
            reveal_node_ref: None,
        }
    }
}

// MP-08/MP-11: one boundary for protected host observations.
fn scrub_browser_result(
    protection: &super::room_secret_observation::RoomSecretObservations,
    scope: &str,
    mut result: Value,
    pixels: bool,
    display: bool,
    mirror: bool,
    snapshot: bool,
) -> Result<Value, DaemonError> {
    if snapshot {
        return protection.scrub_kernel_snapshot_result(scope, result);
    }
    if display && result["source_kind"] == "region" {
        // MP-11: these are trusted lease/codec fields, never page observations.
        protection.require(scope, false)?;
        return scrub_display_metadata(protection, scope, result, true);
    }
    if mirror {
        // MP-08/MP-11: the protected host converts matching page text and
        // metadata to compositor tiles before hashing its final wire tree.
        // A second generic scrub would invalidate that hash/incremental base
        // (and can corrupt protocol IDs/digests for short values). The caller
        // holds the policy/input barrier across the protected host request.
        protection.require(scope, false)?;
        return Ok(result);
    }
    if pixels && display && result.is_object() {
        // MP-08/MP-10/MP-11: this is a host-owned transport schema. Neither
        // fixed keys nor enum/binding strings are document observations.
        protection.require(scope, false)?;
        if let Some(frame) = result.get_mut("display_frame") {
            if !frame.is_null() && !frame.is_object() {
                return Err(host_error("MD-DISPLAY: invalid protected frame".into()));
            }
            *frame = scrub_display_metadata(protection, scope, frame.take(), true)?;
        }
        if let Some(object) = result.as_object_mut() {
            for (key, value) in object {
                if key != "display_frame" {
                    *value = scrub_display_metadata(protection, scope, value.take(), false)?;
                }
            }
        }
        return Ok(result);
    }
    // Trusted masked pixel bytes are opaque; scrubbing base64 as text would
    // corrupt images for short Vault values. All metadata still gets scrubbed.
    let pixel_path = if result.get("frame").is_some() {
        "/frame/data_base64"
    } else {
        "/data_base64"
    };
    let data = if pixels {
        result.pointer_mut(pixel_path).map(Value::take)
    } else {
        None
    };
    let mut result = protection.scrub(scope, result)?;
    if let Some(data) = data {
        if let Some(slot) = result.pointer_mut(pixel_path) {
            *slot = data;
        }
    }
    Ok(result)
}

// Display keys come from the fixed host contract. Scrub document-derived
// values while preserving opaque protected payloads and trusted frame fields.
fn scrub_display_metadata(
    protection: &super::room_secret_observation::RoomSecretObservations,
    scope: &str,
    mut value: Value,
    schema: bool,
) -> Result<Value, DaemonError> {
    // Admission was checked once above; primitives cannot carry text. Avoid
    // serialization/value-lock work for every tile coordinate and key flag.
    if matches!(&value, Value::Null | Value::Bool(_) | Value::Number(_)) {
        return Ok(value);
    }
    if !schema {
        return protection.scrub(scope, value);
    }
    match &mut value {
        Value::Object(object) => {
            for (key, child) in object {
                if schema
                    && matches!(
                        key.as_str(),
                        "kind"
                            | "codec"
                            | "format"
                            | "colour"
                            | "subscription_id"
                            | "tab_id"
                            | "document_id"
                            | "binding_id"
                            | "source_kind"
                            | "data_base64"
                    )
                {
                    continue;
                }
                let segment = schema && matches!(key.as_str(), "tiles" | "stripes");
                *child = scrub_display_metadata(protection, scope, child.take(), segment)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                *child = scrub_display_metadata(protection, scope, child.take(), schema)?;
            }
        }
        Value::String(_) => return protection.scrub(scope, value),
        _ => {}
    }
    Ok(value)
}

pub(super) fn require_vault_owner(user: &str) -> Result<(), DaemonError> {
    // The configured Vault is host-owner state. Collaborators' browser profiles
    // remain independent and cannot resolve this owner's credential handles.
    if user != crate::session::DEFAULT_LOCAL_USER_ID {
        return Err(host_error(
            "MD-5: the kernel owner's Vault is unavailable to another user".into(),
        ));
    }
    Ok(())
}

struct SecretFillContext<'a> {
    actor: Option<&'a str>,
    signing_material: Value,
    plain: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp11_snapshot_schema_survives_a_captured_schema_label() {
        use super::super::room_secret_observation::RoomSecretObservations;
        let root = crate::test_support::TestWorktree::new("snapshot-schema-label");
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        for secret in [
            "name",
            "value",
            "backend:7",
            "textbox",
            "snapshot",
            "generation",
            "PRIVATE-SECRET-ELEMENT",
        ] {
            store.register("snapshot", secret).unwrap();
        }
        let result = json!({"generation":1,"snapshot":{
            "browser_generation":1,"target_id":"name","document_id":"value","snapshot_revision":1,
            "accessibility_nodes":[{"node_ref":"backend:7","parent_ref":null,"child_refs":[],
                "role":"textbox","name":"Account name","description":"value","value":"name",
                "ignored":false,"disabled":false,"focused":false}],
            "dom_documents":[{"document_index":0,"url":"https://example.com/","owner_node_ref":null}],
            "dom_nodes":[{"node_ref":"backend:7","parent_ref":null,"document_index":0,"node_type":1,
                "node_name":"INPUT","text":"name","attributes":{"aria-label":"Account name"},"bounds":null},
                {"node_ref":"backend:8","parent_ref":null,"document_index":0,"node_type":1,
                "node_name":"PRIVATE-SECRET-ELEMENT","text":"","attributes":{},"bounds":null}]}});
        let worker_snapshot = serde_json::from_value(result["snapshot"].clone()).unwrap();
        let worker = store
            .scrub_response(
                "snapshot",
                crate::transport::room_browser_controller::RoomBrowserControllerResult::Snapshot {
                    snapshot: Some(worker_snapshot),
                },
            )
            .expect("worker snapshot schema remains valid");
        let crate::transport::room_browser_controller::RoomBrowserControllerResult::Snapshot {
            snapshot: Some(worker_snapshot),
        } = worker
        else {
            panic!("snapshot reply")
        };
        assert_eq!(
            worker_snapshot.accessibility_nodes[0].name,
            "Account [redacted]"
        );
        assert_eq!(worker_snapshot.target_id, "name");
        let output =
            scrub_browser_result(&store, "snapshot", result, false, false, false, true).unwrap();
        let snapshot: crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot =
            serde_json::from_value(output["snapshot"].clone()).expect("snapshot schema remains valid");
        assert_eq!(snapshot.target_id, "name");
        assert_eq!(snapshot.document_id, "value");
        assert_eq!(snapshot.accessibility_nodes[0].node_ref, "backend:7");
        assert_eq!(snapshot.accessibility_nodes[0].role, "textbox");
        assert_eq!(snapshot.accessibility_nodes[0].name, "Account [redacted]");
        assert_eq!(snapshot.accessibility_nodes[0].value, "[redacted]");
        assert_eq!(snapshot.dom_nodes[0].text, "[redacted]");
        assert_eq!(snapshot.dom_nodes[1].node_name, "[redacted]");
        assert_eq!(
            snapshot.dom_nodes[0].attributes["aria-label"],
            "Account [redacted]"
        );
        assert_eq!(
            store.scrub("snapshot", json!({"name":"value"})).unwrap(),
            json!({"[redacted]":"[redacted]"}),
            "arbitrary page maps still scrub their keys"
        );
    }

    #[test]
    fn mp08_mp11_region_lease_schema_survives_short_secret_scrubbing() {
        use super::super::room_secret_observation::RoomSecretObservations;
        let root = crate::test_support::TestWorktree::new("region-lease-boundary");
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        for secret in ["r", "d", "a"] {
            store.register("region", secret).unwrap();
        }
        let lease = serde_json::json!({"source_kind":"region","binding_id":"r","document_id":"d","subscription_id":"s","codec":"avc1.420033","css_width":100,"css_height":50,"sequence":3,"device_scale_factor":2});
        assert_eq!(
            scrub_browser_result(&store, "region", lease.clone(), false, true, false, false)
                .unwrap(),
            lease
        );
        assert_ne!(
            scrub_browser_result(
                &store,
                "region",
                serde_json::json!({"text":"r"}),
                false,
                false,
                false,
                false
            )
            .unwrap(),
            serde_json::json!({"text":"r"})
        );
    }
    #[test]
    fn mp08_mp11_mirror_wire_tree_survives_kernel_scrub_boundary() {
        use super::super::room_secret_observation::RoomSecretObservations;
        let root = crate::test_support::TestWorktree::new("mirror-wire-boundary");
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        let scope = "mirror-wire";
        // Short credentials must not corrupt protocol keys, node IDs or hashes.
        store.register(scope, "a").unwrap();
        let packet = json!({"hash":"a".repeat(64),"root":"n1","nodes":[{"id":"n1","parent":null,"children":[],"kind":"tile","tag":"div","reason":"protected_text"}],"resources":[],"tiles":[]});
        let wire =
            scrub_browser_result(&store, scope, packet.clone(), false, false, true, false).unwrap();
        assert!(
            wire == packet,
            "MP-11: the final host mirror tree/hash must reach the client unchanged"
        );
        let ordinary = scrub_browser_result(
            &store,
            scope,
            json!({"text":"a"}),
            false,
            false,
            false,
            false,
        )
        .unwrap();
        assert!(
            ordinary["text"] != "a",
            "MP-11: ordinary observations must still pass through the kernel scrubber"
        );
        // Optional actual Chromium packets are produced by the companion browser
        // regression. Replay the same production boundary before client.apply.
        if let Ok(path) = std::env::var("CHARIOX_MIRROR_WIRE_FIXTURE") {
            let fixture: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            store
                .register(scope, fixture["value"].as_str().unwrap())
                .unwrap();
            let packets = fixture["packets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|packet| {
                    scrub_browser_result(&store, scope, packet.clone(), false, false, true, false)
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert!(
                packets
                    .iter()
                    .zip(fixture["packets"].as_array().unwrap())
                    .all(|(wire, host)| wire == host),
                "MP-11: registration/fill and incremental packets changed at the kernel boundary"
            );
            std::fs::write(
                format!("{path}.wire.json"),
                serde_json::to_vec(&packets).unwrap(),
            )
            .unwrap();
        }
    }
    #[test]
    fn mp08_mp11_hydrated_stripes_survive_short_secret_scrubbing() {
        use super::super::room_secret_observation::RoomSecretObservations;
        use crate::transport::kernel_browser_display::encode_display_event;
        let root = crate::test_support::TestWorktree::new("stripe-scrub-boundary");
        let store = RoomSecretObservations::new(root.path().join("observations"), BTreeSet::new());
        for secret in ["AAAA", "stripes", "frame", "avc1", "srgb"] {
            store.register("stripe", secret).unwrap();
        }
        // MP-08/MP-10/MP-11: schema and binding collisions must survive the
        // production scrub -> event extraction -> CXD1 transport boundary.
        let packet = "AAAAAWU=";
        let frame = json!({"kind":"stripes","subscription_id":"frame-stripes",
            "tab_id":"frame-tab", "document_id":"stripes-document", "generation":1,
            "sequence":2, "width":1280, "height":800, "colour":"srgb",
            "stripes":[{"codec":"avc1.420033","key":true,"data_base64":packet}]});
        let result = json!({"display_frame":frame,"frame_sent":true,"generation":1,
            "title":"AAAA stripes frame","url":"https://example.org/stripes/frame",
            "text":"AAAA stripes frame"});
        let wire =
            scrub_browser_result(&store, "stripe", result, true, true, false, false).unwrap();
        assert_eq!(wire["display_frame"], frame);
        assert_eq!(wire["frame_sent"], true);
        assert_eq!(wire["generation"], 1);
        for key in ["title", "text"] {
            let text = wire[key].as_str().unwrap();
            for secret in ["AAAA", "stripes", "frame"] {
                assert!(!text.contains(secret));
            }
        }
        // A short value is masked as a whole word of visible text only, never inside a URL.
        assert_eq!(wire["url"], "https://example.org/stripes/frame");
        let mut reply = json!({"KernelBrowser":{"result":wire}});
        let (id, sequence, event) =
            crate::transport::kernel_browser_display::take_display_event(&mut reply).unwrap();
        assert_eq!(id, "frame-stripes");
        assert_eq!(sequence, 2);
        let encoded = encode_display_event(event).unwrap();
        assert!(encoded.ends_with(&[0, 0, 0, 1, 0x65]));
        let length = u32::from_be_bytes(encoded[4..8].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&encoded[8..8 + length]).unwrap();
        assert_eq!(header["frame"]["kind"], "stripes");
        assert_eq!(header["frame"]["tab_id"], "frame-tab");
        assert_eq!(header["frame"]["document_id"], "stripes-document");
        let mut observed = frame.clone();
        observed["title"] = json!("AAAA stripes frame");
        observed["url"] = json!("https://example.org/stripes/frame");
        let metadata = scrub_browser_result(
            &store,
            "stripe",
            json!({"display_frame":observed,"observation":{"frame":"stripes"}}),
            true,
            true,
            false,
            false,
        )
        .unwrap();
        for secret in ["AAAA", "stripes", "frame"] {
            assert!(!metadata["display_frame"]["title"]
                .as_str()
                .unwrap()
                .contains(secret));
        }
        assert_eq!(
            metadata["display_frame"]["url"],
            "https://example.org/stripes/frame"
        );
        assert!(metadata["observation"].get("frame").is_none());
        let scalar = scrub_browser_result(
            &store,
            "stripe",
            json!("AAAA stripes frame"),
            true,
            true,
            false,
            false,
        )
        .unwrap();
        assert!(!scalar.as_str().unwrap().contains("AAAA"));

        assert!(scrub_browser_result(
            &store,
            "stripe",
            json!({"display_frame":"invalid"}),
            true,
            true,
            false,
            false
        )
        .is_err());
    }

    #[test]
    fn host_vault_is_owner_only_and_tool_never_accepts_secret_bytes() {
        assert!(require_vault_owner(crate::session::DEFAULT_LOCAL_USER_ID).is_ok());
        assert!(require_vault_owner("collaborator").is_err());
        let arguments = json!({"credential_id":"login","tab_id":"tab","generation":1,"document_id":"doc","node_ref":"backend:1","text":"forbidden"});
        assert!(serde_json::from_value::<PasteArgs>(arguments).is_err());
        let schema = paste_spec().input_schema;
        assert!(schema["properties"].get("text").is_none());
    }
    #[tokio::test]
    async fn vault_retirement_finds_dormant_user_profiles_and_preserves_sealed_echoes() {
        use super::super::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
        use super::super::room_secret_observation::RoomSecretObservations;
        let root = crate::test_support::TestWorktree::new("md5-protection");
        let mut room = TestRoom::new("md5-retirement");
        let config = room.runtime.owned.config_projection.snapshot();
        let host = KernelBrowserHost::new(root.path().join("runtime"));
        std::fs::create_dir_all(host.profile_root("local")).unwrap();
        room.runtime.owned.kernel_browser_host = host;
        let path = root.path().join("observations");
        room.runtime.owned.kernel_browser_secret_observations =
            RoomSecretObservations::new(path.clone(), BTreeSet::new())
                .with_identity(&config.relay_private_key);
        let scope = KernelBrowserHost::profile_key("local");
        let store = &room.runtime.owned.kernel_browser_secret_observations;
        store
            .register_credential_source(&scope, Some("login"))
            .unwrap();
        store
            .register(&scope, "synthetic-md5-retired-only")
            .unwrap();
        room.runtime
            .revoke_vault_observation_values("other-key")
            .await
            .unwrap();
        assert!(store.uses_vault_key(&scope, "login").unwrap());
        room.runtime
            .revoke_vault_observation_values("login")
            .await
            .unwrap();
        assert!(!store.uses_vault_key(&scope, "login").unwrap());
        assert_eq!(
            store.scrub_text_or_withhold(&scope, "echo synthetic-md5-retired-only"),
            "echo [redacted]"
        );
        let recovered = RoomSecretObservations::new(path, BTreeSet::new())
            .with_identity(&config.relay_private_key);
        assert_eq!(
            recovered.scrub_text_or_withhold(&scope, "synthetic-md5-retired-only"),
            "[redacted]"
        );
        assert_eq!(recovered.controller_values(&scope).unwrap().len(), 1);
        assert_eq!(
            recovered.scrub_text_or_withhold(&KernelBrowserHost::profile_key("other"), "benign"),
            "benign"
        );
    }
}
