use std::collections::BTreeMap;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crate::error::DaemonError;
use crate::provider::opencode_client::OpenCodePart;
use crate::session::SessionService;

use super::{
    LaunchProviderRequest, OpenCodeClient, OpenCodeEvent, OpenCodeEventSubscription,
    OpenCodeMessage, ProviderRegistry, ProviderRunState, RuntimeProviderRun,
};

#[derive(Debug)]
pub struct ProviderProcessService {
    registry: ProviderRegistry,
    opencode_runs: BTreeMap<String, OpenCodeRunState>,
    runs: BTreeMap<String, RuntimeProviderRun>,
    next_run_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodePollResult {
    pub text_deltas: Vec<Vec<u8>>,
    pub reasoning_deltas: Vec<Vec<u8>>,
    pub tool_updates: Vec<Vec<u8>>,
    pub status_updates: Vec<Vec<u8>>,
    pub prompt_completed: bool,
    pub provider_idle: bool,
    pub notices: Vec<String>,
}

struct OpenCodeEventDrainResult {
    text_deltas: Vec<Vec<u8>>,
    reasoning_deltas: Vec<Vec<u8>>,
    tool_updates: Vec<Vec<u8>>,
    status_updates: Vec<Vec<u8>>,
    prompt_completed: bool,
    provider_idle: bool,
    notices: Vec<String>,
}

#[derive(Debug)]
struct OpenCodeRunState {
    base_url: String,
    session_id: String,
    emitted_text_offsets: BTreeMap<String, usize>,
    emitted_tool_summaries: BTreeMap<String, String>,
    buffered_text_deltas: BTreeMap<String, Vec<String>>,
    message_roles: BTreeMap<String, String>,
    part_kinds: BTreeMap<String, String>,
    part_message_ids: BTreeMap<String, String>,
    event_subscription: OpenCodeEventSubscription,
    last_status_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ToolTranscriptUpdate {
    id: String,
    tool: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
}

impl ProviderProcessService {
    pub fn new() -> Self {
        Self {
            registry: ProviderRegistry::new(),
            opencode_runs: BTreeMap::new(),
            runs: BTreeMap::new(),
            next_run_number: 0,
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn launch_run(
        &mut self,
        sessions: &mut SessionService,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(&request.session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            let active_run = self.get_run(active_run_id)?;
            if active_run.state() == ProviderRunState::Ended {
                sessions.set_active_provider_run(&request.session_id, None)?;
                self.clear_runtime(active_run_id);
            } else {
                self.park_run(sessions, &request.session_id, active_run_id)?;
            }
        }

        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;

        let run_id = self.next_run_id();
        let launch_result = adapter.launch(&request)?;
        let mut run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);
        run.mark_running();

        self.runs.insert(run_id.clone(), run.clone());
        sessions.set_active_provider_run(&request.session_id, Some(run_id))?;

        Ok(run)
    }

    pub fn park_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let session = sessions.get_session(session_id)?;

        if session.active_provider_run_id() != Some(run_id) {
            return Err(DaemonError::InconsistentActiveProviderRun {
                session_id: session_id.to_string(),
                active_provider_run_id: session.active_provider_run_id().map(str::to_owned),
                requested_provider_run_id: run_id.to_string(),
            });
        }

        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "park",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.park(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_parked();
        sessions.set_active_provider_run(session_id, None)?;

        Ok(run.clone())
    }

    pub fn resume_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                self.park_run(sessions, session_id, active_run_id)?;
            }
        }

        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Parked {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "resume",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.resume(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_running();
        sessions.set_active_provider_run(session_id, Some(run_id.to_string()))?;

        Ok(run.clone())
    }

    pub fn terminate_run(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "terminate",
            });
        }

