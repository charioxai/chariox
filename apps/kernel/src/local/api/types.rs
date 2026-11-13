use super::*;

use crate::session::{
    PromptQueueItem, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionLevel, RuntimeProject,
};
use crate::slice::{
    SliceBackendKind, SliceDisplayEndpoint, SliceLogEntry, SliceProviderLoginStart, SliceRecord,
};
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputKind, TerminalOutputRecord};
use chariox_relay::protocol::RelayKernelPresence;

mod account_transfer;
mod agent_deadline;
mod agent_lifecycle;
mod agent_peek;
mod agent_tree;
mod agent_triggers;
mod agent_utility;
mod apps;
mod browser_import;
mod capability;
mod cloud_relay;
mod config_capabilities;
mod control_plane;
mod created_resource;
mod daemon;
mod dictation;
mod event_publication;
mod external_provider_session;
mod file_transfer;
mod history;
mod kernel_access;
mod kernel_browser;
mod kernel_computer;
mod kernel_files;
mod kernel_gateway;
mod kernel_resources;
mod managed_context;
mod metaagent;
mod notes;
mod outside_events;
mod own_machine_enrollment;
mod project_environment;
mod prompt_control;
mod prompt_settings;
mod provider_control;
mod remote_access;
mod request;
mod resource_telemetry;
mod response;
mod room_environment;
mod room_terminal;
mod runtime_identity;
mod screenshot;
mod session_control;
mod slice;
mod terminal_command_catalog;
mod terminal_interaction;
mod usage_snapshot;
mod user_domain_access;
mod vault_secret;
mod waiting_room;
mod workflow;
mod workflow_notification;
mod workspace;

pub use account_transfer::*;
pub use agent_deadline::*;
pub use agent_lifecycle::*;
pub use agent_peek::*;
pub use agent_tree::*;
pub use agent_triggers::*;
pub use agent_utility::*;
pub use apps::*;
pub use browser_import::*;
pub use capability::*;
pub use cloud_relay::*;
pub use config_capabilities::*;
pub use control_plane::*;
pub use created_resource::*;
pub use daemon::*;
pub use dictation::*;
pub use event_publication::*;
pub use external_provider_session::*;
pub use file_transfer::*;
pub use history::*;
pub use kernel_access::*;
pub use kernel_browser::*;
pub use kernel_computer::*;
pub use kernel_files::*;
pub use kernel_gateway::*;
pub use kernel_resources::*;
pub use managed_context::*;
pub use metaagent::*;
pub use notes::*;
pub use outside_events::*;
pub use own_machine_enrollment::*;
pub use project_environment::*;
pub use prompt_control::*;
pub use prompt_settings::*;
pub use provider_control::*;
pub use remote_access::*;
pub use request::*;
pub use resource_telemetry::*;
pub use response::*;
pub use room_environment::*;
pub use room_terminal::*;
pub use runtime_identity::*;
pub use screenshot::*;
pub use session_control::*;
pub use slice::*;
pub use terminal_command_catalog::*;
pub use terminal_interaction::*;
pub use usage_snapshot::*;
pub use user_domain_access::*;
pub use vault_secret::*;
pub use waiting_room::*;
pub use workflow::*;
pub use workflow_notification::*;
pub use workspace::*;

