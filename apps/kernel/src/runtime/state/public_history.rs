//! MP-08 / MP-10 / MP-11, A09: public projection and scoped history service.
use super::*;
use crate::history::public_history::PublicHistoryDocument;
use crate::history::{HistoryEvent, HistoryEventKind};
use crate::session::SessionStatus;
use crate::transport::runtime_tools::{HistorySearchArgs, RuntimeToolResult};

impl room_secret_observation::RoomSecretObservations {
    pub(super) fn public_history_document(
        &self,
        event: &HistoryEvent,
        owner: &str,
    ) -> Option<PublicHistoryDocument> {
        if !matches!(
            event.kind,
            HistoryEventKind::UserPrompt
                | HistoryEventKind::ProviderOutput
                | HistoryEventKind::ProviderTool
                | HistoryEventKind::ProviderError
        ) || event
            .metadata
            .get("legacy_history_import")
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            return None;
        }
        if [
            "private",
            "protected",
            "hidden",
            "provider_login",
            "handoff_input",
        ]
        .iter()
        .any(|key| event.metadata.get(*key).and_then(|v| v.as_bool()) == Some(true))
            || event
                .metadata
                .get("visibility")
                .and_then(|v| v.as_str())
                .is_some_and(|v| matches!(v, "private" | "protected" | "hidden"))
        {
            return None;
        }
        let room = event.session_id.as_deref()?;
        self.require(room, false).ok()?;
        let mut protected = self.protect_history_events(vec![event.clone()]);
        let protected = protected.pop()?;
        let text = protected.content?;
        if text.trim().is_empty()
            || text.starts_with("[sensitive Room")
            || text.starts_with("[recovered sensitive")
        {
            return None;
        }
        // Provider-normalized tool records can contain private arguments. Exclude
        // auth/protected tools entirely and omit private keys before indexing.
        let text = if event.kind == HistoryEventKind::ProviderTool {
            let value: serde_json::Value = serde_json::from_str(&text).ok()?;
            if private_tool(&value) {
                return None;
            }
            serde_json::to_string(&public_tool_value(value)).ok()?
        } else {
            text
        };
        let document = PublicHistoryDocument {
            event_ref: event.event_id.clone(),
            sequence: event.sequence,
            timestamp_ms: event.timestamp_ms,
            owner_user_id: owner.to_owned(),
            session_id: room.to_owned(),
            agent_id: event.agent_id.clone()?,
            kind: event.kind,
            turn_id: event.turn_id.clone().or_else(|| event.prompt_id.clone()),
            truncated: text.chars().count() > 64 * 1024,
            text: text.chars().take(64 * 1024).collect(),
        };
        // Canonical scope/reference fields are internal authority, never rewritten
        // into another identity. A secret collision excludes the entire document.
        let scrubbed = self.scrub(room, document.clone()).ok()?;
        (scrubbed == document).then_some(document)
    }
}

fn private_tool(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(values) => {
            ["tool", "name", "tool_name"]
                .iter()
                .filter_map(|key| values.get(*key).and_then(|v| v.as_str()))
                .any(|name| {
                    let name = name.to_ascii_lowercase();
                    [
                        "credential",
                        "vault",
                        "passkey",
                        "handoff",
                        "hand_off",
                        "login",
                        "auth",
                        "secret",
                    ]
                    .iter()
                    .any(|private| name.contains(private))
                })
                || values.values().any(private_tool)
        }
        serde_json::Value::Array(values) => values.iter().any(private_tool),
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .filter(|v| v.is_object() || v.is_array())
            .is_some_and(|v| private_tool(&v)),
        _ => false,
    }
}