        let _ = self.abort_structured_runtime(run_id);
        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.terminate(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        if active_run_id.as_deref() == Some(run_id) {
            sessions.set_active_provider_run(session_id, None)?;
        }
        self.clear_runtime(run_id);

        Ok(run)
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    pub fn terminate_session_runs(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
    ) -> Result<Vec<RuntimeProviderRun>, DaemonError> {
        let run_ids: Vec<String> = self
            .runs
            .values()
            .filter(|run| run.session_id() == session_id && run.state() != ProviderRunState::Ended)
            .map(|run| run.id().to_string())
            .collect();

        let mut terminated_runs = Vec::with_capacity(run_ids.len());

        for run_id in run_ids {
            terminated_runs.push(self.terminate_run(sessions, session_id, &run_id)?);
        }

        Ok(terminated_runs)
    }

    pub fn initialize_runtime(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if run.adapter_key() != "opencode" {
            return Ok(());
        }

        let base_url = run
            .structured_endpoint()
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "opencode_endpoint_missing",
                message: "opencode run did not expose a structured endpoint".to_string(),
            })?
            .to_string();
        let client = OpenCodeClient::new(run.id(), &base_url)?;
        crate::logging::info_with_fields(
            "daemon.provider.opencode",
            "waiting for opencode health",
            serde_json::json!({
                "provider_run_id": run.id(),
                "base_url": base_url.clone(),
            }),
        );
        client.wait_until_healthy(Duration::from_secs(30))?;
        crate::logging::info_with_fields(
            "daemon.provider.opencode",
            "opencode became healthy",
            serde_json::json!({
                "provider_run_id": run.id(),
                "base_url": base_url.clone(),
            }),
        );
        let session_id = client.create_session()?;
        crate::logging::info_with_fields(
            "daemon.provider.opencode",
            "created opencode session",
            serde_json::json!({
                "provider_run_id": run.id(),
                "provider_session_id": session_id.clone(),
            }),
        );
        let event_subscription = client.subscribe_events()?;
        crate::logging::info_with_fields(
            "daemon.provider.opencode",
            "subscribed to opencode events",
            serde_json::json!({
                "provider_run_id": run.id(),
            }),
        );
        self.opencode_runs.insert(
            run.id().to_string(),
            OpenCodeRunState {
                base_url,
                session_id,
                emitted_text_offsets: BTreeMap::new(),
                emitted_tool_summaries: BTreeMap::new(),
                buffered_text_deltas: BTreeMap::new(),
                message_roles: BTreeMap::new(),
                part_kinds: BTreeMap::new(),
                part_message_ids: BTreeMap::new(),
                event_subscription,
                last_status_kind: None,
            },
        );
        Ok(())
    }

    pub fn clear_runtime(&mut self, provider_run_id: &str) {
        if let Some(state) = self.opencode_runs.remove(provider_run_id) {
            state.event_subscription.stop();
        }
    }

    pub fn abort_structured_runtime(&self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() != "opencode" {
            return Ok(false);
        }

        let state = self.opencode_runs.get(provider_run_id).ok_or_else(|| {
            DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "opencode_session_missing",
                message: "no OpenCode session is bound to this provider run".to_string(),
            }
        })?;
        let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
        client.abort_session(&state.session_id)?;
        Ok(true)
    }

    pub fn submit_structured_prompt(
        &self,
        run: &RuntimeProviderRun,
        prompt: &str,
    ) -> Result<bool, DaemonError> {
        if run.adapter_key() != "opencode" {
            return Ok(false);
        }

        let state =
            self.opencode_runs
                .get(run.id())
                .ok_or_else(|| DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "opencode_session_missing",
                    message: "no OpenCode session is bound to this provider run".to_string(),
                })?;
        let client = OpenCodeClient::new(run.id(), &state.base_url)?;
        client.submit_prompt(&state.session_id, prompt, Some(run.model()))?;
        Ok(true)
    }

    pub fn poll_structured_output(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<OpenCodePollResult>, DaemonError> {
        let run = self.get_run(provider_run_id)?;
        if run.adapter_key() != "opencode" {
            return Ok(None);
        }

        let drain = self.drain_opencode_events(provider_run_id)?;

        Ok(Some(OpenCodePollResult {
            text_deltas: drain.text_deltas,
            reasoning_deltas: drain.reasoning_deltas,
            tool_updates: drain.tool_updates,
            status_updates: drain.status_updates,
            prompt_completed: drain.prompt_completed,
            provider_idle: drain.provider_idle,
            notices: drain.notices,
        }))
    }

    pub fn mark_run_ended(
        &mut self,
        sessions: &mut SessionService,
        session_id: &str,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let active_run_id = sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            return Ok(run_snapshot);
        }

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        if active_run_id.as_deref() == Some(run_id) {
            sessions.set_active_provider_run(session_id, None)?;
        }
        self.clear_runtime(run_id);

        Ok(run)
    }

    fn get_run_mut(&mut self, run_id: &str) -> Result<&mut RuntimeProviderRun, DaemonError> {
        self.runs
            .get_mut(run_id)
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    fn adapter_for(
        &self,
        adapter_key: &str,
    ) -> Result<&'static dyn super::ProviderAdapter, DaemonError> {
        self.registry
            .resolve(adapter_key)
            .ok_or_else(|| DaemonError::ProviderAdapterNotFound {
                adapter_key: adapter_key.to_string(),
            })
    }

    fn next_run_id(&mut self) -> String {
        self.next_run_number += 1;
        format!("provider-run-{}", self.next_run_number)
    }

    fn drain_opencode_events(
        &mut self,
        provider_run_id: &str,
    ) -> Result<OpenCodeEventDrainResult, DaemonError> {
        let state = self.opencode_runs.get_mut(provider_run_id).ok_or_else(|| {
            DaemonError::ProviderProtocol {
                provider_run_id: provider_run_id.to_string(),
                operation: "opencode_session_missing",
                message: "no OpenCode session is bound to this provider run".to_string(),
            }
        })?;

        let mut deltas = Vec::new();
        let mut reasoning_deltas = Vec::new();
        let mut tool_updates = Vec::new();
        let mut status_updates = Vec::new();
        let mut prompt_completed = false;
        let mut provider_idle = false;
        let mut notices = Vec::new();

        loop {
            match state.event_subscription.receiver.try_recv() {
                Ok(OpenCodeEvent::MessageUpdated { info }) => {
                    state
                        .message_roles
                        .insert(info.id.clone(), info.role.clone());
                    if info.session_id == state.session_id
                        && info.role == "assistant"
                        && info.time.completed.is_some()
                    {
                        prompt_completed = true;
                    }
                }
                Ok(OpenCodeEvent::MessagePartDelta {
                    session_id,
                    message_id,
                    part_id,
                    field,
                    delta,
                    ..
                }) => {
                    if session_id != state.session_id || field != "text" || delta.is_empty() {
                        continue;
                    }
                    state
                        .part_message_ids
                        .insert(part_id.clone(), message_id.clone());
                    if !state.message_roles.contains_key(&message_id) {
                        refresh_opencode_message_metadata(state, provider_run_id)?;
                    }
                    let Some(role) = state.message_roles.get(&message_id).map(String::as_str)
                    else {
                        state
                            .buffered_text_deltas
                            .entry(part_id)
                            .or_default()
                            .push(delta);
                        continue;
                    };
                    if role != "assistant" {
                        continue;
                    }
                    match state.part_kinds.get(&part_id).map(String::as_str) {
                        Some("reasoning") => {
                            let emitted = state.emitted_text_offsets.entry(part_id).or_insert(0);
                            *emitted += delta.len();
                            reasoning_deltas.push(delta.into_bytes());
                        }
                        Some("text") => {
                            let emitted = state.emitted_text_offsets.entry(part_id).or_insert(0);
                            *emitted += delta.len();
                            deltas.push(delta.into_bytes());
                        }
                        Some(_) => {}
                        None => {
                            state
                                .buffered_text_deltas
                                .entry(part_id)
                                .or_default()
                                .push(delta);
                        }
                    }
                }
                Ok(OpenCodeEvent::MessagePartUpdated { part }) => {
                    if part.session_id != state.session_id {
                        continue;
                    }
                    state
                        .part_message_ids
                        .insert(part.id.clone(), part.message_id.clone());
                    state.part_kinds.insert(part.id.clone(), part.kind.clone());
                    if !state.message_roles.contains_key(&part.message_id) {
                        refresh_opencode_message_metadata(state, provider_run_id)?;
                    }
                    let role = state
                        .message_roles
                        .get(&part.message_id)
                        .map(String::as_str);
                    if let Some(buffered_deltas) = state.buffered_text_deltas.remove(&part.id) {
                        for delta in buffered_deltas {
                            if role != Some("assistant") {
                                continue;
                            }
                            let emitted = state
                                .emitted_text_offsets
                                .entry(part.id.clone())
                                .or_insert(0);
                            *emitted += delta.len();
                            match part.kind.as_str() {
                                "reasoning" => reasoning_deltas.push(delta.into_bytes()),
                                "text" => deltas.push(delta.into_bytes()),
                                _ => {}
                            }
                        }
                    }
                    match part.kind.as_str() {
                        "text" => {
                            if role != Some("assistant") || part.text.is_empty() {
                                continue;
                            }
                            let emitted = state
                                .emitted_text_offsets
                                .entry(part.id.clone())
                                .or_insert(0);
                            let start = (*emitted).min(part.text.len());
                            if start == part.text.len() {
                                continue;
                            }
                            deltas.push(part.text.as_bytes()[start..].to_vec());
                            *emitted = part.text.len();
                        }
                        "reasoning" => {
                            if role != Some("assistant") || part.text.is_empty() {
                                continue;
                            }
                            let emitted = state
                                .emitted_text_offsets
                                .entry(part.id.clone())
                                .or_insert(0);
                            let start = (*emitted).min(part.text.len());
                            if start == part.text.len() {
                                continue;
                            }
                            reasoning_deltas.push(part.text.as_bytes()[start..].to_vec());
                            *emitted = part.text.len();
                        }
                        "tool" => {
                            if role != Some("assistant") {
                                continue;
                            }
                            let summary = render_tool_transcript_update(&part);
                            let previous = state.emitted_tool_summaries.get(&part.id);
                            if previous.map(String::as_str) != Some(summary.as_str()) {
                                state
                                    .emitted_tool_summaries
                                    .insert(part.id.clone(), summary.clone());
                                tool_updates.push(summary.into_bytes());
                            }
                        }
                        _ => {}
                    }
                }
                Ok(OpenCodeEvent::SessionError {
                    session_id,
                    message,
                }) => {
                    if session_id == state.session_id {
                        notices.push(message);
                        prompt_completed = true;
                    }
                }
                Ok(OpenCodeEvent::SessionStatus { session_id, kind }) => {
                    if session_id == state.session_id {
                        if kind == "idle" {
                            provider_idle = true;
                        }
                        if state.last_status_kind.as_deref() != Some(kind.as_str()) {
                            state.last_status_kind = Some(kind.clone());
                            if kind != "idle" {
                                status_updates.push(format_session_status(&kind).into_bytes());
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                    state.event_subscription = client.subscribe_events()?;
                    if let Ok(snapshot) = client.snapshot(&state.session_id) {
                        record_snapshot_message_metadata(state, &snapshot.messages);
                        let snapshot_deltas =
                            render_snapshot_text_deltas(state, &snapshot.messages);
                        deltas.extend(snapshot_deltas.text_deltas);
                        reasoning_deltas.extend(snapshot_deltas.reasoning_deltas);
                        tool_updates.extend(snapshot_deltas.tool_updates);
                        if snapshot.status == "idle" {
                            provider_idle = true;
                        }
                        if snapshot.messages.iter().any(|message| {
                            message.info.session_id == state.session_id
                                && message.info.role == "assistant"
                                && message.info.time.completed.is_some()
                        }) {
                            prompt_completed = true;
                        }
                    }
                }
            }
        }

        Ok(OpenCodeEventDrainResult {
            text_deltas: deltas,
            reasoning_deltas,
            tool_updates,
            status_updates,
            prompt_completed,
            provider_idle,
            notices,
        })
    }
}

struct SnapshotRenderResult {
    text_deltas: Vec<Vec<u8>>,
    reasoning_deltas: Vec<Vec<u8>>,
    tool_updates: Vec<Vec<u8>>,
}

fn refresh_opencode_message_metadata(
    state: &mut OpenCodeRunState,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
    if let Ok(messages) = client.messages(&state.session_id) {
        record_snapshot_message_metadata(state, &messages);
    }
    Ok(())
}

fn record_snapshot_message_metadata(state: &mut OpenCodeRunState, messages: &[OpenCodeMessage]) {
    for message in messages {
        state
            .message_roles
            .insert(message.info.id.clone(), message.info.role.clone());
        for part in &message.parts {
            state
                .part_message_ids
                .insert(part.id.clone(), part.message_id.clone());
            state.part_kinds.insert(part.id.clone(), part.kind.clone());
        }
    }
}

fn render_snapshot_text_deltas(
    state: &mut OpenCodeRunState,
    messages: &[OpenCodeMessage],
) -> SnapshotRenderResult {
    let mut text_deltas = Vec::new();
    let mut reasoning_deltas = Vec::new();
    let mut tool_updates = Vec::new();
    for message in messages
        .iter()
        .filter(|message| message.info.role == "assistant")
    {
        for part in &message.parts {
            match part.kind.as_str() {
                "text" | "reasoning" => {
                    if part.text.is_empty() {
                        continue;
                    }
                    let emitted = state
                        .emitted_text_offsets
                        .entry(part.id.clone())
                        .or_insert(0);
                    let start = (*emitted).min(part.text.len());
                    if start == part.text.len() {
                        continue;
                    }
                    let bytes = part.text.as_bytes()[start..].to_vec();
                    if part.kind == "reasoning" {
                        reasoning_deltas.push(bytes);
                    } else {
                        text_deltas.push(bytes);
                    }
                    *emitted = part.text.len();
                }
                "tool" => {
                    let summary = render_tool_transcript_update(part);
                    let previous = state.emitted_tool_summaries.get(&part.id);
                    if previous.map(String::as_str) != Some(summary.as_str()) {
                        state
                            .emitted_tool_summaries
                            .insert(part.id.clone(), summary.clone());
                        tool_updates.push(summary.into_bytes());
                    }
                }
                _ => {}
            }
        }
    }
    SnapshotRenderResult {
        text_deltas,
        reasoning_deltas,
        tool_updates,
    }
}

fn render_tool_transcript_update(part: &OpenCodePart) -> String {
    let tool_name = if part.tool.is_empty() {
        "tool"
    } else {
        part.tool.as_str()
    };
    let status = part
        .state
        .as_ref()
        .map(|state| state.status.as_str())
        .filter(|status: &&str| !status.is_empty())
        .unwrap_or("updated");
    let rendered_text = (!part.text.trim().is_empty()).then(|| part.text.trim().to_string());
    let input = part.state.as_ref().and_then(|state| {
        (!state.input.is_null() && !is_empty_json_value(&state.input)).then(|| state.input.clone())
    });
    let output = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.output.as_str()).map(str::to_string))
        .or_else(|| tool_metadata_field(part, &["output", "stdout"]));
    let description = tool_metadata_field(part, &["description"]);
    let title = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.title.as_str()).map(str::to_string));
    let error = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.error.as_str()).map(str::to_string));
    let raw = part
        .state
        .as_ref()
        .and_then(|state| non_empty(state.raw.as_str()))
        .map(render_tool_raw_detail)
        .filter(|value| {
            rendered_text.as_deref() != Some(value.as_str())
                && output.as_deref() != Some(value.as_str())
        });

    serde_json::to_string(&ToolTranscriptUpdate {
        id: part.id.clone(),
        tool: tool_name.to_string(),
        status: status.to_string(),
        title,
        description,
        text: rendered_text,
        input,
        output,
        error,
        raw,
    })
    .unwrap_or_else(|_| {
        format!(
            "{{\"id\":{id:?},\"tool\":{tool:?},\"status\":{status:?}}}",
            id = part.id,
            tool = tool_name,
            status = status,
        )
    })
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn is_empty_json_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Array(items) => items.is_empty(),
        serde_json::Value::Object(items) => items.is_empty(),
        _ => false,
    }
}

