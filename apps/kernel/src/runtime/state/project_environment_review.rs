//! MP-08 / MP-10 / MP-11: One kernel-owned review, projected by existing interactions.
use super::project_environment_export::environment_failure;
use super::*;
use crate::project_environment::*;
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};
use zeroize::Zeroizing;

impl KernelRuntimeState {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn review_project_environment(
        &self,
        session_id: &str,
        agent_id: &str,
        state: &mut StoredProjectEnvironment,
        input: &mut ProjectEnvironmentDiscoveryInput,
        roots: &BTreeMap<String, PathBuf>,
        workspace_environment: &BTreeMap<String, BTreeMap<String, Zeroizing<String>>>,
        project: &str,
        target: &str,
        code: String,
        vault: &dyn crate::secret::CredentialVaultStore,
        owner_context_id: Option<&str>,
        vault_transfer: bool,
    ) -> Result<ProjectPrivateFileAdditions, DaemonError> {
        // MP-11: credential-free owner copies use the same human-only guard as kernel-only copies.
        let id = owner_context_id.map_or_else(
            || format!("project-environment:{}", rand::random::<u64>()),
            |context| format!("owner-context:{context}"),
        );
        // MP-08 / MP-11: only context transfers move history through Vault and admit it.
        let vault_items = if vault_transfer {
            let scan_roots = roots.clone();
            tokio::task::spawn_blocking(move || {
                crate::managed_context::development::vault_artifacts::review_items(&scan_roots)
            })
            .await
            .map_err(|_| environment_failure("Vault review task failed"))??
        } else {
            Vec::new()
        };
        let mut expanded = false;
        let mut files_view = UnreviewedFilesView::default();
        let mut finding = false;
        let mut revise = false;
        let mut supply: Option<String> = None;
        loop {
            let resolved =
                resolve_project_environment(&state.manifest, roots, workspace_environment, vault)?;
            for entry in &mut state.manifest.entries {
                entry.status = resolved
                    .unresolved
                    .iter()
                    .find(|missing| {
                        missing.name == entry.name && missing.workspace_id == entry.workspace_id
                    })
                    .map_or(ProjectEnvironmentEntryStatus::Found, |missing| {
                        missing.status
                    });
            }
            // Flipped files leave the list, so keep the page inside what is still there.
            let last_page = files_view.matching(&state.manifest).saturating_sub(1)
                / LISTED_UNREVIEWED_FILES
                * LISTED_UNREVIEWED_FILES;
            files_view.offset = files_view.offset.min(last_page);
            let mut review = ProjectEnvironmentReview::build_with_view(
                state,
                project,
                target,
                code.clone(),
                expanded,
                false,
                &files_view,
            );
            if !vault_items.is_empty() {
                review.rows.push(ProjectEnvironmentReviewRow { label: "Vault".into(), summary: "Secret-bearing files and history will travel through your Chariox Vault. Unlock the source and target Vaults before copying.".into() });
                for item in &vault_items {
                    review.rows.push(ProjectEnvironmentReviewRow {
                        label: "Vault".into(),
                        summary: format!("{} · {}", item.path, item.kind),
                    });
                }
            }
            let mut choices = vec![
                RuntimeInteractionChoice::new(
                    "continue",
                    "Looks good, continue",
                    "continue",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "change",
                    "Change something...",
                    "change",
                    Some(RuntimeInteractionChoiceStyle::Secondary),
                ),
                RuntimeInteractionChoice::new(
                    "details",
                    if expanded { "Collapse" } else { "Details" },
                    "details",
                    Some(RuntimeInteractionChoiceStyle::Secondary),
                ),
            ];
            let (unreviewed_left, unreviewed_brought) =
                unreviewed_project_private_files(&state.manifest);
            if unreviewed_left > 0 {
                choices.push(RuntimeInteractionChoice::new(
                    "bring-unreviewed",
                    format!("Bring all {unreviewed_left} not-reviewed ignored files"),
                    "bring-unreviewed",
                    None,
                ));
            }
            if unreviewed_brought > 0 {
                choices.push(RuntimeInteractionChoice::new(
                    "leave-unreviewed",
                    format!("Leave the {unreviewed_brought} not-reviewed ignored files behind"),
                    "leave-unreviewed",
                    None,
                ));
            }
            if expanded {
                let matching = files_view.matching(&state.manifest);
                if files_view.offset + LISTED_UNREVIEWED_FILES < matching {
                    choices.push(RuntimeInteractionChoice::new(
                        "files-next",
                        "Show the next not-reviewed ignored files",
                        "files-next",
                        None,
                    ));
                }
                if files_view.offset > 0 {
                    choices.push(RuntimeInteractionChoice::new(
                        "files-previous",
                        "Show the previous not-reviewed ignored files",
                        "files-previous",
                        None,
                    ));
                }
                if matching > 0 || !files_view.query.is_empty() {
                    choices.push(RuntimeInteractionChoice::new(
                        "files-find",
                        "Search not-reviewed ignored files...",
                        "files-find",
                        None,
                    ));
                }
                if !files_view.query.is_empty() {
                    choices.push(RuntimeInteractionChoice::new(
                        "files-all",
                        format!(
                            "Show all not-reviewed ignored files (searching \"{}\")",
                            files_view.query
                        ),
                        "files-all",
                        None,
                    ));
                }
                for file in review
                    .files
                    .iter()
                    .filter(|file| !review.changed_only || file.changed)
                {
                    let is_secret = state.manifest.private_files.iter().any(|decision| {
                        decision.secret_looking
                            && project_environment_item_id(&decision.workspace_id, &decision.path)
                                == file.id
                    });
                    if !is_secret && !secret_looking_project_path(&file.path) && !crate::workspace_live_sync_ignore::workspace_live_sync_force_excluded_path(&file.path) {
                        let action = format!("file-{}", file.id);
                        choices.push(RuntimeInteractionChoice::new(&action, format!("{} {}", if file.bring {"Leave"} else {"Bring"}, file.path), &action, None));
                    }
                }
                for entry in review
                    .inputs
                    .iter()
                    .filter(|entry| !review.changed_only || entry.changed)
                {
                    let action = format!("input-{}", entry.id);
                    choices.push(RuntimeInteractionChoice::new(
                        &action,
                        format!("Change {} inclusion", entry.name),
                        &action,
                        None,
                    ));
                }
            }
            for missing in review.inputs.iter().filter(|entry| entry.missing) {
                let action = format!("paste-{}", missing.id);
                choices.push(RuntimeInteractionChoice::new(
                    &action,
                    format!("Paste {}", missing.name),
                    &action,
                    None,
                ));
            }
            let custom = if finding {
                Some(RuntimeInteractionCustomChoice::new(
                    "find",
                    "Search not-reviewed ignored files...",
                    Some("Part of the file path".into()),
                    Some(1),
                    Some(256),
                ))
            } else if revise {
                Some(RuntimeInteractionCustomChoice::new(
                    "revise",
                    "Change something...",
                    Some("What should follow this Project?".into()),
                    Some(1),
                    Some(4096),
                ))
            } else if let Some(input_id) = &supply {
                let missing = review
                    .inputs
                    .iter()
                    .find(|entry| &entry.id == input_id && entry.missing)
                    .ok_or_else(|| environment_failure("Project input no longer missing"))?;
                Some(RuntimeInteractionCustomChoice::secret(
                    "supply",
                    format!("Paste {}", missing.name),
                    None,
                    Some(1),
                    Some(128 * 1024),
                ))
            } else {
                None
            };
            if !review.inputs.iter().all(|entry| !entry.missing) {
                choices.push(RuntimeInteractionChoice::new(
                    "skip",
                    "Skip missing values and continue",
                    "skip",
                    Some(RuntimeInteractionChoiceStyle::Secondary),
                ));
            }
            choices.push(RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(RuntimeInteractionChoiceStyle::Danger),
            ));
            let message = if owner_context_id.is_some() {
                "Review your Project files and any selected kernel extensions and personal instructions before continuing: automatic credential checks cannot identify every secret in free-form files."
            } else {
                "Review your Project setup"
            };
            let interaction = RuntimeInteraction::new(
                &id,
                agent_id,
                RuntimeInteractionKind::Choice,
                RuntimeInteractionLevel::Info,
                Some(format!("Ready to move {project} to {target}")),
                message,
                choices,
                custom,
                Some(900),
                Some("cancel".into()),
            )
            .with_project_environment_review(review.clone());
            // MP-11: keep Project editing controls on the existing terminal UI,
            // bind its human owner and never dispatch this review to a Meta agent.
            let receiver = if owner_context_id.is_some() {
                self.create_terminal_credential_interaction(session_id, interaction)?
            } else {
                self.create_runtime_interaction(session_id, interaction)
                    .await?
            };
            let resolution = match tokio::time::timeout(Duration::from_secs(900), receiver).await {
                Ok(Ok(resolution)) => resolution,
                _ => {
                    self.timeout_runtime_interaction(session_id, &id).await?;
                    return Err(environment_failure("Project review timed out"));
                }
            };
            let action = resolution.choice_id.as_deref().unwrap_or("cancel");
            match action {
                "continue" | "skip" => {
                    let additions = self.retrieve_project_private_files(state, roots).await?;
                    validate_project_private_files_present(&state.manifest, roots)?;
                    accept_project_environment_review(state, review);
                    return Ok(additions);
                }
                "bring-unreviewed" | "leave-unreviewed" => {
                    set_unreviewed_project_private_files(state, action == "bring-unreviewed");
                    supply = None;
                }
                "details" => {
                    expanded = !expanded;
                    revise = false;
                    finding = false;
                    supply = None;
                }
                "files-next" => files_view.offset += LISTED_UNREVIEWED_FILES,
                "files-previous" => {
                    files_view.offset = files_view.offset.saturating_sub(LISTED_UNREVIEWED_FILES)
                }
                "files-find" => finding = true,
                "files-all" => files_view = UnreviewedFilesView::default(),
                "find" => {
                    let query = resolution
                        .reply
                        .ok_or_else(|| environment_failure("Project file search is missing"))?;
                    if query.len() > 256 || query.contains('\0') {
                        return Err(environment_failure("Project file search exceeds bounds"));
                    }
                    files_view = UnreviewedFilesView { offset: 0, query };
                    finding = false;
                }
                "change" => {
                    finding = false;
                    revise = true;
                    supply = None;
                    expanded = true;
                }
                "revise" => {
                    let reply = Zeroizing::new(
                        resolution
                            .reply
                            .ok_or_else(|| environment_failure("Project revision is missing"))?,
                    );
                    if reply.len() > 4096 || reply.contains('\0') {
                        return Err(environment_failure("Project revision exceeds bounds"));
                    }
                    input.previous_manifest = Some(state.manifest.clone());
                    input.revision = Some(reply.to_string());
                    state.manifest = self
                        .discover_project_environment(session_id, agent_id, input)
                        .await?;
                    input.revision = None;
                    revise = false;
                    expanded = true;
                }
                "supply" => {
                    let input_id = supply
                        .take()
                        .ok_or_else(|| environment_failure("Project input was not selected"))?;
                    let missing = review
                        .inputs
                        .iter()
                        .find(|entry| entry.id == input_id && entry.missing)
                        .ok_or_else(|| environment_failure("unknown Project input"))?;
                    let value =
                        Zeroizing::new(resolution.reply.ok_or_else(|| {
                            environment_failure("Project input reply is missing")
                        })?);
                    supply_project_environment_inputs(
                        state,
                        BTreeMap::from([(
                            (missing.workspace_id.clone(), missing.name.clone()),
                            value,
                        )]),
                        vault,
                    )?;
                    // Persist each supplied value's locator immediately; a cancelled review cannot lose it.
                    ProjectEnvironmentStore::new(
                        &self
                            .owned
                            .config_projection
                            .snapshot()
                            .private_runtime_state_root(),
                    )
                    .save(state)?;
                }
                _ if action.starts_with("file-") => {
                    flip_project_environment_item(state, &action[5..], true)?;
                    expanded = true;
                }
                _ if action.starts_with("input-") => {
                    flip_project_environment_item(state, &action[6..], false)?;
                    expanded = true;
                }
                _ if action.starts_with("paste-") => {
                    supply = Some(action[6..].into());
                    finding = false;
                    revise = false;
                }
                _ => return Err(environment_failure("Project export cancelled")),
            }
        }
    }
}

pub(super) fn project_environment_code_summary(
    repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
) -> String {
    repositories
        .iter()
        .map(|repository| {
            let git = |args: &[&str]| -> Option<String> {
                let output = crate::process_spawn::command("git")
                    .arg("-C")
                    .arg(&repository.worktree_path)
                    .args(args)
                    .output()
                    .ok()?;
                if !output.status.success() || output.stdout.len() > 4 * 1024 * 1024 {
                    return None;
                }
                String::from_utf8(output.stdout).ok()
            };
            let branch =
                git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "workspace".into());
            let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
            let dirty = git(&["status", "--porcelain", "-z", "--untracked-files=normal"])
                .map_or(0, |status| {
                    status.split('\0').filter(|path| !path.is_empty()).count()
                });
            format!(
                "{} {}@{} · {} uncommitted",
                repository
                    .worktree_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                branch.trim(),
                commit.trim(),
                dirty
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}