fn public_tool_value(value: serde_json::Value) -> serde_json::Value {
    fn project(value: serde_json::Value, depth: usize) -> serde_json::Value {
        use serde_json::Value;
        if depth > 16 {
            return Value::Null;
        }
        match value {
            Value::Object(values)
                if values
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        matches!(
                            kind,
                            "image"
                                | "input_image"
                                | "audio"
                                | "input_audio"
                                | "file"
                                | "resource"
                                | "resource_link"
                                | "reasoning"
                                | "thinking"
                        )
                    }) =>
            {
                Value::Null
            }
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .filter_map(|(key, value)| {
                        let name = key.to_ascii_lowercase();
                        if [
                            "password",
                            "secret",
                            "token",
                            "credential",
                            "authorization",
                            "cookie",
                            "private",
                            "reasoning",
                            "thinking",
                            "system",
                            "hidden",
                            "overlay",
                            "attachment",
                            "base64",
                            "api_key",
                            "apikey",
                            "raw",
                            "headers",
                            "image_url",
                            "imageurl",
                            "data_url",
                        ]
                        .iter()
                        .any(|private| name.contains(private))
                        {
                            None
                        } else {
                            Some((key, project(value, depth + 1)))
                        }
                    })
                    .collect(),
            ),
            Value::Array(values) => {
                Value::Array(values.into_iter().map(|v| project(v, depth + 1)).collect())
            }
            Value::String(text) => match serde_json::from_str(&text) {
                Ok(structured @ (Value::Object(_) | Value::Array(_))) => {
                    project(structured, depth + 1)
                }
                _ => Value::String(text),
            },
            value => value,
        }
    }
    project(value, 0)
}

impl KernelRuntimeState {
    fn authorize_public_history_room(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        requested: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        use crate::runtime::room_tool_admission::denied;
        self.authorize_current_external_command()?;
        let current_session = self
            .owned
            .session_store
            .get_session(session.id())
            .map_err(|_| denied("history requires an authorized live room agent"))?;
        let current_agent = self
            .owned
            .agent_store
            .get_agent(agent.id())
            .map_err(|_| denied("history requires an authorized live room agent"))?;
        let session = &current_session;
        let agent = &current_agent;

        if !self.room_agent_tools_enabled()
            || agent.session_id() != session.id()
            || agent.owner_user_id() != session.owner_user_id()
            || session.status() == SessionStatus::Ended
        {
            return Err(denied("history requires an authorized live room agent"));
        }
        if let Some((actor, run)) = &self.room_provider_origin {
            if actor != agent.id() {
                return Err(denied("history caller identity mismatch"));
            }
            self.authorize_room_provider_epoch(Some(actor), Some(run))?;
        }
        let target = requested.unwrap_or(session.id());
        if target == session.id() {
            return Ok(session.clone());
        }
        // Sudo requires an explicit exact session, one live admitted provider run,
        // and the SAME owner. No alias/global/missing scope expansion.
        let (_, run_id) = self
            .room_provider_origin
            .as_ref()
            .ok_or_else(|| denied("history is outside the caller room"))?;
        let run = self.owned.provider_store.get_run(run_id)?;
        let turn = self.sudo_for_auth_token(
            run.runtime_mcp_auth_token()
                .ok_or_else(|| denied("history is outside the caller room"))?,
        )?;
        if turn.agent_id != agent.id() || turn.owner_user_id != agent.owner_user_id() {
            return Err(denied("history is outside the caller room"));
        }
        let target = self
            .owned
            .session_store
            .get_session(target)
            .map_err(|_| denied("history is outside the caller room"))?;
        if target.owner_user_id() != turn.owner_user_id || target.status() == SessionStatus::Ended {
            return Err(denied("history is outside the caller room"));
        }
        Ok(target)
    }

