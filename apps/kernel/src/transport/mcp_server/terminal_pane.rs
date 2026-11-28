//! Claude Code's own terminal as an agent pane (docs/PROTOCOL.md section 3.3.2.1). The run's
//! MCP notification stream reads the kernel's pane feed and translates it: the status line
//! shows it, a card opens as Claude's own MCP elicitation dialog, and a model or mode switch
//! made in the terminal is reported with `ReportPaneSettings`. Every answer is the same
//! `RespondToInteraction` any client sends, so one answer closes the card everywhere.
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, Weak};
use std::time::Duration;

use tokio::time::Instant;

use super::*;
use crate::local::{
    GetPaneFeedRequest, LocalDaemonRequest, LocalDaemonResponse, PaneCard, PaneFeed, PaneState,
    ReportPaneSettingsRequest, RespondToInteractionRequest,
};
use crate::provider::{
    is_claude_native_permission, AgentExecutionMode, AgentPermissionLevel, RuntimeProviderRun,
    CLAUDE_PANE_STATUS_FILE_ENV, CLAUDE_TERMINAL_MODEL_FILE_ENV, CLAUDE_TERMINAL_MODE_FILE_ENV,
};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};

/// How often the terminal's own settings are read; a switch shows everywhere within it.
const SETTINGS_TICK: Duration = Duration::from_secs(1);
/// Stripes carry times the Room does not signal; the feed is read again at least this often.
const FEED_REFRESH: Duration = Duration::from_secs(5);
const REQUEST_ID_PREFIX: &str = "chariox-card-";

/// The dialogs open in provider terminals, by JSON-RPC request id. The terminal's answer
/// arrives as a separate POST; whoever takes the entry closes the dialog.
static DIALOGS: LazyLock<Mutex<HashMap<String, Dialog>>> = LazyLock::new(Default::default);

// A reconnect belongs to the same provider run: it keeps the open dialog, dismissals and
// observed settings. Discard ended runs whenever another terminal connects or answers.
static RUN_PANES: LazyLock<Mutex<HashMap<String, RunPane>>> = LazyLock::new(Default::default);

struct RunPane {
    router: Weak<CommandRouter>,
    token: String,
    state: Arc<Mutex<RunPaneState>>,
}

#[derive(Default)]
struct RunPaneState {
    /// The provider run survives reconnecting its notification stream.
    closed_here: BTreeSet<String>,
    open: Option<(String, String)>,
    model_seen: Option<TerminalModel>,
}

fn run_state(router: &Arc<CommandRouter>, token: &str, run_id: &str) -> Arc<Mutex<RunPaneState>> {
    let mut panes = RUN_PANES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    panes.retain(|id, pane| {
        let running = pane.router.upgrade().is_some_and(|router| {
            router
                .runtime_mcp_catalog_run(&pane.token)
                .is_some_and(|run| run.id() == id)
        });
        if !running {
            dialogs().retain(|_, dialog| &dialog.run_id != id);
        }
        running
    });
    panes
        .entry(run_id.to_string())
        .or_insert_with(|| RunPane {
            router: Arc::downgrade(router),
            token: token.to_string(),
            state: Arc::default(),
        })
        .state
        .clone()
}

#[derive(Debug, Clone, PartialEq)]
struct Dialog {
    run_id: String,
    session_id: String,
    owner_user_id: String,
    card_id: String,
    answers: DialogAnswers,
}

/// What an answer from the dialog may say.
#[derive(Debug, Clone, PartialEq)]
enum DialogAnswers {
    /// A form: one of these choices, or a typed reply for the reply choice.
    Form {
        choices: Vec<String>,
        reply_choice: Option<String>,
    },
    /// A link to the same pane: the card is answered there.
    Link,
}

fn dialogs() -> std::sync::MutexGuard<'static, HashMap<String, Dialog>> {
    DIALOGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The run behind a token when a provider's own terminal runs Claude with a pane.
fn terminal_run(router: &CommandRouter, token: &str) -> Option<(RuntimeProviderRun, PaneFiles)> {
    let run = router.runtime_mcp_catalog_run(token)?;
    if run.client_interface().is_chariox() {
        return None;
    }
    let files = PaneFiles::of(&run)?;
    Some((run, files))
}

