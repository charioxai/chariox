//! MP-08 / MP-11: selection capture for prompt notes; owner admission and surface wiring only.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::local::{NoteAnchor, NoteCommand, NoteResult, NoteSelection, NoteWindow};
use crate::runtime::kernel_browser_host::KernelBrowserAdmission;
use crate::runtime::notes::observation::BrowserNoteObservation;

fn selection_of(
    anchor: NoteAnchor,
    box_css: Option<crate::local::NoteBox>,
) -> Result<NoteSelection, String> {
    crate::runtime::notes::validate_anchor(&anchor)?;
    crate::runtime::notes::validate_box(box_css.as_ref())?;
    Ok(NoteSelection { anchor, box_css })
}

impl KernelRuntimeState {
    pub(crate) async fn notes_terminal_request(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        command: NoteCommand,
    ) -> Result<NoteResult, DaemonError> {
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        Box::pin(self.notes_command(&user, &actor, None, command)).await
    }
    async fn notes_command(
        &self,
        user: &str,
        actor: &str,
        admission: Option<KernelBrowserAdmission>,
        command: NoteCommand,
    ) -> Result<NoteResult, DaemonError> {
        let _ = actor;
        let host = &self.owned.kernel_browser_host;
        host.check_admission(admission.as_ref())
            .map_err(host_error)?;
        let selection = match command {
            NoteCommand::CaptureSelection { window } => {
                let observation =
                    Box::pin(self.observe_note_window(user, &window, admission.clone())).await?;
                host.check_admission(admission.as_ref())
                    .map_err(host_error)?;
                observation
                    .selection
                    .map(|selection| {
                        selection_of(
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
                    .transpose()
                    .map_err(host_error)?
            }
            NoteCommand::ReportSelection { anchor, box_css } => {
                if matches!(
                    anchor.window,
                    NoteWindow::KernelBrowser { .. } | NoteWindow::RoomBrowser { .. }
                ) {
                    return Err(host_error(
                        "MD-N2: streamed selection must come from controller".into(),
                    ));
                }
                self.check_note_window(user, &anchor.window)?;
                Some(selection_of(anchor, box_css).map_err(host_error)?)
            }
        };
        host.check_admission(admission.as_ref())
            .map_err(host_error)?;
        Ok(NoteResult::SelectionChanged { selection })
    }
    fn check_note_window(&self, user: &str, window: &NoteWindow) -> Result<(), DaemonError> {
        crate::runtime::notes::validate_window(window).map_err(host_error)?;
        if let NoteWindow::RoomBrowser { session_id, .. }
        | NoteWindow::Terminal { session_id, .. } = window
        {
            let session = self.owned.session_snapshot(session_id)?;
            // MP-11: notes use the account owner's canonical identity, while a
            // Room can retain the configured Cloud identity of that same owner.
            // Normalize both sides with the existing owner alias; collaborators
            // keep their independent identities and still require membership.
            let config = self.owned.config_projection.snapshot();
            let owner =
                crate::account_profile::provider_account_authority_owner_user_id(&config, user);
            if !session.members().iter().any(|member| {
                crate::account_profile::provider_account_authority_owner_user_id(
                    &config,
                    member.user_id(),
                ) == owner
            }) {
                return Err(host_error("MD-N2: Room membership required".into()));
            }
        }
        Ok(())
    }
    async fn observe_note_window(
        &self,
        user: &str,
        window: &NoteWindow,
        admission: Option<KernelBrowserAdmission>,
    ) -> Result<BrowserNoteObservation, DaemonError> {
        self.check_note_window(user, window)?;
        let observation = match window {
            NoteWindow::KernelBrowser { tab_id, generation } => {
                let value=self.kernel_browser_operation_admitted(user,admission,"host.browser",serde_json::json!({"op":"note_selection","tab_id":tab_id,"generation":generation})).await?;
                serde_json::from_value::<BrowserNoteObservation>(value["observation"].clone())
                    .map_err(|_| host_error("MD-N2: invalid host selection result".into()))?
            }
            NoteWindow::RoomBrowser { session_id, tab_id } => {
                self.reconcile_browser_controller_environment(session_id)
                    .await?;
                let binding = self
                    .room_environment_controller_tab_binding(session_id, tab_id)
                    .map_err(|_| host_error("MD-N2: Room tab unavailable".into()))?;
                let result=self.room_browser_controller_command(session_id,crate::transport::room_browser_controller::RoomBrowserControllerCommand::NoteObservation {target_id:binding.runtime_target_id.clone(),document_id:binding.document_id.clone()}).await?;
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

#[cfg(test)]
mod owner_alias_tests {
    use super::*;
    use crate::runtime::router::CommandRouter;
    use crate::session::RuntimeSession;

    #[tokio::test]
    async fn mp08_mp10_mp11_notes_room_membership_uses_only_the_configured_owner_alias() {
        for (configured_owner, room_owner, caller, allowed) in [
            (Some("cloud-owner"), "cloud-owner", "local", true),
            (Some("cloud-owner"), "local", "cloud-owner", true),
            (Some("cloud-owner"), "cloud-owner", "collaborator", false),
            (Some("cloud-owner"), "collaborator", "local", false),
            (Some("cloud-owner"), "collaborator", "collaborator", true),
            (None, "cloud-owner", "local", false),
            (None, "local", "local", true),
        ] {
            let mut config = crate::config::DaemonConfig::for_tests();
            config.cloud_relay =
                configured_owner.map(|owner| crate::config::PersistedCloudRelayProfile {
                    user_id: owner.into(),
                    ..Default::default()
                });
            let app = crate::app::DaemonApp::bootstrap(config).unwrap();
            let mut room = RuntimeSession::new(
                format!("notes-alias-{:016x}", rand::random::<u64>()),
                None,
                "workspace",
                "worktree",
                "machine",
                "kernel",
            );
            room.set_owner_user_id(room_owner);
            let session_id = room.id().to_owned();
            app.sessions_mut().restore_session(room);
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            for window in [
                NoteWindow::RoomBrowser {
                    session_id: session_id.clone(),
                    tab_id: "tab".into(),
                },
                NoteWindow::Terminal {
                    session_id: session_id.clone(),
                    window_id: "terminal".into(),
                },
            ] {
                assert_eq!(
                    router
                        .runtime_state()
                        .check_note_window(caller, &window)
                        .is_ok(),
                    allowed,
                    "configured={configured_owner:?}, Room={room_owner}, caller={caller}, window={window:?}",
                );
            }
        }
    }
}
