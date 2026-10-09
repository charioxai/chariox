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

mod agent_lifecycle;
mod agent_prompt_schedule;
mod agent_utility;
mod apps;
mod browser_import;
mod capability;
mod cloud_relay;
mod config_capabilities;
mod daemon;
mod disposable_worker;
mod event_publication;
mod external_provider_session;
mod history;
mod kernel_access;
mod managed_context;
mod managed_environment;
mod metaagent;
mod project_environment_manifest;
mod project_environment_setup;
mod prompt_control;
mod prompt_settings;
mod provider_control;
mod remote_access;
mod request;
mod resource_telemetry;
mod response;
mod room_environment;
mod session_control;
mod slice;
mod terminal_command_catalog;
mod terminal_interaction;
mod waiting_room;
mod workflow;
mod workflow_notification;
mod workspace;

pub use agent_lifecycle::*;
pub use agent_prompt_schedule::*;
pub use agent_utility::*;
pub use apps::*;
pub use browser_import::*;
pub use capability::*;
pub use cloud_relay::*;
pub use config_capabilities::*;
pub use daemon::*;
pub use disposable_worker::*;
pub use event_publication::*;
pub use external_provider_session::*;
pub use history::*;
pub use kernel_access::*;
pub use managed_context::*;
pub use managed_environment::*;
pub use metaagent::*;
pub use project_environment_manifest::*;
pub use project_environment_setup::*;
pub use prompt_control::*;
pub use prompt_settings::*;
pub use provider_control::*;
pub use remote_access::*;
pub use request::*;
pub use resource_telemetry::*;
pub use response::*;
pub use room_environment::*;
pub use session_control::*;
pub use slice::*;
pub use terminal_command_catalog::*;
pub use terminal_interaction::*;
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
/// Version 337 adds authenticated managed-environment reimage control and receipt evidence.
/// Version 338 adds deliberate, generation-bound pre-reimage observation reporting.
/// Version 339 requires a fresh managed-context selection for every reimage request.
/// Version 340 coordinates the signed managed-activity transition-time contract.
/// Version 341 adds the authenticated managed-environment reimage preflight read.
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
/// coordinates the signed managed auto-stop quiescence HTTP contract.
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
/// Version 405 binds native approvals to the originating turn and provider run.
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
/// Version 451 grants the whole local kernel and routes access popups without sessions.
/// Version 450 adds immutable room spawn/object creators and regular room tools
/// behind the transitional room-agent-tools flag (MP-08/MP-10/MP-11 A01).
/// Version 452 adds durable agent events, enforced turn dispositions and
/// kernel-emitted workflow notifications (MP-08/09/10/11 A02).
/// Version 459 adds kernel-owned timer/process wakes with proof-of-life and
/// delivery receipts on `RuntimeSession.agent_wakes` (MP-08/09/10/11 A03).
/// Version 460 replaces one-turn sudo with hour-scale windows bound to owner
/// work, fresh-passkey extension, session status and no agent approvals
/// (MP-08/MP-10/MP-11 A04).
/// Version 470 adds structured OS requester identity to access decisions.
/// Version 472 advertises identity-preserving terminal relay renewal with
/// explicit capability negotiation and recoverable target-offline handshakes.
/// The agent-model chain carries 472 provisionally on top of A01-A04 and the
/// A10 home-ordered leased wakes and both-end sudo window fences until the
/// coordinator allocates its next number (MP-08/09/10/11).
pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 472;
