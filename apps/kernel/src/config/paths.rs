use std::env;
use std::path::{Path, PathBuf};

use super::{persisted_daemon, DaemonConfig};

impl DaemonConfig {
    pub fn default_local_socket_path(daemon_id: &str) -> PathBuf {
        default_runtime_dir().join(format!("{daemon_id}.sock"))
    }

    pub fn default_session_history_root() -> PathBuf {
        default_config_dir().join("sessions")
    }

    pub fn operational_history_path(&self) -> PathBuf {
        if let Some(root) = &self.publication_control_state_root {
            return root.join("history").join("operational.db");
        }
        self.user_config
            .history
            .operational
            .path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_config_dir().join("history").join("operational.db"))
    }

    pub fn operational_artifact_root(&self) -> PathBuf {
        if let Some(root) = &self.publication_control_state_root {
            return root.join("artifacts");
        }
        self.user_config
            .artifacts
            .operational
            .root
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_state_dir().join("artifacts"))
    }

    pub fn operational_artifact_index_path(&self) -> PathBuf {
        if self.publication_control_state_root.is_some() {
            return self.operational_artifact_root().join("index.db");
        }
        self.user_config
            .artifacts
            .operational
            .index_path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| self.operational_artifact_root().join("index.db"))
    }

    pub fn durable_state_path(&self) -> PathBuf {
        if let Some(root) = &self.publication_control_state_root {
            return root.join("state.db");
        }
        self.user_config
            .state
            .path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| {
                default_config_dir()
                    .join("kernels")
                    .join(&self.daemon_id)
                    .join("state.db")
            })
    }

    pub fn account_profile_registry_path(&self) -> PathBuf {
        self.private_runtime_state_root()
            .join("provider-accounts.json")
    }

    pub fn private_runtime_state_root(&self) -> PathBuf {
        if self.publication_control_state_root.is_some() {
            return self
                .user_config_path
                .parent()
                .unwrap_or(Path::new("."))
                .join("kernels")
                .join(&self.daemon_id);
        }
        self.durable_state_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(default_config_dir)
    }

    pub fn session_history_root(&self) -> PathBuf {
        self.publication_control_state_root
            .as_ref()
            .map(|root| root.join("sessions"))
            .unwrap_or_else(|| self.session_history_root.clone())
    }

    pub fn workflow_runtime_artifact_root(&self) -> PathBuf {
        self.durable_state_path()
            .parent()
            .map(|root| root.join("workflow-runtime"))
            .unwrap_or_else(|| {
                default_state_dir()
                    .join("workflow-runtime")
                    .join(&self.daemon_id)
            })
    }

    pub fn workflow_code_artifact_root(&self) -> PathBuf {
        self.durable_state_path()
            .parent()
            .map(|root| root.join("workflow-code"))
            .unwrap_or_else(|| default_config_dir().join("workflow-code"))
    }

    pub fn workflow_registry_root(&self) -> PathBuf {
        self.durable_state_path()
            .parent()
            .map(|root| root.join("workflows"))
            .unwrap_or_else(|| default_config_dir().join("workflows"))
    }

    pub fn slice_root(&self) -> PathBuf {
        if let Some(root) = std::env::var_os("CHARIOX_SLICE_ROOT") {
            return PathBuf::from(root);
        }
        self.user_config
            .slices
            .root
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_config_dir().join("slices"))
    }

    pub fn kernel_event_counter_path(&self) -> PathBuf {
        self.event_counter_root()
            .join(&self.daemon_id)
            .join("event-counter.json")
    }

    pub fn kernel_relay_event_counter_path(&self) -> PathBuf {
        self.event_counter_root()
            .join(&self.daemon_id)
            .join("relay-event-counter.json")
    }

    pub fn kernel_prompt_counter_path(&self) -> PathBuf {
        self.event_counter_root()
            .join(&self.daemon_id)
            .join("prompt-counter.json")
    }

    fn event_counter_root(&self) -> PathBuf {
        if let Some(root) = &self.publication_control_state_root {
            return root.join("kernel-events");
        }
        self.user_config
            .state
            .path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(expand_user_path)
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .map(|root| root.join("kernel-events"))
            .unwrap_or_else(|| default_state_dir().join("kernel-events"))
    }

    pub fn default_runtime_identity_path() -> PathBuf {
        default_state_dir().join("daemon").join("identity.json")
    }

    pub fn default_machine_identity_path() -> PathBuf {
        default_config_dir().join("machine").join("identity.json")
    }

    pub fn default_kernel_registry_path() -> PathBuf {
        default_config_dir().join("kernels").join("registry.json")
    }

    pub fn default_active_kernel_registry_dir() -> PathBuf {
        default_config_dir().join("kernels").join("active")
    }

    pub fn default_daemon_config_path() -> PathBuf {
        persisted_daemon::default_daemon_config_path()
    }

    pub fn default_user_config_path() -> PathBuf {
        default_config_dir().join("config.toml")
    }
}

pub(super) fn default_state_dir() -> PathBuf {
    if let Some(home_dir) = chariox_home_dir() {
        return home_dir.join("state");
    }

    if let Some(state_dir) = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return state_dir.join("chariox");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".local").join("state").join("chariox");
    }

    env::temp_dir().join("chariox")
}

pub(super) fn default_config_dir() -> PathBuf {
    if let Some(home_dir) = chariox_home_dir() {
        return home_dir;
    }

    if let Some(config_dir) = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return config_dir.join("chariox");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".chariox");
    }

    env::temp_dir().join("chariox").join("config")
}

fn default_runtime_dir() -> PathBuf {
    if let Some(home_dir) = chariox_home_dir() {
        return home_dir.join("run");
    }

    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return runtime_dir.join("chariox");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".chariox").join("run");
    }

    env::temp_dir().join("chariox")
}

fn expand_user_path(value: &str) -> PathBuf {
    let value = value.trim();
    if let Some(home_dir) = chariox_home_dir() {
        if value == "~/.chariox" {
            return home_dir;
        }
        if let Some(suffix) = value.strip_prefix("~/.chariox/") {
            return home_dir.join(suffix);
        }
    }
    if value == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(suffix) = value.strip_prefix("~/") {
        if let Some(home_dir) = env::var_os("HOME").map(PathBuf::from) {
            return home_dir.join(suffix);
        }
    }
    PathBuf::from(value)
}

fn chariox_home_dir() -> Option<PathBuf> {
    env::var_os("CHARIOX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