/// Version 319 adds Git credential enrollment for an existing managed environment.
/// Version 320 adds encrypted browser-cookie delivery to a consent-bound Environment.
/// Version 321 adds admitted private browser-cookie delivery and durable recovery.
/// Version 322 makes Selkies the omitted backend for new headed slices.
/// Version 323 projects bounded provider-run termination metadata with failed turns.
/// Version 325 preserves failed turn lifecycle in retained history outlines.
/// Version 326 adds kernel-owned project environment setup and readiness operations.
/// Version 327 carries the source-selected managed-context plan required for
/// confirmed disposable-worker imports.
/// Version 328 carries the home-authoritative setup attempt through relay
/// recovery and the authenticated worker setup boundary.
/// Version 329 is reserved by the integrated structured relay retryability
/// contract.
/// Version 330 carries home-authoritative managed browser/profile inventory.
/// Version 331 carries recipe and lockfile input attestations for worker setup.
/// Version 332 carries the public camelCase managed-context transfer receipt.
/// Version 333 carries definition-derived executable path entries for worker setup.
/// Version 334 carries kernel-authoritative managed-target resource telemetry.
/// Version 335 adds measured CPU utilization and its sample window to that telemetry.
/// Version 336 adds kernel-verified installed release identity to resource telemetry.
/// Version 340 coordinates the signed managed-activity transition-time contract.
/// Version 342 carries the create-selected trusted managed repository root and
/// returns the persisted root in managed-environment summaries.
/// Version 343 carries the originating home prompt in forwarded worker tool context.
/// Version 344 carries the home-owned Room browser capability in provider-run responses.
/// Version 345 carries provider/Cloud owner-bound correlation metadata.
/// Version 346 adds an explicit fresh relay kernel inventory observation.
/// Version 347 requires home persistence acknowledgment before leased setup validation.
/// Version 348 projects owner-authorized managed activity and operation history.
/// Version 349 binds CLI relay and display requests to a persisted terminal key.
/// Version 350 exposes paired writable-layer and persistent-home slice caps in user config and
/// coordinates the signed managed quiescence HTTP contract.
/// Version 351 adds home-bound disposable Cloud worker controls and managed keep-running.
/// Versions 344–365 also exist on a divergent Apps branch and do not imply these controls.
/// Version 366 adds explicit kernel control capabilities to RelayStatus.
/// Version 367 adds bounded, explicitly negotiated Unix guarded-control sessions.
/// On the Apps branch, versions 344-367 carried the Apps contracts instead; a
/// kernel at version 410 or later implements both lines:
/// Version 344 merges the Chariox Apps contracts (branch protocols 288-297:
/// App installation, upload, bindings, events, HTTP/Fetch, kernel operation
/// approvals and publisher enrollment) onto the main protocol line.
/// Version 345 adds owner-scoped App worker control (status/start/stop/restart)
/// and App automation list/configure/disable requests.
/// Version 346 opens App views as managed Room Tabs (`OpenAppView`) and adds
/// the room-controller `app_view` command.
/// Version 347 adds owner-scoped `UninstallApp` at an expected generation.
/// Version 348 adds `GetAppLogs` for the App's own `log.write` entries.
/// Version 349 adds `BeginAppUpdate`, a local release replacement.
/// Version 350 binds a foreground App to the focus agent: `AppViewOpened`
/// gains `bound_agent_id`.
/// Version 351 marks App view Tabs in the Room snapshot: `EnvironmentTab.app`
/// with the installation and the reserved private conversation panel.
/// Version 352 adds `CreateAgentWorkflow`: a visible one-node workflow for an
/// agent that gets a trigger or deployment, with `WorkflowDefinition.origin`.
/// Version 353 adds the installation inbox: `CreateAppInboxRoute`,
/// `RemoveAppInboxRoute`, `ListAppInboxRoutes` and `TestAppInboxRoute`.
/// Version 354 adds user-selected file grants: `GrantAppFile` answers an App's
/// `host.pick_file` request with the chosen files (`AppFileGranted`).
/// Version 355 adds `SaveAppFileExport`: the owner takes a copy of a file an
/// App offered with `files.export` (`AppFileExport`).
/// Version 356 reconnects App views: the room-controller `app_view` command
/// gains `reload`, which serves an open view the current generation's assets.
/// Version 357 adds `GetRoomEnvironmentTabAccessibility`: a Room Tab's
/// accessibility outline for terminals (`RoomEnvironmentTabAccessibility`).
/// Version 358 lets an App inbox route subscribe to an event generator
/// connection (`CreateAppInboxRoute.connection`, `AppInboxRouteSummary.connection`).
/// Version 359 lets an owner grant an App one of their event generator
/// connections (`GrantAppConnection`, `RevokeAppConnection`,
/// `ListAppConnections` → `AppConnections`); the App acts through it only with
/// the actions its signed `capabilities.connections` declares.
/// Version 360 adds `MoveEventBindingToApp`: a workflow event binding becomes
/// an App inbox route on the same generator connection
/// (`EventBindingMovedToApp`).
/// Version 361 adds `GetAppSet`: the owner's App set, a versioned description
/// of each active installation's release, approved capabilities and
/// configuration (`AppSet`).
/// Version 362 adds an optional `filter` to event generator resources: the
/// filter that narrows a binding to a resource sharing its connection scope.
/// Version 363 adds `delete_data` to `UninstallApp` and `data_kept` to App
/// installation summaries: an uninstalled installation keeps its data for a
/// reinstall unless the owner deletes it.
/// Version 364 removes the workflow event reply surface: event bindings, their
/// create request and publication binding templates lose the reply mode, the
/// relay workflow turn context loses its reply capability, and the reply
/// runtime tool is gone. Workflows cannot post `notification.reply` at all:
/// bindings refuse to enable it and `event_action` refuses it; replies go
/// through the generator's App.
/// Version 365 retires direct workflow event bindings: the binding requests
/// and responses, `MoveEventBindingToApp` and the `event_context` /
/// `event_action` runtime tools are gone. Events reach workflows only through
/// App inbox routes and App automations. Connection dependencies
/// (`EventConnectionDependency`), `attached_trigger_count` and the event
/// delivery status count App routes and grants; `EventConnectionRemoved` loses
/// `deactivated_bindings`. Relay peer protocol 69 drops the same flags.
/// Version 366 pins a workflow publication's App plan
/// (`WorkflowPublicationDefinition.apps`, `chariox.publication-apps.v1`) at the
/// first deployment preparation (a client `ExportWorkflowPublicationPackage`):
/// the Apps granted to its agents or feeding it through App automations, with
/// release, signer, capabilities digest and configuration. App-bound workflows
/// export `apps.json` and `capabilities.apps` in the deployment contract;
/// requirements follow the publication snapshot's agents.
/// Version 367 adds `PreviewDeploymentApps` (`DeploymentAppsPreview`: a
/// publication's App plan with capabilities, before any export) and
/// `PrepareDeploymentApps` (`DeploymentAppsConsent`): one
/// kernel prompt asks the owner to deploy a workflow together with the Apps of
/// its pinned App plan and to share their connections with the copy; the
/// answer is recorded durably. A deployment copy's install of exactly a
/// consented release, whose capabilities the owner approved interactively
/// before, is approved by the `kernel_deployment_consent:<interaction>`
/// policy. Copy installations are tagged with their deployment: absent from
/// `ListAppInstallations`, marked by `AppSetInstallation.deployment_id`.
/// Version 368 adds Cloud-coordinated managed release update requests.
/// Version 369 coordinates canonical signed relay daemon identity admission.
/// Version 370 drains pending terminal output and projects managed runtime start observations.
/// Version 371 adds native process identity to authenticated product status and
/// the value-free Project environment layer and inputs (MP-08).
/// Version 372 adds session-bound post-launch Project Environment adjustment (MP-08).
/// Version 373 adds worker Environment queries and waiting-room pending review visibility (MP-08).
/// Version 374 binds Computer credential approval to a native display target (MP-08).
/// Version 375 normalizes primitive MCP script results to object structured content (MP-08/MP-10).
/// Version 376 adds receiving-kernel provider login interaction projection and
/// restores single terminal workflow event delivery (MP-08/MP-10/MP-11).
/// Version 377 makes App plans per release: each owner export packages the
/// owner's current App set and records that plan by package digest
/// (`WorkflowPublicationDefinition.release_app_plans`; `apps` is the latest).
/// Bind, recovery and rollback use the release's own plan, and a release with
/// exactly the App releases the owner approved before for the deployment is
/// consented without asking again. `PreviewDeploymentApps.package_digest`
/// also returns that release's plan (`DeploymentAppsPreview.release_plan`).
/// Version 379 adds the Room browser bar: `SetRoomBrowserBar` and the
/// snapshot's `browser_bar_visible` (a change emits `TabsChanged`).
/// Version 380 lets Apps place their agent panel (`ui.agentPanel`,
/// `chariox.panel` from the page) and the user move or minimize it
/// (`SetAppViewPanel`); `EnvironmentAppPanel` carries `placement` and
/// `minimized`, and an App that shows no panel has none.
/// Version 381 adds the install operation phase `queued`: approved and waiting
/// to start (for example for a free App worker slot), which earlier kernels
/// reported as `awaiting_approval`, then as `starting`.
/// Version 393 adds `AgentInstance.failed_requests` (`{prompt_id, excerpt,
/// reason}`, omitted when empty): requests whose turn failed before completing
/// are dropped. The failed turn gets a "Request not carried out" provider-error
/// transcript entry, and the agent's next turn delivered to its provider
/// carries a one-time hidden note not to act on them unless the user asks
/// again; the field clears once a provider accepted that turn.
/// Version 394 adds `RevokeAppFileGrants` (`AppFileGrantsRevoked`): the owner
/// ends an installation's file requests and the grants its App has not
/// imported, all of them or one request's.
/// Version 397 makes agent substitutes per-turn only: a turn that fails with a
/// provider error is rerun on the next substitute in order, and the next turn
/// starts on the agent's configured profile again. `AgentSubstituteAction`
/// loses `Activate` and `Primary`; `AgentInstance` loses `primary_provider`,
/// `primary_model`, `primary_effort`, `primary_account_profile`,
/// `active_substitute_index` and `last_substitution`. An agent persisted on a
/// substitute loads on its primary profile.
/// Version 407 binds native approvals to the originating turn and provider run.
/// Version 407 exposes `AppWorkerPhase::Quarantined` after the supervisor's
/// restart limit is exhausted. Recovery uses the existing explicit start action.
/// Version 409 adds owner-mediated App clipboard and link acceptance.
/// Version 410 adds owner-scoped named saved App data snapshot restore.
/// Version 411 combines Apps Phase 1 and Browser/Computer G2 with secret observation masks.
/// Version 402 adds KA connection classes and critical-approval audit attribution.
/// Version 403 adds shared owner passkey popup projection.
/// Version 404 adds process-bound external access grants and lifetime configuration.
/// Version 412 enforces local credentials and admitted terminal authority.
/// Version 413 adds terminal sudo turns and critical approval receipts.
/// Version 415 adds external sudo requests and requester attribution.
/// Version 416 refuses evicted App control replays with `receipt_expired`.
/// Version 435 combines Apps, Browser artifacts, bounded Computer holds, and
/// the allowlisted public provider-run DTO (MP-08/MP-10/MP-11).
/// MP-08/MP-10/MP-11: version 443 combines main 435 with the multidomain
/// host browser, App views, Notes, grants, captures and DOM mirror contract.
/// MP-08/MP-11: reserved version446 adds owned Linux desktop targets and input.
/// MP-08/MP-10/MP-11: version 447 adds bounded stripe display to that contract.
/// MP-08 / MP-10 / MP-11: Room AT-SPI and per-agent Computer revoke.
/// MP-08 / MP-11: version 461 adds bulk Room Computer restore (`agent_id: null`).
/// MP-08/MP-10: version 466 makes display frame events binary (raw payload
/// segments) and adds lossless WebP tiles and scroll move rectangles.
/// MP-08/MP-10/MP-11: version 474 adds the protected desktop display subscription.
/// MP-08/MP-10: version 475 adds pushed display (`display_ack` drives the
/// kernel push pump of a relay display subscription; `display_next` remains).
/// Version 450 adds immutable room spawn/object creators and regular room tools
/// behind the transitional room-agent-tools flag (MP-08/MP-10/MP-11 A01).
/// Version 462 adds user-requested capability grant causes, absolute grant
/// expiry and the `not_requested` refusal (MP-08/MP-10/MP-11 A05).
/// Version 452 adds durable agent events, enforced turn dispositions and
/// kernel-emitted workflow notifications (MP-08/09/10/11 A02).
/// Version 459 adds kernel-owned timer/process wakes with proof-of-life and
/// delivery receipts on `RuntimeSession.agent_wakes` (MP-08/09/10/11 A03).
/// Version 451 grants the whole local kernel and routes access popups without sessions.
/// Version 470 adds structured OS requester identity to access decisions.
/// Version 472 advertises identity-preserving terminal relay renewal with
/// explicit capability negotiation and recoverable target-offline handshakes.
/// Version 484 replaces one-turn sudo with hour-scale windows bound to owner
/// work, fresh-passkey extension, session status and no agent approvals
/// (MP-08/MP-10/MP-11 A04).
/// Version 487 adds value-free Project Environment aggregates and reserves the
/// complete Environment operation contract on top of 472 (developed as 471;
/// MP-02/MP-03/MP-08/MP-10/MP-11).
/// Version 493 reconciles the agent-model union, task/wake projections and leased watchers (MP-08/MP-09/MP-10/MP-11).
/// MP-08/MP-10/MP-11: version 447 added bounded stripe display, App views,
/// Notes, grants, captures and DOM mirrors; 466 added binary display frames,
/// WebP tiles and scroll rectangles; 475 added relay display push/ACKs.
/// Version 490 added bounded relay response framing and socket-local receipts.
/// Version 491 combined native display and agent-tab provenance/activity.
/// Main's version 491 (#974) adds credential-free Cloud kernel ownership
/// projections, receiving CLIENT key-bound pivots, Cloud-free terminal pairing
/// and provider account portability preflight, and requires a distinct relay
/// subscription identity for every fresh encrypted binding while retaining the
/// logical session replay cursor. It also restores workflow notifications, Room
/// workflow inventory and controls, opt-in Cloud device denial and exact
/// home-prompt cancellation admission (MP-08/MP-10/MP-11).
/// Version 506 integrates those contracts, including main's 491, with the 493
/// agent-model union; generated consumers follow this coordinator-allocated
/// acceptance version.
/// Version 511 orders a DOM mirror read behind its viewer's earlier mirror input,
/// so a client sends the read with the key and gets its echo in one round trip
/// (MP-08/MP-10/MP-11).
/// Version 517 combines App Tab calls/grants and ordered reads with DOM mirror v2
/// author CSS, live controls, compact deltas, native density and long-poll credits.
/// MP-08/MP-10/MP-11: allocated 525 carries shared producer resumption on
/// the display integration train (519 region clients precede this final step).
/// MP-08/MP-10/MP-11: allocated 528 adds the Browser state's `chrome_window` (the
/// native band's window epoch and active Tab) and `browser_lockdown` fields.
/// MP-08/MP-10/MP-11: allocated 529 adds page display region_offset placement.
/// Allocated 530 adds band middle/right click and drag (`KernelBrowserInput`), the
/// `browser_auxiliary` display target and the state's `chrome_window.auxiliary`.
/// MP-08/MP-10/MP-11: allocated 532 bounds multiplexed opaque relay transfers.
/// Allocated 535 registers525 resumption, worker attach and initial push credit once.
/// MP-08/MP-10/MP-11: allocated536 captures only the visible page on the owned desktop.
/// MP-08/MP-10/MP-11: allocated 540 keeps the omnibox list's `chrome_window.auxiliary`
/// entry (visible height 0) while the address bar has the person's keys.
/// MP-08/MP-10/MP-11: allocated 550 versions the mirror v2 wire extensions (navigation preview,
/// font requirements, known sheets, flush, snapshot completion); clients require it for mirror v2.
/// MP-08/MP-10/MP-11: allocated 552 adds the Browser reply's `parked_input` (input this observer has
/// parked behind a busy renderer); the viewer holds its preview handover until it is admitted.
/// MP-08/MP-10/MP-11: allocated 556 adds `KernelBrowser` `state_wait`: a parked state read
/// replacing the viewer's 250 ms poll.
/// MP-08/MP-10/MP-11: allocated 567 adds the mirror v2 `fonts` op's `absent` (stack families the native machine lacks).
/// MP-08/MP-10/MP-11: allocated 596 reads a session state's revision before the state, so a state is never
/// older than its `agent_activity_revision`, and lets `AttachSessionView` ask for `latest_prompt_count` turns.
/// Allocated 594 adds `Dictation`: kernel speech-to-text with an on-demand model download.
/// Allocated 603 adds speech-service selection, owner Vault metadata, setup state and recording context.
/// Allocated 610 unions the staging features 588-603 with the new-UI contracts.
/// Agent-model line (537, relay peer 145) contracts were integrated in 570/178:
/// task/wake, completion, leased-watcher, paired-display and historical-workflow projections, owner-only 520/128 direct carriers
/// and owner-managed contexts.
/// Room terminal service (`RoomTerminal`, formerly provisional 569) is retained.
/// The event-core line (561, relay peer 169) is merged too: task/wake driver sources (`AgentWake.summary`/`detail`) and the
/// `agent_task_updated` delta event.
/// Allocated 573/181 binds leased assessment wire identities to the home check.
/// Allocated 585 adds outside event subscriptions and executor-local agent deadlines.
/// Allocated 591 adds the outside `steer` event.
/// Allocated 609 adds explicit same-owner Vault-backed provider account copy.
/// Allocated 614 unions the staging 610 line with the external agents 609 line.
/// MP-08/MP-10/MP-11: allocated 629 adds the shared browser viewport resize mutation, the mirror v2 viewport-first
/// snapshot (`viewport_first`, `deferred` records, `snapshot_viewport_complete`) and `:visited` links (`visited_links`).
/// Protocol 629 also includes protocol 627 attachment byte sizes in shared history.
/// MP-08/MP-09/MP-10/MP-11: landing union 636 carries 630 (shared browser dialogs), 631 (grant-scoped kernel
/// health events), 632 (binary Room exec) and 636 (archived-agent discovery); every earlier contract is retained.
/// Protocol 641 adds `CoverKernelAccessProcess`: a grant covers a named background process by identity.
/// Recipient account offers and kernel-owned copy consent.
/// MP-08 / MP-09 / MP-10 / MP-11: 780 persists semantic session activity; public snapshot schema 14.
/// MP-08/MP-10/MP-11: 720 adds the Tab display (display_subscribe tab_display, display_park; P1 of
/// docs/BROWSER_LATENCY_GAP_PLAN.md).
/// MP-08/MP-09/MP-10/MP-11: allocated 811 adds structured provider login retry feedback.
/// MP-08/MP-09/MP-10/MP-11: landing union 642 carries 630 (shared browser dialogs), 631 (grant-scoped kernel
/// health events), 632 (binary Room exec), 636 (archived-agent discovery), 640 (the typed cause, retryable flag and
/// reset time of a failed turn on its outside event and history) and 642 (the turn outcome: one typed `kind`, `retryable`
/// and `reason` for every turn that ends, with the prompt and turn id on the outside event); every earlier contract is retained.
/// Protocol 643 answers `CancelActivePrompt` for a parked task with `AgentTaskUpdated`; 506 leaves the predecessor list to
/// keep it within the 32-entry policy.
/// Allocated 637 adds the optional `native_checkpoint_binding` on provider launch requests (native provider checkpoints).
/// Allocated 638/240 adds durable agent Move and Return: the `AgentMove` peer carrier, `MoveAgentToLocal` and its
/// `abort_pending` cancellation, and Project materialization on the target.
/// Allocated 639/241 composes a Move onto a target kernel with native Project setup: resource receipts, probe gating and
/// the kernel-held deferred Move published on the agent (`agent_move`).
/// MP-08/MP-09/MP-10/MP-11: 721 pairs bounded raw binary relay chunks with relay capability 97.
/// MP-08 / MP-09 / MP-10 / MP-11: 780 persists semantic session activity (public snapshot schema 14).
/// MP-08/MP-09/MP-10/MP-11: allocated 910 adds the optional `images` data-URL list and `images_omitted` count to the
/// provider tool payload (Claude tool results), and Codex web search and image view tool calls with their lifecycle status.
/// Allocated 880 adds owner-scoped listing and replay of the webhooks an integration received
/// (`ListEventConnectionEvents`, `ReplayEventConnectionEvent`); the predecessor list keeps only the
/// current release (636) and the one before it (629).
/// MP-08 / MP-09 / MP-10 / MP-11: 800 adds process limits, Health observations and owned process controls.
/// Request timing distributions and bounded sampled health diagnostics.
/// MP-08/MP-09/MP-10/MP-11: landing union 911 carries 910 with 880 (integration received events), 800
/// (session process samples and controls, `memory_limit` and `outside_end` terminations) and 890 (daemon
/// health request timings and durable writer queue).
/// Allocated 913 adds `processes` to the `SessionState` response: the same rows the live
/// `session_processes_changed` feed shows, readable by a one-shot request.
/// Version 919 removes the user-side browser frame stream: `KernelBrowserCommand::Subscribe` and `Poll`
/// (and `KernelBrowserDisplayRequest::Subscribe`) no longer parse, so a client that sends them is refused; `DisplayAck`
/// gains `paused` (nobody watches: the push pump goes dormant, the acknowledgement still renews the lease).
/// MP-08/MP-10/MP-11: allocated 920 carries the page's document mode on mirror resets (`document_mode`, the viewer
/// frame lays out in the page's mode) and adds `KernelBrowser` `pointer_cursor`, the CSS cursor at a point for a
/// viewer that shows the page as video.
/// Version 923 adds `UnlockCredentialVault`: the owner's own client unlocks the kernel's
/// existing Vault with no session or agent (`CredentialVaultUnlocked`).
/// Version 929 adds `RequestVaultSecret` and `GetVaultSecretStatus` for an outside agent that needs a Vault value
/// the owner types in a connected client or on a link's page, and `PasskeyPromptKind::VaultSecret` for that popup.
/// Landing union 923 carries 913 (`SessionState` `processes`), 917 (direct browser sessions over WebRTC),
/// 919 (user-side browser frame stream removed, `display_ack` `paused`), 920 (mirror `document_mode`,
/// `pointer_cursor`) and 923 (session-less Vault unlock).
/// MP-08/MP-09/MP-10/MP-11: 925 marks a hot reload's relay close (`kernel_reload`), which the relay
/// forwards to the kernel's subscribed clients so they retry within the swap.
/// Version 926 makes the direct data channel's liveness a contract: the kernel pings
/// every second ahead of queued frames and ends a peer silent for 3 s; browsers leave
/// a direct path silent for 2.5 s (TR-16).
/// Version 927 adds admission: `SessionProcess.state` (`running`, `held` or `paused`, with the
/// reason, place in line, need and free room) and `StartSessionProcessNow` (`SessionProcessStartedNow`).
/// Allocated 928 adds `question` (the decision as the person sees it) to a pending outside `Interaction`
/// event and `resolution` (`via`, `choice_id`) to a resolved one; an answer to a closed decision fails with
/// the stable `interaction_closed`.
/// Allocated 930 adds agent triggers; `/wait-in` and `/wait-every` become schedule triggers.
/// Allocated 931 adds trigger deliveries the person can resend and removes the `/wait` schedule shape.
/// Allocated 932 adds the HTTP trigger source and `InvokeAgentTrigger`/`ReadAgentTriggerTranscript`.
/// Landing union 932 carries 925 (hot reload `kernel_reload`), 926 (direct
/// data channel liveness), 927 (process admission), 928 (outside interaction `question`/`resolution`), 929 (Vault
/// secret requests) and 930 to 932 (agent triggers).
/// Browser header 933 adds the portable active Tab, history bounds and bounded favicon PNGs to the
/// `KernelBrowser` state (`active_tab_id`, `can_go_back`, `can_go_forward`, `favicon_png_base64`).
/// MP-08/MP-10/MP-11: 934 acknowledges the leased projection drain (peer 383).
/// MP-08/MP-10/MP-11: 935 binds the leased prompt completion to its exact prompt and worker run (peer 384).
/// MP-08/MP-10/MP-11: 936 adds non-blocking task-owned questions (`asking_task_id` on interactions, answered
/// through `RespondToInteraction`; peer 385).
/// Browser downloads 937 adds bounded Room artifacts and cancellation.
/// MP-08/MP-10/MP-11: 938 lets the kernel pick an access holder from the peer's host ancestry, own
/// `--no-wait` requests and report one holder's status (`GetKernelAccessStatus`; peer 387).
/// MP-08/MP-10/MP-11: 941 adds person Vault fill and audience-specific display masking (peer 390).
/// MP-08/MP-09/MP-10/MP-11: 943 adds the `env` and `file` credential uses and injection kinds (Vault capture/use by command and file).
/// Resource register 944 adds project-scoped record/list/revoke tools and metadata.
/// Vault TOTP 945 adds computed factor input and sibling backup-code credentials.
/// VAUDIT S3 948 adds the kernel-browser Vault capture_browser and fill_field tools.
/// Protocol 950 advertises the answering kernel protocol in RelayStatus.
/// MP-08 / MP-09 / MP-10 / MP-11: 954 adds nullable cumulative process disk I/O (peer403).
/// MP-08/MP-09/MP-10/MP-11: 960 integrates Machines and canonical owner/agent Vault uses (peer409).
/// MP-08/MP-09/MP-10/MP-11: 963 binds every captured Vault entry to one declared use (peer412).
/// Connected service machine requests use the ordinary Chariox access grant.
/// MP-08/MP-09/MP-10/MP-11: 965 integrates declared Vault uses and enrolled-kernel machine authority (peer414).
/// 971 replaces AddSshMachine/RemoveSshMachine with IssueOwnMachineEnrollment (peer420).
/// MP-08/MP-09/MP-10/MP-11: 966 integrates WASM Apps with Landing965 (peer415).
/// Environments 972 replace the Project Environment requests with one saved record per Project,
/// its presence on this kernel and Setup runs (peer421 drops the leased Project environment requests).
/// Environments 973 add Import from another kernel of the same user and Draft with agent (peer422).
/// Protocol 990 binds Environment file chunks to their complete content digest (peer 429).
/// MP-08 / MP-09 / MP-10 / MP-11: 996 carries bounded lossless peer snapshots.
/// Protocol 987 adds exact owner cancellation of retained agent tasks and timers.
/// MP-08/MP-09/MP-10/MP-11: 986 adds the person-only kernel browser `show_tab` op (peer415).
/// OSS boundary: 967 removes the 25 retired Cloud controls (human session invites and collaborators, relay pairing,
/// managed-environment and disposable-worker requests) and their responses; peer 416 pairs the embedded union.
/// MP-08/MP-10: 981 names a quirks-mode frame document in its mirror v2 record (`compat`), so the viewer lays frames out in their source's mode.
/// Allocated 983 gives a waiting command one row: `SessionProcessState` `held` and `paused` carry the
/// same fields (reason, place in line, need, free, command line, folder, waiting since), and an
/// outside `process` event carries that row (peer424).
/// Allocated 985 adds what a process holds on disk to its row: `disk_written_bytes`,
/// `deleted_open_bytes` (deleted files it keeps open) and `outlived` (it kept running after the
/// command that started it ended). The peer protocol is unchanged (424).
/// Allocated 991 carries current disk occupancy in the existing process feed.
/// MP-08/MP-09/MP-10/MP-11: 974 adds the agent reach identity and "ask before agents acquire machines"; 966 integrated WASM Apps (peer415).
/// MP-08 / MP-09 / MP-10 / MP-11: 989 integrates working-agent activity with reach.
/// Vault unlock 984 makes the owner pick a duration on every unlock (`UnlockCredentialVault.duration`,
/// the unlock options in `CredentialVaultStatus`, a duration on Vault-unlock choices; peer425).
/// MP-08/MP-09/MP-10/MP-11: 992 adds positive browser-card verification conditions and non-secret receipts (peer431).
/// MP-08/MP-09/MP-10/MP-11: 993 keeps lifecycle capabilities discoverable without an acquisition offer (peer432).
/// Environments 982 hold an operation read until it changes, add MCPs and Skills that project files declare, and Check without a model (peer423 carries them in the record).
/// MP-08/MP-09/MP-10/MP-11: 994 adds per-exit workflow notification sources, agent notification triggers and drops subscription options (peer422).
/// MP-08/MP-09/MP-10/MP-11: 998 adds source-side listeners and durable queued notification expiry (peer 437).
/// MP-08/MP-09/MP-10/MP-11: 1000 lets an outside agent's grant follow notification sources and read their occurrences as events,
/// lets an owner-managed copy select event connection records, adds the parent edge with SetAgentParent, the spawn key,
/// child_finished and the inbox sender relation, the sub-agent tree read and its change event, and carries Vault
/// provider logins to a new managed machine (peer 439).
/// MP-08/MP-09/MP-10/MP-11: 1001 carries peers across rooms and kernels of one owner (spawn --room, messages, steer,
/// cancel and follow, `peer_event`), the `agent_tree_changed` event and the `kernel_access_sub_agent` refusal (peer 440).
/// Temporary 1002 (landing 7): workflow entry gates (`WorkflowGate`, gated entry acceptance, typed event
/// data), `exec_place` with `placeable_on` on held launches, and a sub-agent tree node's mode, permission
/// level and effort.
/// 1003 (Landing 8): `HideAgent`/`AgentHidden` and the agents' `hidden` flag replace the agent archive;
/// machines have no aliases (`RenameRemoteMachine` is gone); a notification trigger's summary carries
/// `source_available`; an agent's activity carries `liveness` and health lists `silent_agents`; an App
/// install that waits for its owner is a passkey prompt (`app_install`); App summaries carry
/// `display_name` and `icon`; the Vault reports `awaiting_passphrase`; a gateway request and its answer
/// may carry bytes as bytes (`framed_plaintext_v1`).
/// 1005 (Landing 9): Files on a machine are listed with a revision and a mode per entry and in pages
/// (`ListKernelDirectory.offset`), never reach the kernel's own state or Vault, and a file or folder is
/// copied or moved between the owner's machines by a kernel-owned transfer job
/// (`StartFileTransfer`, `GetFileTransfer`, `ListFileTransfers`, `CancelFileTransfer`). Room setup
/// (`CreateSession.setup`, `RuntimeSession.room_setup`, `RetryRoomSetup`, `DismissRoomSetup`),
/// `SuggestSessionAlias`, and `git_available` on the launch target. A yield without a deadline
/// reports `wait.deadline_ms` as `null`. A sub-agent tree node carries its `account_profile`,
/// `workspace_id`, `worktree_id` and `worker_kernel_id`, and an agent's wait is a `provider_tool` row of its
/// transcript.
/// 1027: generic runtime identity observation and opaque enrollment authority evidence.
/// 1006: the kernel resource feed (`kernel_resources` subscription, `WatchKernelResources`,
/// `KernelResourcesWatched`, `kernel_resource_changed`) for the Vault, provider accounts and Apps.
/// 1006 (Landing 10, provisional): the owner ends a wait with `CancelAgentWait` (the agent is not woken), an agent's
/// `waiting` state is projected on its activity, and a turn the kernel started is marked `system_prompt` in the outline.
/// 1006 (person cards, provisional): an active interaction carries `asker_name`, the asking agent's
/// alias (else its ref), and a sub-agent's card is shown in its top parent's panel whoever holds it.
/// 1009 (Landing 10 peek, provisional): `PeekAgentTranscript` lists an agent's latest transcript rows as
/// titles (`AgentTranscriptPeek`) with a `before` cursor and `ReadAgentTranscriptEntry` reads one row by
/// pages (`AgentTranscriptEntryPage`); a wait without a deadline gets `now + 30 min` as `deadline_ms`.
pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 1027;