struct PaneFiles {
    status: PathBuf,
    model: PathBuf,
    mode: PathBuf,
}

impl PaneFiles {
    fn of(run: &RuntimeProviderRun) -> Option<Self> {
        let path = |name: &str| run.pty_env().get(name).map(PathBuf::from);
        Some(Self {
            status: path(CLAUDE_PANE_STATUS_FILE_ENV)?,
            model: path(CLAUDE_TERMINAL_MODEL_FILE_ENV)?,
            mode: path(CLAUDE_TERMINAL_MODE_FILE_ENV)?,
        })
    }
}

/// The pane's frames on the run's notification stream, or `None` when no provider terminal
/// with a pane runs behind this token.
pub(super) fn stream(
    router: Arc<CommandRouter>,
    headers: &hyper::HeaderMap,
    shutdown: tokio::sync::watch::Receiver<()>,
) -> Option<impl futures_util::Stream<Item = Result<hyper::body::Frame<Bytes>, Infallible>>> {
    let token = parse_bearer_token(headers)?;
    let (run, files) = terminal_run(&router, &token)?;
    let agent_id = run.agent_instance_id()?.to_string();
    let state = run_state(&router, &token, run.id());
    let pane = Pane {
        sessions_seen: router.session_projection_change_sequence(),
        inbox_seen: router.person_inbox_change_sequence(),
        router: Arc::downgrade(&router),
        token,
        run_id: run.id().to_string(),
        session_id: run.session_id().to_string(),
        owner_user_id: run.owner_user_id().to_string(),
        agent_id,
        files,
        shutdown,
        state,
        status: None,
        feed: None,
        feed_due: Instant::now(),
    };
    Some(futures_util::stream::unfold(pane, |mut pane| async move {
        loop {
            let router = match pane.router.upgrade() {
                Some(router) if pane.bound(&router) => router,
                _ => {
                    pane.close();
                    return None;
                }
            };
            let frames = pane.tick(&router).await;
            if !frames.is_empty() {
                let frame = hyper::body::Frame::data(Bytes::from(frames));
                return Some((Ok(frame), pane));
            }
            let (sessions, inbox) = (pane.sessions_seen, pane.inbox_seen);
            tokio::select! {
                _ = pane.shutdown.changed() => {
                    pane.close();
                    return None;
                }
                () = router.wait_for_session_projection_change_after(sessions) => pane.feed_due = Instant::now(),
                () = router.wait_for_person_inbox_change_after(inbox) => pane.feed_due = Instant::now(),
                () = tokio::time::sleep(SETTINGS_TICK) => {}
            }
        }
    }))
}

struct Pane {
    router: Weak<CommandRouter>,
    token: String,
    run_id: String,
    session_id: String,
    owner_user_id: String,
    agent_id: String,
    files: PaneFiles,
    shutdown: tokio::sync::watch::Receiver<()>,
    /// Dialog ownership, dismissals and observed settings belong to the run, across GET streams.
    state: Arc<Mutex<RunPaneState>>,
    status: Option<String>,
    feed: Option<PaneFeed>,
    feed_due: Instant,
    sessions_seen: u64,
    inbox_seen: u64,
}

impl Pane {
    fn bound(&self, router: &CommandRouter) -> bool {
        self.shutdown.has_changed().is_ok()
            && router
                .runtime_mcp_catalog_run(&self.token)
                .is_some_and(|run| run.id() == self.run_id)
    }

    fn close(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((request_id, _)) = state.open.take() {
            dialogs().remove(&request_id);
        }
    }

    fn command(&self, request: &LocalDaemonRequest) -> KernelCommand {
        command_as(&self.owner_user_id, request)
    }