    // Canonical state -> projection is the writer lock order. Revalidate outside
    // the projection mutex, then fence invalidation without taking canonical locks.
    fn finish_public_history_snapshot<'a>(
        &'a self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        room: &crate::session::RuntimeSession,
        requested: Option<&str>,
        revision: u64,
    ) -> Result<std::sync::MutexGuard<'a, ()>, DaemonError> {
        let current = self.authorize_public_history_room(session, agent, requested)?;
        let guard = self.owned.operational_history_store.lock_public_history()?;
        self.owned
            .room_secret_observations
            .require(room.id(), false)?;
        if current.id() != room.id()
            || current.owner_user_id() != room.owner_user_id()
            || self
                .owned
                .operational_history_store
                .public_history_revision_locked(room.id())?
                != revision
        {
            return Err(crate::runtime::room_tool_admission::denied(
                "Public history changed; retry the query",
            ));
        }
        Ok(guard)
    }

    pub(super) async fn search_room_history(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: HistorySearchArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let state = self.clone();
        let session = session.clone();
        let agent = agent.clone();
        tokio::task::spawn_blocking(move || {
            let room = state.authorize_public_history_room(
                &session,
                &agent,
                args.session_id.as_deref(),
            )?;
            let agent_id = if let Some(reference) = args.agent_ref.as_deref() {
                let agents = state.owned.agent_store.get_session_agents(room.id());
                Some(
                    crate::runtime::room_tool_admission::resolve_agent(
                        &agents,
                        room.id(),
                        reference,
                    )?
                    .id()
                    .to_owned(),
                )
            } else {
                None
            };
            let _guard = state
                .owned
                .operational_history_store
                .lock_public_history()?;
            state
                .owned
                .room_secret_observations
                .require(room.id(), false)?;
            let revision = state
                .owned
                .operational_history_store
                .public_history_revision_locked(room.id())?;
            let result = state
                .owned
                .operational_history_store
                .search_public_history_locked(
                    room.owner_user_id(),
                    room.id(),
                    agent_id.as_deref(),
                    &args.query,
                    args.limit.unwrap_or(20),
                    args.cursor.as_deref(),
                )?;
            drop(_guard);
            let _guard = state.finish_public_history_snapshot(
                &session,
                &agent,
                &room,
                args.session_id.as_deref(),
                revision,
            )?;
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::to_value(result).map_err(|_| {
                    crate::runtime::room_tool_admission::denied("invalid public history projection")
                })?,
            })
        })
        .await
        .map_err(|_| {
            crate::runtime::room_tool_admission::denied("history query did not complete")
        })?
    }

    pub(super) async fn read_room_history(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        event_ref: String,
        requested: Option<String>,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let state = self.clone();
        let session = session.clone();
        let agent = agent.clone();
        tokio::task::spawn_blocking(move || {
            let room =
                state.authorize_public_history_room(&session, &agent, requested.as_deref())?;
            let _guard = state
                .owned
                .operational_history_store
                .lock_public_history()?;
            state
                .owned
                .room_secret_observations
                .require(room.id(), false)?;
            let revision = state
                .owned
                .operational_history_store
                .public_history_revision_locked(room.id())?;
            let document = state
                .owned
                .operational_history_store
                .read_public_history_locked(room.owner_user_id(), room.id(), &event_ref)?;
            drop(_guard);
            let _guard = state.finish_public_history_snapshot(
                &session,
                &agent,
                &room,
                requested.as_deref(),
                revision,
            )?;
            Ok(RuntimeToolResult {
                ok: document.is_some(),
                payload: match document {
                    Some(document) => serde_json::json!({"event":document}),
                    None => serde_json::json!({"error":"Public history reference is unavailable"}),
                },
            })
        })
        .await
        .map_err(|_| crate::runtime::room_tool_admission::denied("history read did not complete"))?
    }

    pub(super) async fn room_public_turn(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        args: crate::transport::runtime_tools::MetaTurnOverviewArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let state = self.clone();
        let session = session.clone();
        let agent = agent.clone();
        tokio::task::spawn_blocking(move|| {
            let room=state.authorize_public_history_room(&session,&agent,None)?;
            let agents=state.owned.agent_store.get_session_agents(room.id());
            let target=crate::runtime::room_tool_admission::resolve_agent(&agents,room.id(),args.agent_ref.as_deref().unwrap_or(agent.id()))?;
            let _guard=state.owned.operational_history_store.lock_public_history()?;
            state.owned.room_secret_observations.require(room.id(),false)?;
            let revision=state.owned.operational_history_store.public_history_revision_locked(room.id())?;
            let mut events=state.owned.operational_history_store.public_history_turn_locked(room.owner_user_id(),room.id(),target.id(),200)?;
            drop(_guard);
            let _guard=state.finish_public_history_snapshot(&session,&agent,&room,None,revision)?;
            let mut turns=Vec::new();for event in &events {if !turns.contains(&event.turn_id) {turns.push(event.turn_id.clone());}}
            let turn=args.turn_ref.or_else(||turns.get(args.turns_back.unwrap_or(0)).cloned().flatten());
            events.retain(|event|event.turn_id==turn);events.truncate(args.limit.unwrap_or(200).clamp(1,200));events.reverse();
            Ok(RuntimeToolResult {ok:true,payload:serde_json::json!({"agent_id":target.id(),"events":events,"redaction_version":crate::history::public_history::PUBLIC_HISTORY_VERSION})})
        }).await.map_err(|_|crate::runtime::room_tool_admission::denied("history turn did not complete"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{HistoryEventRole, HistoryEventTurnContext, OperationalHistoryStore};
    #[test]
    fn public_history_owner_provenance_is_never_accepted_or_emitted_as_json() {
        let context: HistoryEventTurnContext = serde_json::from_value(
            serde_json::json!({"public_history_owner_user_id":"forged-owner"}),
        )
        .unwrap();
        assert!(context.public_history_owner_user_id.is_none());
        let event = HistoryEvent::operational(
            1,
            HistoryEventKind::UserPrompt,
            None,
            Some("public task".into()),
            Default::default(),
            HistoryEventTurnContext {
                public_history_owner_user_id: Some("trusted-owner".into()),
                ..Default::default()
            },
        );
        let mut wire = serde_json::to_value(event).unwrap();
        assert!(wire.get("public_history_owner_user_id").is_none());
        wire["public_history_owner_user_id"] = serde_json::json!("forged-owner");
        assert!(serde_json::from_value::<HistoryEvent>(wire)
            .unwrap()
            .public_history_owner_user_id
            .is_none());
    }

    #[test]
    fn public_history_projection_excludes_private_content_and_scrubs_echoes() {
        let root = std::env::temp_dir().join(format!(
            "chariox-am9-protection-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let protection = room_secret_observation::RoomSecretObservations::new(
            root.join("observation"),
            Default::default(),
        );
        let store = OperationalHistoryStore::open(root.join("history.sqlite")).unwrap();
        let projector = protection.clone();
        store.set_public_history_projector(Arc::new(move |event| {
            projector.public_history_document(event, "owner")
        }));
        let protection = protection.with_public_history(store.clone());
        let append = |kind, text: &str| {
            store
                .append_operational_event(
                    kind,
                    Some(HistoryEventRole::Assistant),
                    Some(text.into()),
                    Default::default(),
                    HistoryEventTurnContext {
                        session_id: Some("room".into()),
                        agent_id: Some("peer".into()),
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        let prior = append(HistoryEventKind::UserPrompt, "prior sensitive_canary");
        protection.register("room", "sensitive_canary").unwrap();
        append(
            HistoryEventKind::ProviderReasoning,
            "private_reasoning_canary",
        );
        append(
            HistoryEventKind::ProviderTool,
            r#"{"tool":"vault.read","output":"credential_canary"}"#,
        );
        append(
            HistoryEventKind::ProviderTool,
            r#"{"tool":"shell","arguments":{"password":"private_argument_canary","command":"cargo check"},"output":"safe_compiler_result sensitive_canary"}"#,
        );
        append(
            HistoryEventKind::UserPrompt,
            "public_prompt_marker sensitive_canary",
        );
        append(
            HistoryEventKind::ProviderTool,
            r#"{"tool":"shell","arguments":"{\"password\":\"nested_private_canary\",\"command\":\"cargo check\"}","output":"safe_nested_result"}"#,
        );

        append(
            HistoryEventKind::ProviderTool,
            r#"{"output":"{\"tool\":\"vault.read\",\"output\":\"nested_auth_canary\"}"}"#,
        );

        append(
            HistoryEventKind::ProviderTool,
            r#"{"tool":"bash","raw":"provider_raw_canary","input":{"api_key":"provider_api_canary"},"output":{"image_url":"image_body_canary","text":"safe_media_result"}}"#,
        );

        append(
            HistoryEventKind::ProviderTool,
            r#"{"tool":"browser","output":[{"type":"image","data":"typed_attachment_canary","mimeType":"image/png"},{"type":"text","text":"safe_capture_description"}]}"#,
        );

        let _guard = store.lock_public_history().unwrap();
        for query in [
            "sensitive_canary",
            "private_reasoning_canary",
            "credential_canary",
            "private_argument_canary",
            "nested_private_canary",
            "nested_auth_canary",
            "provider_raw_canary",
            "provider_api_canary",
            "image_body_canary",
            "typed_attachment_canary",
        ] {
            assert!(store
                .search_public_history_locked("owner", "room", None, query, 50, None)
                .unwrap()
                .hits
                .is_empty());
        }
        assert!(store
            .read_public_history_locked("owner", "room", &prior.event_id)
            .unwrap()
            .is_none());
        for query in ["safe_compiler_result", "public_prompt_marker"] {
            let result = store
                .search_public_history_locked("owner", "room", None, query, 50, None)
                .unwrap();
            assert_eq!(result.hits.len(), 1);
            let doc = store
                .read_public_history_locked("owner", "room", &result.hits[0].event_ref)
                .unwrap()
                .unwrap();
            assert!(!doc.text.contains("sensitive_canary"));
            assert!(!doc.text.contains("private_argument_canary"));
        }
        drop(_guard);
        drop(store);
        drop(protection);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn public_history_turn_selects_older_turn_before_event_limit() {
        std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                let root = std::env::temp_dir().join(format!("chariox-am9-turn-{:016x}", rand::random::<u64>()));
                std::fs::create_dir(&root).unwrap();
                let mut config = crate::config::DaemonConfig::for_tests().with_session_history_root(root.join("history"));
                config.room_agent_tools = true;
                let mut app = crate::DaemonApp::bootstrap(config).unwrap();
                let (room, actor) = crate::app::KernelSessionService::new(&mut app).create_session(
                    crate::session::CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                ).unwrap();
                let state = crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1).runtime_state();
                let append = |turn: &str, text: &str| state.owned.operational_history_store.append_operational_event(
                    HistoryEventKind::UserPrompt, Some(crate::history::HistoryEventRole::User), Some(text.into()),
                    Default::default(), crate::history::HistoryEventTurnContext {
                        session_id: Some(room.id().into()), agent_id: Some(actor.id().into()),
                        turn_id: Some(turn.into()), public_history_owner_user_id: Some(room.owner_user_id().into()),
                        ..Default::default()
                    }
                ).unwrap();
                let first = append("previous", "previous first");
                let second = append("previous", "previous second");
                for index in 0..225 { append("newest", &format!("newest {index}")); }
                for args in [serde_json::json!({"turns_back":1}), serde_json::json!({"turn_ref":"previous"})] {
                    let result = state.dispatch_meta_runtime_tool_call_for_agent(room.id(), actor.id(), "chariox.history.turn", args).await.unwrap();
                    assert!(result.ok);
                    let events = result.payload["events"].as_array().unwrap();
                    assert_eq!(events.len(), 2, "newest turn must not hide the requested retained turn");
                    assert_eq!(events[0]["event_ref"], first.event_id);
                    assert_eq!(events[1]["event_ref"], second.event_id);
                }
                let bounded = state.dispatch_meta_runtime_tool_call_for_agent(room.id(), actor.id(), "chariox.history.turn",
                    serde_json::json!({"turn_ref":"previous", "limit":1})).await.unwrap();
                assert_eq!(bounded.payload["events"].as_array().unwrap().len(), 1);
                assert_eq!(bounded.payload["events"][0]["event_ref"], second.event_id);
                let newest = state.dispatch_meta_runtime_tool_call_for_agent(room.id(), actor.id(), "chariox.history.turn",
                    serde_json::json!({"turns_back":0})).await.unwrap();
                assert_eq!(newest.payload["events"].as_array().unwrap().len(), 200);
                let missing = state.dispatch_meta_runtime_tool_call_for_agent(room.id(), actor.id(), "chariox.history.turn",
                    serde_json::json!({"turn_ref":"missing"})).await.unwrap();
                assert!(missing.payload["events"].as_array().unwrap().is_empty());
                drop(state);
                std::fs::remove_dir_all(root).unwrap();
            })
        }).unwrap().join().unwrap();
    }

    #[test]
    fn public_history_runtime_denies_foreign_scope_refs_and_stale_runs() {
        std::thread::Builder::new().stack_size(64*1024*1024).spawn(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                let root=std::env::temp_dir().join(format!("chariox-am9-admission-{:016x}",rand::random::<u64>()));
                std::fs::create_dir(&root).unwrap();
                let mut config=crate::config::DaemonConfig::for_tests().with_session_history_root(root.join("history"));
                config.room_agent_tools=true;
                let mut app=crate::DaemonApp::bootstrap(config).unwrap();
                let (room,actor)=crate::app::KernelSessionService::new(&mut app).create_session(crate::session::CreateSessionRequest::new(root.to_string_lossy(),root.to_string_lossy())).unwrap();
                let peer=crate::app::KernelSessionService::new(&mut app).spawn_agent(crate::agent::CreateAgentRequest::new(room.id(),"dev-stub")).unwrap();
                let (foreign,foreign_actor)=crate::app::KernelSessionService::new(&mut app).create_session(crate::session::CreateSessionRequest::new(root.to_string_lossy(),root.to_string_lossy())).unwrap();
                let state=crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)),1).runtime_state();
                let append=|session:&str,agent:&str,text:&str|state.owned.operational_history_store.append_operational_event(HistoryEventKind::UserPrompt,Some(crate::history::HistoryEventRole::User),Some(text.into()),Default::default(),crate::history::HistoryEventTurnContext {session_id:Some(session.into()),agent_id:Some(agent.into()),public_history_owner_user_id:state.owned.session_store.get_session(session).ok().map(|s|s.owner_user_id().to_owned()),..Default::default()}).unwrap();
                // No canonical session lock may be acquired by the projector.
                let locked_sessions = state.owned.session_store.write();
                let history = state.owned.operational_history_store.clone();
                let session_id = room.id().to_owned();
                let agent_id = actor.id().to_owned();
                let owner = room.owner_user_id().to_owned();
                let (sent, received) = std::sync::mpsc::channel();
                let writer = std::thread::spawn(move || {
                    let result = history.append_operational_event(HistoryEventKind::UserPrompt,Some(crate::history::HistoryEventRole::User),Some("writer_lock_public_task".into()),Default::default(),crate::history::HistoryEventTurnContext {session_id:Some(session_id),agent_id:Some(agent_id),public_history_owner_user_id:Some(owner),..Default::default()});
                    sent.send(result.is_ok()).unwrap();
                });
                let completed_while_locked = received.recv_timeout(Duration::from_secs(1)).unwrap_or(false);
                drop(locked_sessions);
                writer.join().unwrap();
                assert!(completed_while_locked,"public history producer inverted the session/history lock order");
                append(room.id(),peer.id(),"authorized_peer_task");
                let foreign_event=append(foreign.id(),foreign_actor.id(),"foreign_private_marker");
                let invoke=|args|state.dispatch_meta_runtime_tool_call_for_agent(room.id(),actor.id(),"chariox.history.search",args);
                let result=invoke(serde_json::json!({"query":"authorized_peer_task"})).await.unwrap();assert!(result.ok);assert_eq!(result.payload["hits"].as_array().unwrap().len(),1);
                assert!(invoke(serde_json::json!({"query":"foreign_private_marker","session_id":foreign.id()})).await.is_err());
                assert!(invoke(serde_json::json!({"query":"foreign_private_marker","owner_user_id":foreign_actor.owner_user_id()})).await.is_err());
                let result=invoke(serde_json::json!({"query":"foreign_private_marker"})).await.unwrap();assert!(result.payload["hits"].as_array().unwrap().is_empty());
                let foreign_read=state.read_room_history(&room,&actor,foreign_event.event_id,None).await.unwrap();
                let guessed_read=state.read_room_history(&room,&actor,"evt_guessed".into(),None).await.unwrap();
                assert!(!foreign_read.ok);assert_eq!(foreign_read.payload,guessed_read.payload);
                let stale=state.with_room_provider_origin(Some(actor.id()),Some("retired-provider-run"));
                assert!(stale.search_room_history(&room,&actor,serde_json::from_value(serde_json::json!({"query":"authorized_peer_task"})).unwrap()).await.is_err());
                drop(stale);drop(state);let _=std::fs::remove_dir_all(root);
            })
        }).unwrap().join().unwrap();
    }
}
