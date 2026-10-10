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
struct PasteArgs {
    credential_id: String,
    tab_id: String,
    generation: u64,
    document_id: String,
    node_ref: String,
    #[serde(default)]
    submit: bool,
}
pub(super) fn paste_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: PASTE.into(),
        description: "MD-5: insert a Vault credential into an observed editable text field in a user-domain tab. Use tab/generation/document_id/node_ref from a fresh snapshot. Secret bytes never enter tool arguments/results. Default submit=false; use ordinary input to click a separate sign-in button.".into(),
        input_schema: json!({"type":"object","properties":{"credential_id":{"type":"string"},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"document_id":{"type":"string"},"node_ref":{"type":"string"},"submit":{"type":"boolean","default":false}},"required":["credential_id","tab_id","generation","document_id","node_ref"],"additionalProperties":false}),
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
        let _capture = if params["display_subscription_id"].is_string() {
            Some(gate.capture().await.map_err(host_error)?)
        } else {
            None
        };
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let at = std::time::Instant::now();
        let _barrier = protection.barrier(&scope)?.read_owned().await;
        crate::transport::kernel_browser_display::timing("barrier", at);
        self.kernel_browser_bound_operation(user, admission.as_ref(), method, params, true)
            .await
    }

    // Caller holds the scope's capture/input barrier for every protected operation.
    async fn kernel_browser_bound_operation(
        &self,
        user: &str,
        admission: Option<&KernelBrowserAdmission>,
        method: &str,
        params: Value,
        protect: bool,
    ) -> Result<Value, DaemonError> {
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let policy = if protect {
            protection.require(&scope, false)?;
            serde_json::from_str(&protection.capture_policy(&scope)?)
                .map_err(|_| host_error("MD-5: invalid observation policy".into()))?
        } else {
            Value::Null
        };
        let host = self.owned.kernel_browser_host.clone();
        let admission = admission.cloned();
        let (user, method) = (user.to_string(), method.to_string());
        let pixels = matches!(method.as_str(), "host.browser" | "host.computer")
            && (params["op"] == "screenshot" || params["op"] == "poll");
        let display = method == "host.browser"
            && params["op"] == "screenshot"
            && params["display_subscription_id"].is_string();
        let mirror = method == "host.browser" && params["op"] == "mirror_next";
        let at = std::time::Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            host.protected_request_admitted(&user, admission.as_ref(), &method, params, policy)
        })
        .await
        .map_err(|_| host_error("MD-5: browser task failed".into()))?
        .map_err(|message| protection.scrub_error(&scope, message.into_daemon("kernel_browser")))?;
        crate::transport::kernel_browser_display::timing("protected_host_ipc", at);
        if !protect {
            return Ok(result);
        }
        scrub_browser_result(protection, &scope, result, pixels, display, mirror)
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

    pub(super) async fn kernel_browser_paste_secret(
        &self,
        session: &str,
        agent: &crate::agent::AgentInstance,
        arguments: Value,
        admission: KernelBrowserAdmission,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args: PasteArgs = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-5: invalid Vault input arguments".into()))?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let user = user.as_str();
        require_vault_owner(user)?;
        self.owned
            .kernel_browser_host
            .check_admission(Some(&admission))
            .map_err(host_error)?;
        // Validate the observed document before prompting for unlock. Recheck
        // metadata, focus and target afterward, under the actual input barrier.
        let url = self
            .kernel_browser_secret_target_url(user, &admission, &args)
            .await?;
        self.home_runtime_secret_service()?
            .validate_browser_secret_input_for_target_url(&args.credential_id, &url)?;
        let _unlock = self
            .ensure_vault_unlocked_for_agent(session, agent.id(), "kernel_browser_paste_secret")
            .await?;
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let (service, _vault_guard) = self
            .scoped_secret_input_service(protection, &scope, &args.credential_id)
            .await?;
        let _barrier = protection.barrier(&scope)?.write_owned().await;
        let current = self.owned.agent_store.get_agent(agent.id())?;
        if current.session_id() != session
            || current.owner_user_id() != agent.owner_user_id()
            || self.owned.session_snapshot(session)?.focused_agent_id() != Some(agent.id())
        {
            return Err(host_error(
                "MD-3: current local focus required after Vault wait".into(),
            ));
        }
        require_vault_owner(
            &self.provider_account_authority_owner_user_id(current.owner_user_id()),
        )?;
        let url = self
            .kernel_browser_secret_target_url_bound(user, &admission, &args)
            .await?;
        let secret = zeroize::Zeroizing::new(
            service.browser_secret_input_for_target_url(&args.credential_id, &url)?,
        );
        // Reuse Room registration before any input handler, including failures.
        let snapshot = self
            .kernel_browser_bound_operation(
                user,
                Some(&admission),
                "host.browser",
                json!({"op":"snapshot","tab_id":args.tab_id,"generation":args.generation}),
                true,
            )
            .await?;
        let target = snapshot["snapshot"]["target_id"]
            .as_str()
            .ok_or_else(|| host_error("MD-5: missing target identity".into()))?;
        let action = BrowserLocatorAction::Fill {
            text: secret.to_string(),
            append: false,
            submit: args.submit,
            expected_document_url: Some(url),
        };
        let command =
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Action {
                execution_id: "kernel-browser-secret".into(),
                target_id: target.into(),
                document_id: args.document_id.clone(),
                node_ref: args.node_ref.clone(),
                action: action.clone(),
                timeout_ms: 10_000,
            };
        protection.register_command(&scope, &command)?;
        self.kernel_browser_bound_operation(user, Some(&admission), "host.secret", json!({"tab_id":args.tab_id,"generation":args.generation,"document_id":args.document_id,"node_ref":args.node_ref,"action":action}), true).await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: json!({"inserted":true}),
        })
    }

    async fn kernel_browser_secret_target_url(
        &self,
        user: &str,
        admission: &KernelBrowserAdmission,
        args: &PasteArgs,
    ) -> Result<String, DaemonError> {
        let scope = KernelBrowserHost::profile_key(user);
        let _barrier = self
            .owned
            .kernel_browser_secret_observations
            .barrier(&scope)?
            .read_owned()
            .await;
        self.kernel_browser_secret_target_url_bound(user, admission, args)
            .await
    }
    async fn kernel_browser_secret_target_url_bound(
        &self,
        user: &str,
        admission: &KernelBrowserAdmission,
        args: &PasteArgs,
    ) -> Result<String, DaemonError> {
        let result = self
            .kernel_browser_bound_operation(
                user,
                Some(admission),
                "host.browser",
                json!({"op":"snapshot","tab_id":args.tab_id,"generation":args.generation}),
                true,
            )
            .await?;
        let snapshot: crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot = serde_json::from_value(result["snapshot"].clone()).map_err(|_| host_error("MD-5: invalid browser snapshot".into()))?;
        snapshot
            .validate(&snapshot.target_id, &args.document_id)
            .map_err(host_error)?;
        let node = snapshot
            .dom_nodes
            .iter()
            .find(|node| node.node_ref == args.node_ref)
            .ok_or_else(|| host_error("MD-5: rediscover the fill field".into()))?;
        let editable = node.node_name.eq_ignore_ascii_case("input")
            || node.node_name.eq_ignore_ascii_case("textarea")
            || node.attributes.get("contenteditable").is_some_and(|value| {
                value.is_empty()
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("plaintext-only")
            });
        if !editable
            || node.attributes.contains_key("disabled")
            || node.attributes.contains_key("readonly")
        {
            return Err(host_error(
                "MD-5: Vault input requires an editable text field".into(),
            ));
        }
        snapshot
            .document_url_for_node(&args.node_ref)
            .map(str::to_string)
            .map_err(host_error)
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
) -> Result<Value, DaemonError> {
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

fn require_vault_owner(user: &str) -> Result<(), DaemonError> {
    // The configured Vault is host-owner state. Collaborators' browser profiles
    // remain independent and cannot resolve this owner's credential handles.
    if user != crate::session::DEFAULT_LOCAL_USER_ID {
        return Err(host_error(
            "MD-5: the kernel owner's Vault is unavailable to another user".into(),
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
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
        let wire = scrub_browser_result(&store, "stripe", result, true, true, false).unwrap();
        assert_eq!(wire["display_frame"], frame);
        assert_eq!(wire["frame_sent"], true);
        assert_eq!(wire["generation"], 1);
        for key in ["title", "url", "text"] {
            let text = wire[key].as_str().unwrap();
            for secret in ["AAAA", "stripes", "frame"] {
                assert!(!text.contains(secret));
            }
        }
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
        )
        .unwrap();
        for key in ["title", "url"] {
            for secret in ["AAAA", "stripes", "frame"] {
                assert!(!metadata["display_frame"][key]
                    .as_str()
                    .unwrap()
                    .contains(secret));
            }
        }
        assert!(metadata["observation"].get("frame").is_none());
        let scalar = scrub_browser_result(
            &store,
            "stripe",
            json!("AAAA stripes frame"),
            true,
            true,
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
            false
        )
        .is_err());
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
        let wire = scrub_browser_result(&store, scope, packet.clone(), false, false, true).unwrap();
        assert!(
            wire == packet,
            "MP-11: the final host mirror tree/hash must reach the client unchanged"
        );
        let ordinary =
            scrub_browser_result(&store, scope, json!({"text":"a"}), false, false, false).unwrap();
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
                    scrub_browser_result(&store, scope, packet.clone(), false, false, true).unwrap()
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