    async fn tick(&mut self, router: &CommandRouter) -> Vec<u8> {
        self.report_settings(router).await;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((request_id, card_id)) = state.open.clone() {
                if !dialogs().contains_key(&request_id) {
                    state.closed_here.insert(card_id);
                    state.open = None;
                    self.feed_due = Instant::now();
                }
            }
        }
        if Instant::now() >= self.feed_due {
            self.sessions_seen = router.session_projection_change_sequence();
            self.inbox_seen = router.person_inbox_change_sequence();
            self.feed_due = Instant::now() + FEED_REFRESH;
            self.read_feed(router).await;
        }
        let Some(feed) = &self.feed else {
            return Vec::new();
        };
        let mut frames = Vec::new();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((request_id, card_id)) = state.open.clone() {
            if !feed.cards.iter().any(|card| card.id == card_id) {
                // Answered elsewhere: close the terminal's dialog.
                dialogs().remove(&request_id);
                frames.extend(sse_message(&serde_json::json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "method": "notifications/cancelled",
                    "params": { "requestId": request_id, "reason": "Answered in Chariox." }
                })));
                state.open = None;
            }
        }
        state
            .closed_here
            .retain(|card_id| feed.cards.iter().any(|card| &card.id == card_id));
        if state.open.is_none() {
            // Claude's own permission dialog is the terminal face of its card.
            let next = feed.cards.iter().find(|card| {
                !state.closed_here.contains(&card.id)
                    && !is_claude_native_permission(&card.id, &self.run_id)
            });
            if let Some((request, dialog)) = next.and_then(|card| dialog_for(card, feed)) {
                let request_id = next_request_id();
                let dialog = Dialog {
                    run_id: self.run_id.clone(),
                    session_id: self.session_id.clone(),
                    owner_user_id: self.owner_user_id.clone(),
                    card_id: dialog.0,
                    answers: dialog.1,
                };
                state.open = Some((request_id.clone(), dialog.card_id.clone()));
                dialogs().insert(request_id.clone(), dialog);
                frames.extend(sse_message(&serde_json::json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": request_id,
                    "method": "elicitation/create",
                    "params": request
                })));
            } else if let Some(card) = next {
                // Nothing to open here (no link to the pane): the status line names it.
                state.closed_here.insert(card.id.clone());
            }
        }
        frames
    }

    async fn read_feed(&mut self, router: &CommandRouter) {
        let request = LocalDaemonRequest::GetPaneFeed(GetPaneFeedRequest {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
        });
        let feed = match router.dispatch(self.command(&request), request).await {
            Ok(LocalDaemonResponse::PaneFeed { feed }) => feed,
            Ok(_) => return,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.mcp.terminal_pane",
                    "pane feed read failed",
                    serde_json::json!({ "run_id": self.run_id, "error": error.to_string() }),
                );
                return;
            }
        };
        let status = status_lines(&feed);
        if self.status.as_deref() != Some(status.as_str())
            && publish(&self.files.status, &status).is_ok()
        {
            self.status = Some(status);
        }
        self.feed = Some(feed);
    }

    /// Reports a model, effort or mode the person switched to in the terminal.
    async fn report_settings(&mut self, router: &CommandRouter) {
        let model = std::fs::read_to_string(&self.files.model)
            .ok()
            .and_then(|raw| serde_json::from_str::<TerminalModel>(&raw).ok());
        let mode = std::fs::read_to_string(&self.files.mode).ok();
        let mut report = ReportPaneSettingsRequest {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
        };
        // Claude names the model it runs by its full id, the kernel may record an alias: the
        // first reading is the launch choice, a later different one is the person's switch.
        if let Some(model) = model {
            if let Some(seen) = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .model_seen
                .replace(model.clone())
            {
                if seen.model != model.model {
                    report.model = Some(model.model.clone());
                }
                if seen.effort != model.effort {
                    report.effort = Some(model.effort);
                }
            }
        }
        if let (Some(mode), Some(feed)) = (mode, &self.feed) {
            let (execution_mode, permission_level) = scope_for(mode.trim());
            report.execution_mode =
                execution_mode.filter(|mode| *mode != feed.settings.execution_mode);
            report.permission_level =
                permission_level.filter(|level| *level != feed.settings.permission_level);
        }
        if report.model.is_none()
            && report.effort.is_none()
            && report.execution_mode.is_none()
            && report.permission_level.is_none()
        {
            return;
        }
        let request = LocalDaemonRequest::ReportPaneSettings(report);
        match router.dispatch(self.command(&request), request).await {
            Ok(LocalDaemonResponse::PaneFeed { feed }) => self.feed = Some(feed),
            Ok(_) => {}
            Err(error) => crate::logging::warn_with_fields(
                "daemon.mcp.terminal_pane",
                "pane settings report failed",
                serde_json::json!({ "run_id": self.run_id, "error": error.to_string() }),
            ),
        }
        self.feed_due = Instant::now();
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct TerminalModel {
    model: String,
    effort: Option<String>,
}

/// Claude's permission mode as the kernel's mode and permissions. Claude's own finer modes
/// (accept edits, auto) keep the permissions the kernel records: they still ask for some tools.
fn scope_for(mode: &str) -> (Option<AgentExecutionMode>, Option<AgentPermissionLevel>) {
    match mode {
        "plan" => (Some(AgentExecutionMode::Plan), None),
        "default" => (
            Some(AgentExecutionMode::Build),
            Some(AgentPermissionLevel::Required),
        ),
        "bypassPermissions" => (
            Some(AgentExecutionMode::Build),
            Some(AgentPermissionLevel::Yolo),
        ),
        "acceptEdits" | "auto" | "dontAsk" => (Some(AgentExecutionMode::Build), None),
        _ => (None, None),
    }
}

fn command_as(owner_user_id: &str, request: &LocalDaemonRequest) -> KernelCommand {
    let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalIpc);
    caller.user_id = Some(owner_user_id.to_string());
    KernelCommand::from_local_request_with_caller(
        format!("terminal-pane-{}", next_request_id()),
        KernelCommandSource::LocalIpc,
        caller,
        None,
        None,
        request,
    )
}

