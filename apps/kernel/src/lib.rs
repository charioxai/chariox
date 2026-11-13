#![recursion_limit = "256"]

pub mod account_profile;
pub mod agent;
pub(crate) mod agent_file_copy;
pub mod agent_move;
pub(crate) mod agent_temp;
pub mod app;
pub mod artifacts;
pub mod attachment;
pub(crate) mod blocking_io;
pub mod capability;
pub mod config;
pub mod connector;
pub mod credential;
pub(crate) mod debug_trace;
pub(crate) mod dictation;
pub(crate) mod disk_scan;
pub(crate) mod durable_prompt_state;
pub(crate) mod durable_snapshot;
pub mod durable_state;
#[cfg(test)]
pub(crate) mod env_lock;
pub mod error;
pub(crate) mod event_connection;
pub mod execution_lease;
pub mod extension;
pub(crate) mod git_observer;
pub(crate) mod git_worktree_placement;
pub mod history;
pub mod history_archive;
#[cfg(unix)]
pub mod hot_reload;
pub mod io;
mod lease_worker_enrollment;
pub(crate) mod jobs;
pub mod local;
pub mod logging;
pub(crate) mod machine_files;
pub mod managed_bootstrap;
pub mod managed_context;
pub mod mcp;
pub(crate) mod outbound_dial;
pub mod owner_managed_bootstrap;
pub(crate) mod process_spawn;
pub mod project_environment;
pub(crate) mod prompt_assembly;
pub mod prompt_notes;
pub mod prompt_transcript;
pub mod provider;
pub mod provider_account_copy;
mod provider_discovery;
pub(crate) mod provider_output_policy;
pub mod provider_vault_login;
pub mod pty;
mod publication_provider_accounts;
pub mod room_exec;
pub mod room_terminal;
pub mod runtime;
pub mod runtime_transport;
pub mod scheduler;
pub mod script;
pub mod secret;
pub(crate) mod secret_redaction;
pub mod session;
pub mod session_history_page;
pub mod shutdown;
pub mod skill;
pub mod slice;
pub mod slice_provider_auth;
pub mod terminal;
pub mod transport;
pub mod web_bot_auth;
pub mod workflow_code;
pub(crate) mod workflow_publication_apps;
pub(crate) mod workflow_publication_requirements;
pub(crate) mod workspace_live_sync_ignore;
pub(crate) mod workspace_live_sync_journal;

pub use app::DaemonApp;
pub use config::DaemonConfig;
pub use error::DaemonError;

#[cfg(test)]
mod lib_tests;
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(all(feature = "native-display", target_os = "linux"))]
pub mod display_native;
