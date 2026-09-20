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
mod event_publication;
mod external_provider_session;
mod history;
mod managed_context;
mod managed_environment;
mod metaagent;
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
pub use event_publication::*;
pub use external_provider_session::*;
pub use history::*;
pub use managed_context::*;
pub use managed_environment::*;
pub use metaagent::*;
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
pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 335;
