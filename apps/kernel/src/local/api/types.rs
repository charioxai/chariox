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
mod browser_import;
mod capability;
mod cloud_relay;
mod config_capabilities;
mod daemon;
mod disposable_worker;
mod event_publication;
mod external_provider_session;
mod history;
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
mod workspace;

pub use agent_lifecycle::*;
pub use agent_prompt_schedule::*;
pub use agent_utility::*;
pub use browser_import::*;
pub use capability::*;
pub use cloud_relay::*;
pub use config_capabilities::*;
pub use daemon::*;
pub use disposable_worker::*;
pub use event_publication::*;
pub use external_provider_session::*;
pub use history::*;
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
/// Version 368 adds Cloud-coordinated managed release update requests.
/// Version 369 coordinates canonical signed relay daemon identity admission.
/// Version 370 drains pending terminal output and projects managed runtime start observations.
/// Version 371 adds native process identity to authenticated product status and
/// the value-free Project environment layer and inputs (MP-08).
/// Version 372 adds session-bound post-launch Project Environment adjustment (MP-08).
/// Version 373 adds worker Environment queries and waiting-room pending review visibility (MP-08).
/// Version 374 binds Computer credential approval to a native display target (MP-08).
/// Version 375 normalizes primitive MCP script results to object structured content (MP-08/MP-10).
/// Version 376 adds receiving-kernel provider login interaction projection (MP-08/MP-10/MP-11).
pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 376;
