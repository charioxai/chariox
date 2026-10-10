//! MD-N1–N4 / MP-08 / MP-11: owner/focus admission and surface wiring only.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::local::{NoteAnchor, NoteCommand, NoteResult, NoteWindow};
use crate::runtime::kernel_browser_host::{KernelBrowserAdmission, KernelBrowserCapability};
use crate::runtime::notes::observation::BrowserNoteObservation;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

const LOADER: &str = "chariox.load_notes";
const TOOLS: &[&str] = &[
    LOADER,
    "chariox.list_notes",
    "chariox.read_note",
    "chariox.reply_to_note",
    "chariox.resolve_note",
];

impl KernelRuntimeState {
    /// MD-N5 / MP-10: fixture assets use the normal Room controller route.
    #[cfg(test)]
    pub(crate) async fn notes_drill_app_view(
        &self,
        session_id: &str,
        request: crate::runtime::browser_controller_app_view::BrowserAppViewRequest,
    ) -> Result<(), DaemonError> {
        Box::pin(self.room_browser_controller_command(
            session_id,
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::AppView {
                request,
            },
        ))
        .await
        .map(|_| ())
    }

    pub(crate) async fn notes_terminal_request(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        command: NoteCommand,
    ) -> Result<NoteResult, DaemonError> {
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        Box::pin(self.notes_command(&user, &actor, None, command)).await
    }
    pub(super) fn notes_tool_specs(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        agent: &crate::agent::AgentInstance,
    ) -> Vec<RuntimeToolSpec> {
        let _ = run;
        let mut specs=vec![RuntimeToolSpec { name:LOADER.into(),description:"MD-N4: load notes on demand. Notes belong to the user and focus claims notes and active tasks retain their claimed notes.".into(),input_schema:serde_json::json!({"type":"object","properties":{},"additionalProperties":false}) }];
        if !self.owned.kernel_browser_host.is_loaded_for(
            &self.provider_account_authority_owner_user_id(agent.owner_user_id()),
            agent.id(),
            KernelBrowserCapability::Notes,
        ) {
            return specs;
        }
        for name in &TOOLS[1..] {
            let list = *name == "chariox.list_notes";
            let reply = *name == "chariox.reply_to_note";
            let mut properties = serde_json::json!({"note_id":{"type":"string","maxLength":512}});
            let mut required = vec!["note_id"];
            if list {
                properties = serde_json::json!({"window":{"type":"object","properties":{"kind":{"enum":["kernel_browser","room_browser","panel","terminal"]},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"session_id":{"type":"string"},"window_id":{"type":"string"}},"required":["kind"],"additionalProperties":false}});
                required = vec!["window"];
            }
            if reply {
                properties["comment"] =
                    serde_json::json!({"type":"string","minLength":1,"maxLength":16384});
                required.push("comment");
            }
            specs.push(RuntimeToolSpec { name:(*name).into(),description:"MD-N4: notes on a window or selected text; access retains claimed notes while the task is active or awaiting a wake. Read refreshes the text quote anchor; original quote survives a missing page.".into(),input_schema:serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}) });
        }
        specs
    }
    pub(super) fn is_note_tool(name: &str) -> bool {
        TOOLS.contains(&name)
    }
    pub(super) async fn notes_tool(
        &self,
        token: &str,
        name: &str,
        mut arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(host_error(
                "MD-N4: one admitted local provider run required".into(),
            ));
        };
        let agent = self.user_domain_tool_agent(run).await?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let host = self.owned.kernel_browser_host.clone();
        if name == LOADER {
            if arguments.as_object().is_none_or(|o| !o.is_empty()) {
                return Err(host_error("MD-N4: loader accepts no arguments".into()));
            }
            host.load_for(&user, agent.id(), KernelBrowserCapability::Notes)
                .map_err(host_error)?;
            return Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"loaded":&TOOLS[1..]}),
            });
        }
        if !arguments.is_object() {
            return Err(host_error("MD-N4: object arguments required".into()));
        }
        if arguments.get("op").is_some() {
            return Err(host_error("MD-N4: unknown arguments".into()));
        }
        arguments["op"] = serde_json::json!(match name {
            "chariox.list_notes" => "list",
            "chariox.read_note" => "read",
            "chariox.reply_to_note" => "reply",
            "chariox.resolve_note" => "resolve",
            _ => return Err(host_error("MD-N4: unknown note tool".into())),
        });
        let command: NoteCommand = serde_json::from_value(arguments)
            .map_err(|_| host_error("MD-N4: invalid note arguments".into()))?;
        let authority = self.clone();
        let auth_token = token.to_string();
        let run_id = run.id().to_string();
        let admission = host
            .admit_for(&user, agent.id(), KernelBrowserCapability::Notes)
            .map_err(host_error)?
            .with_authority(move || {
                let runs = authority
                    .owned
                    .provider_store
                    .get_runs_by_runtime_mcp_auth_token(&auth_token);
                let [current] = runs.as_slice() else {
                    return false;
                };
                current.id() == run_id && authority.kernel_browser_agent(current).is_some()
            });
        let result = Box::pin(self.notes_command(
            &user,
            &format!("agent:{}", agent.id()),
            Some(admission),
            command,
        ))
        .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::to_value(result)
                .map_err(|_| host_error("MD-N4: invalid note result".into()))?,
        })
    }
    async fn notes_command(
        &self,
        user: &str,
        actor: &str,
        admission: Option<KernelBrowserAdmission>,
        command: NoteCommand,
    ) -> Result<NoteResult, DaemonError> {
        use NoteCommand::*;
        let host = &self.owned.kernel_browser_host;
        host.check_admission(admission.as_ref())
            .map_err(host_error)?;
        let store = &self.owned.notes;
        if let Read { note_id }
        | Reply { note_id, .. }
        | Resolve { note_id }
        | Reanchor { note_id }
        | Ask { note_id } = &command
        {
            let resource = crate::local::UserDomainResource::Note {
                note_id: note_id.clone(),
            };
            host.claim_resource(admission.as_ref(), resource.clone(), false)
                .map_err(host_error)?;
            host.resource_notice(admission.as_ref(), resource);
        }
        let result = match command {
            CaptureSelection { window } => {
                let observation =
                    Box::pin(self.observe_note_window(user, &window, None, admission.clone()))
                        .await?;
                host.check_admission(admission.as_ref())
                    .map_err(host_error)?;
                let selection = observation
                    .selection
                    .map(|selection| {
                        host.note_operation(user, admission.as_ref(), || {
                            store.selection(
                                user,
                                NoteAnchor {
                                    window,
                                    url: Some(observation.url),
                                    document_id: Some(observation.document_id),
                                    hint: Some(selection.hint),
                                    quote: selection.quote,
                                },
                                Some(selection.box_css),
                            )
                        })
                    })
                    .transpose()
                    .map_err(host_error)?;
                NoteResult::SelectionChanged { selection }
            }
            ReportSelection { anchor, box_css } => {
                if matches!(
                    anchor.window,
                    NoteWindow::KernelBrowser { .. } | NoteWindow::RoomBrowser { .. }
                ) {
                    return Err(host_error(
                        "MD-N2: streamed selection must come from controller".into(),
                    ));
                }
                self.check_note_window(user, &anchor.window)?;
                NoteResult::SelectionChanged {
                    selection: Some(
                        host.note_operation(user, admission.as_ref(), || {
                            store.selection(user, anchor, box_css)
                        })
                        .map_err(host_error)?,
                    ),
                }
            }
            Create {
                selection_id,
                comment,
            } => NoteResult::NoteChanged {
                note: host
                    .note_operation(user, admission.as_ref(), || {
                        store.create(user, &selection_id, &comment)
                    })
                    .map_err(host_error)?,
            },
            List { window } => NoteResult::NotesListed {
                notes: host
                    .note_operation(user, admission.as_ref(), || {
                        store.list(user, &window, actor)
                    })
                    .map_err(host_error)?,
            },
            Read { note_id } | Reanchor { note_id } => {
                let mut note = host
                    .note_operation(user, admission.as_ref(), || {
                        store.read(user, &note_id, actor)
                    })
                    .map_err(host_error)?;
                if matches!(
                    note.anchor.window,
                    NoteWindow::KernelBrowser { .. } | NoteWindow::RoomBrowser { .. }
                ) {
                    match Box::pin(self.observe_note_window(
                        user,
                        &note.anchor.window,
                        Some(note.anchor.quote.clone()),
                        admission.clone(),
                    ))
                    .await
                    {
                        Ok(observation) => {
                            host.check_admission(admission.as_ref())
                                .map_err(host_error)?;
                            let anchoring = observation.anchoring.ok_or_else(|| {
                                host_error("MD-N2: missing anchoring result".into())
                            })?;
                            note = host
                                .note_operation(user, admission.as_ref(), || {
                                    store.reanchor(
                                        user,
                                        &note_id,
                                        actor,
                                        &anchoring.anchor_state,
                                        anchoring.box_css,
                                        Some(observation.document_id),
                                    )
                                })
                                .map_err(host_error)?;
                        }
                        Err(_) => {
                            host.check_admission(admission.as_ref())
                                .map_err(host_error)?;
                            note = host
                                .note_operation(user, admission.as_ref(), || {
                                    store.reanchor(
                                        user,
                                        &note_id,
                                        actor,
                                        "unavailable",
                                        None,
                                        note.anchor.document_id.clone(),
                                    )
                                })
                                .map_err(host_error)?;
                        }
                    }
                }
                NoteResult::NoteChanged { note }
            }
            Reply { note_id, comment } => NoteResult::NoteChanged {
                note: host
                    .note_operation(user, admission.as_ref(), || {
                        store.reply(user, &note_id, actor, &comment)
                    })
                    .map_err(host_error)?,
            },
            Resolve { note_id } => NoteResult::NoteChanged {
                note: host
                    .note_operation(user, admission.as_ref(), || {
                        store.resolve(user, &note_id, actor)
                    })
                    .map_err(host_error)?,
            },
            Ask { note_id } => host
                .note_operation(user, admission.as_ref(), || {
                    store.ask(user, &note_id, actor)
                })
                .map_err(host_error)?,
        };
        let result = if let NoteResult::NotesListed { mut notes } = result {
            notes.retain(|note| {
                host.resource_visible(
                    admission.as_ref(),
                    &crate::local::UserDomainResource::Note {
                        note_id: note.note_id.clone(),
                    },
                )
            });
            NoteResult::NotesListed { notes }
        } else {
            result
        };
        host.check_admission(admission.as_ref())
            .map_err(host_error)?;
        Ok(result)
    }
    fn check_note_window(&self, user: &str, window: &NoteWindow) -> Result<(), DaemonError> {
        crate::runtime::notes::validate_window(window).map_err(host_error)?;
        if let NoteWindow::RoomBrowser { session_id, .. }
        | NoteWindow::Terminal { session_id, .. } = window
        {
            let session = self.owned.session_snapshot(session_id)?;
            if !session.has_member(user) {
                return Err(host_error("MD-N2: Room membership required".into()));
            }
        }
        Ok(())
    }
    async fn observe_note_window(
        &self,
        user: &str,
        window: &NoteWindow,
        quote: Option<crate::local::NoteTextQuote>,
        admission: Option<KernelBrowserAdmission>,
    ) -> Result<BrowserNoteObservation, DaemonError> {
        self.check_note_window(user, window)?;
        let observation = match window {
            NoteWindow::KernelBrowser { tab_id, generation } => {
                // New capture must keep its caller's generation fence. A durable
                // note reads the current binding for its stable restored tab.
                let generation = if quote.is_some() {
                    let current = self
                        .kernel_browser_operation_admitted(
                            user,
                            admission.clone(),
                            "host.browser",
                            serde_json::json!({"op":"state"}),
                        )
                        .await?;
                    current["generation"].as_u64().ok_or_else(|| {
                        host_error("MD-N2: current browser generation unavailable".into())
                    })?
                } else {
                    *generation
                };
                let value=self.kernel_browser_operation_admitted(user,admission,"host.browser",serde_json::json!({"op":if quote.is_some(){"note_reanchor"}else{"note_selection"},"tab_id":tab_id,"generation":generation,"quote":quote})).await?;
                serde_json::from_value::<BrowserNoteObservation>(value["observation"].clone())
                    .map_err(|_| host_error("MD-N2: invalid host selection result".into()))?
            }
            NoteWindow::RoomBrowser { session_id, tab_id } => {
                self.reconcile_browser_controller_environment(session_id)
                    .await?;
                let binding = self
                    .room_environment_controller_tab_binding(session_id, tab_id)
                    .map_err(|_| host_error("MD-N2: Room tab unavailable".into()))?;
                let result=self.room_browser_controller_command(session_id,crate::transport::room_browser_controller::RoomBrowserControllerCommand::NoteObservation {target_id:binding.runtime_target_id.clone(),document_id:binding.document_id.clone(),quote}).await?;
                let crate::transport::room_browser_controller::RoomBrowserControllerResult::NoteObservation {observation:Some(observation)}=result else { return Err(host_error("MD-N2: Room selection unavailable".into())); };
                observation
                    .validate(Some(&binding.runtime_target_id), Some(&binding.document_id))
                    .map_err(host_error)?;
                observation
            }
            _ => {
                return Err(host_error(
                    "MD-N2: native panels report their own DOM selection".into(),
                ))
            }
        };
        observation.validate(None, None).map_err(host_error)?;
        Ok(observation)
    }
}