fn tool_metadata_field(part: &OpenCodePart, keys: &[&str]) -> Option<String> {
    let metadata = part.state.as_ref()?.metadata.as_object()?;
    keys.iter().find_map(|key| {
        metadata
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(non_empty)
            .map(str::to_string)
    })
}

fn render_tool_raw_detail(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    }
}

fn format_session_status(kind: &str) -> String {
    match kind {
        "busy" => "OpenCode is thinking...".to_string(),
        "idle" => "OpenCode is idle.".to_string(),
        other => format!("OpenCode status: {other}"),
    }
}

impl Default for ProviderProcessService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::config::DaemonConfig;
    use crate::provider::opencode_client::{OpenCodePart, OpenCodeToolState};
    use crate::session::{CreateSessionRequest, SessionService, SessionStatus};

    use super::{
        render_tool_transcript_update, LaunchProviderRequest, ProviderProcessService,
        ProviderRunState, ToolTranscriptUpdate,
    };

    fn sessions() -> SessionService {
        SessionService::new(&DaemonConfig::for_tests())
    }

    fn launch_request(session_id: &str, model: &str) -> LaunchProviderRequest {
        LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
    }

    #[test]
    fn launches_the_first_provider_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let run = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("provider run should launch");
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");

        assert_eq!(run.id(), "provider-run-1");
        assert_eq!(run.state(), ProviderRunState::Running);
        assert_eq!(run.adapter_key(), "dev-stub");
        assert_eq!(session.active_provider_run_id(), Some(run.id()));
        assert_eq!(session.status(), SessionStatus::Active);
    }

    #[test]
    fn parks_existing_run_when_new_run_becomes_active() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("first run should launch");
        let second = providers
            .launch_run(&mut sessions, launch_request(session.id(), "opus"))
            .expect("second run should launch");

        let first = providers
            .get_run(first.id())
            .expect("first run should still exist");
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");

        assert_eq!(first.state(), ProviderRunState::Parked);
        assert_eq!(second.state(), ProviderRunState::Running);
        assert_eq!(session.active_provider_run_id(), Some(second.id()));
    }

    #[test]
    fn rejects_inconsistent_active_run_state() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        sessions
            .set_active_provider_run(session.id(), Some("missing-run".to_string()))
            .expect("session active run can be set for this invariant test");

        let error = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect_err("launch should reject inconsistent active run state");

        match error {
            crate::DaemonError::ProviderRunNotFound { provider_run_id } => {
                assert_eq!(provider_run_id, "missing-run");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn launches_new_run_when_session_points_at_ended_active_run() {
        let mut sessions = sessions();
        let session = sessions
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let mut providers = ProviderProcessService::new();

        let first = providers
            .launch_run(&mut sessions, launch_request(session.id(), "sonnet"))
            .expect("first run should launch");
        providers
            .get_run_mut(first.id())
            .expect("first run should exist")
            .mark_ended();

        let second = providers
            .launch_run(&mut sessions, launch_request(session.id(), "opus"))
            .expect("second run should launch even if active run is stale and ended");
        let session = sessions
            .get_session(session.id())
            .expect("session should exist");
        let first = providers
            .get_run(first.id())
            .expect("first run should still exist");

        assert_eq!(first.state(), ProviderRunState::Ended);
        assert_eq!(second.state(), ProviderRunState::Running);
        assert_eq!(session.active_provider_run_id(), Some(second.id()));
    }

    #[test]
    fn renders_structured_tool_update_with_input_and_output() {
        let payload = render_tool_transcript_update(&OpenCodePart {
            id: "part-1".to_string(),
            session_id: "session-1".to_string(),
            message_id: "message-1".to_string(),
            kind: "tool".to_string(),
            text: String::new(),
            tool: "bash".to_string(),
            state: Some(OpenCodeToolState {
                status: "completed".to_string(),
                input: json!({ "command": "git status" }),
                output: String::new(),
                title: String::new(),
                metadata: json!({
                    "output": "On branch main",
                    "description": "Shows working tree status"
                }),
                error: String::new(),
                raw: String::new(),
            }),
            time: None,
        });

        let parsed: ToolTranscriptUpdate =
            serde_json::from_str(&payload).expect("tool payload should deserialize");
        assert_eq!(parsed.id, "part-1");
        assert_eq!(parsed.tool, "bash");
        assert_eq!(parsed.status, "completed");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Shows working tree status")
        );
        assert_eq!(parsed.output.as_deref(), Some("On branch main"));
        assert_eq!(parsed.input, Some(json!({ "command": "git status" })));
    }
}