fn next_request_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "{REQUEST_ID_PREFIX}{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn publish(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("writing");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

fn sse_message(message: &Value) -> Vec<u8> {
    format!("event: message\ndata: {message}\n\n").into_bytes()
}

/// The pane in at most three status lines: who and how it runs; what it waits for; stripes
/// and notices. Every word comes from the feed.
pub(super) fn status_lines(feed: &PaneFeed) -> String {
    let settings = &feed.settings;
    let mut path = feed.identity.parents.clone();
    path.push(feed.identity.agent_name.clone());
    let mut first = vec![feed.identity.room_name.clone(), path.join(" ▸ ")];
    first.extend(settings.account.clone());
    first.push(
        settings
            .model
            .clone()
            .unwrap_or_else(|| "default".to_string()),
    );
    first.push(
        settings
            .effort
            .clone()
            .unwrap_or_else(|| "default".to_string()),
    );
    first.push(settings.execution_mode.to_string());
    first.push(
        match settings.permission_level {
            AgentPermissionLevel::Required => "Ask first",
            AgentPermissionLevel::Yolo => "Full access",
        }
        .to_string(),
    );
    let mut lines = vec![first.join(" · ")];
    let mut second = match feed.state {
        PaneState::Idle => Vec::new(),
        PaneState::Working => vec!["Working".to_string()],
        PaneState::WaitingForAnswer => vec!["Waiting for your answer".to_string()],
    };
    match feed.cards.len() {
        0 => {}
        1 => second.push("1 card waiting".to_string()),
        count => second.push(format!("{count} cards waiting")),
    }
    second.extend(feed.cards.first().map(|card| card.open_elsewhere.clone()));
    if !second.is_empty() {
        lines.push(second.join(" · "));
    }
    let third = feed
        .stripes
        .iter()
        .map(|stripe| stripe.text.clone())
        .chain(feed.notices.iter().map(|notice| notice.title.clone()))
        .collect::<Vec<_>>();
    if !third.is_empty() {
        lines.push(third.join(" · "));
    }
    lines.join("\n")
}

/// The elicitation that shows a card in Claude's terminal: a form when a text terminal can
/// answer it, else a link to the same pane. `None` when there is neither.
fn dialog_for(card: &PaneCard, feed: &PaneFeed) -> Option<(Value, (String, DialogAnswers))> {
    let message = match &card.asker_name {
        Some(asker) => format!("{asker}: {}", card.message),
        None => card.message.clone(),
    };
    let message = match &card.title {
        Some(title) if !title.is_empty() => format!("{title}\n\n{message}"),
        _ => message,
    };
    if !card.text_terminal {
        let url = feed.link.clone()?;
        return Some((
            serde_json::json!({
                "mode": "url",
                "message": message,
                "url": url,
                "elicitationId": card.id,
            }),
            (card.id.clone(), DialogAnswers::Link),
        ));
    }
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    if !card.choices.is_empty() {
        properties.insert(
            "choice".into(),
            serde_json::json!({
                "type": "string",
                "title": "Answer",
                "oneOf": card.choices.iter().map(|choice| serde_json::json!({
                    "const": choice.id,
                    "title": choice.label,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    if let Some(reply) = &card.reply {
        let mut field = serde_json::json!({ "type": "string", "title": reply.label });
        if let Some(placeholder) = &reply.placeholder {
            field["description"] = placeholder.clone().into();
        }
        properties.insert("reply".into(), field);
    }
    match (card.choices.is_empty(), card.reply.is_some()) {
        (false, false) => required.push("choice"),
        (true, true) => required.push("reply"),
        (true, false) => return None,
        (false, true) => {}
    }
    Some((
        serde_json::json!({
            "mode": "form",
            "message": message,
            "requestedSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        }),
        (
            card.id.clone(),
            DialogAnswers::Form {
                choices: card
                    .choices
                    .iter()
                    .map(|choice| choice.id.clone())
                    .collect(),
                reply_choice: card.reply.as_ref().map(|reply| reply.id.clone()),
            },
        ),
    ))
}

/// The card answer an elicitation result carries. Decline and cancel are "not now": the
/// card stays open for every client.
fn answer_for(dialog: &Dialog, result: &Value) -> Option<RespondToInteractionRequest> {
    if result.get("action").and_then(Value::as_str) != Some("accept") {
        return None;
    }
    let DialogAnswers::Form {
        choices,
        reply_choice,
    } = &dialog.answers
    else {
        return None;
    };
    let content = result.get("content");
    let field = |name: &str| {
        content
            .and_then(|content| content.get(name))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
    };
    let respond = |choice_id: &str, custom_reply: Option<String>| RespondToInteractionRequest {
        session_id: dialog.session_id.clone(),
        interaction_id: dialog.card_id.clone(),
        choice_id: choice_id.to_string(),
        custom_reply,
        passkey: None,
        passkey_remember_minutes: None,
    };
    if let (Some(reply_choice), Some(reply)) = (reply_choice, field("reply")) {
        return Some(respond(reply_choice, Some(reply.to_string())));
    }
    let choice = field("choice")?;
    choices
        .iter()
        .any(|id| id == choice)
        .then(|| respond(choice, None))
}

/// Whether a JSON-RPC message is Claude's answer to one of the pane's dialogs.
pub(super) fn is_dialog_answer(payload: &Value) -> bool {
    payload.get("method").is_none()
        && payload
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.starts_with(REQUEST_ID_PREFIX))
}

/// Takes the terminal's answer to a dialog and answers the card with it, as the person
/// who runs the terminal.
pub(super) async fn answer(router: Arc<CommandRouter>, token: &str, payload: &Value) {
    let Some(request_id) = payload.get("id").and_then(Value::as_str) else {
        return;
    };
    let Some(run) = router.runtime_mcp_catalog_run(token) else {
        return;
    };
    let dialog = {
        let mut dialogs = dialogs();
        match dialogs.get(request_id) {
            Some(dialog) if dialog.run_id == run.id() => dialogs.remove(request_id),
            _ => None,
        }
    };
    let Some(dialog) = dialog else {
        return;
    };
    run_state(&router, token, run.id())
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .closed_here
        .insert(dialog.card_id.clone());
    let Some(answer) = payload
        .get("result")
        .and_then(|result| answer_for(&dialog, result))
    else {
        return;
    };
    let request = LocalDaemonRequest::RespondToInteraction(answer);
    if let Err(error) = router
        .dispatch(command_as(&dialog.owner_user_id, &request), request)
        .await
    {
        crate::logging::warn_with_fields(
            "daemon.mcp.terminal_pane",
            "terminal answer refused",
            serde_json::json!({ "run_id": dialog.run_id, "card_id": dialog.card_id, "error": error.to_string() }),
        );
    }
}

#[cfg(test)]
#[path = "terminal_pane_tests.rs"]
mod tests;
