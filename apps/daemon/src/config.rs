use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub daemon_id: String,
    pub host_machine_id: String,
    pub os_user: String,
    pub local_socket_path: PathBuf,
    pub kernel_websocket_host: String,
    pub kernel_websocket_port: u16,
    pub kernel_websocket_queue_capacity: usize,
    pub kernel_websocket_write_delay_ms: u64,
    pub runtime_mcp_host: String,
    pub runtime_mcp_port: u16,
    pub session_history_root: PathBuf,
}

impl DaemonConfig {
    pub fn load_from_env() -> Self {
        let daemon_id = env::var("ARROBA_DAEMON_ID").unwrap_or_else(|_| "daemon-local".to_string());
        Self {
            local_socket_path: env::var_os("ARROBA_DAEMON_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| Self::default_local_socket_path(&daemon_id)),
            kernel_websocket_host: env::var("ARROBA_KERNEL_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            kernel_websocket_port: env::var("ARROBA_KERNEL_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(43118),
            kernel_websocket_queue_capacity: env::var("ARROBA_KERNEL_QUEUE_CAPACITY")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(128),
            kernel_websocket_write_delay_ms: env::var("ARROBA_KERNEL_WRITE_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            runtime_mcp_host: env::var("ARROBA_MCP_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            runtime_mcp_port: env::var("ARROBA_MCP_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(43120),
            session_history_root: env::var_os("ARROBA_SESSION_HISTORY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(Self::default_session_history_root),
            daemon_id,
            host_machine_id: env::var("ARROBA_MACHINE_ID")
                .unwrap_or_else(|_| "machine-local".to_string()),
            os_user: env::var("USER")
                .or_else(|_| env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }

    pub fn new(
        daemon_id: impl Into<String>,
        host_machine_id: impl Into<String>,
        os_user: impl Into<String>,
    ) -> Self {
        let daemon_id = daemon_id.into();
        Self {
            local_socket_path: Self::default_local_socket_path(&daemon_id),
            kernel_websocket_host: "127.0.0.1".to_string(),
            kernel_websocket_port: 43118,
            kernel_websocket_queue_capacity: 128,
            kernel_websocket_write_delay_ms: 0,
            runtime_mcp_host: "127.0.0.1".to_string(),
            runtime_mcp_port: 43120,
            session_history_root: Self::default_session_history_root(),
            daemon_id,
            host_machine_id: host_machine_id.into(),
            os_user: os_user.into(),
        }
    }

    pub fn for_tests() -> Self {
        static TEST_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

        let index = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
        let mut config = Self::new("daemon-test", "machine-test", "tester");
        config.local_socket_path = std::env::temp_dir().join("arroba-tests").join(format!(
            "daemon-test-{}-{}.sock",
            std::process::id(),
            index
        ));
        config.session_history_root = std::env::temp_dir().join("arroba-tests").join(format!(
            "session-history-{}-{}",
            std::process::id(),
            index
        ));
        config
    }

    pub fn with_local_socket_path(mut self, path: PathBuf) -> Self {
        self.local_socket_path = path;
        self
    }

    pub fn with_session_history_root(mut self, path: PathBuf) -> Self {
        self.session_history_root = path;
        self
    }

    pub fn kernel_websocket_url(&self) -> String {
        format!(
            "ws://{}:{}/kernel",
            self.kernel_websocket_host, self.kernel_websocket_port
        )
    }

    pub fn runtime_mcp_url(&self) -> String {
        format!(
            "http://{}:{}/mcp",
            self.runtime_mcp_host, self.runtime_mcp_port
        )
    }

    pub fn default_local_socket_path(daemon_id: &str) -> PathBuf {
        default_runtime_dir().join(format!("{daemon_id}.sock"))
    }

    pub fn default_session_history_root() -> PathBuf {
        default_state_dir().join("sessions")
    }

    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_non_empty("daemon_id", &self.daemon_id)?;
        validate_non_empty("host_machine_id", &self.host_machine_id)?;
        validate_non_empty("os_user", &self.os_user)?;
        if self.local_socket_path.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "local_socket_path",
                message: "value must not be empty",
            });
        }
        validate_non_empty("kernel_websocket_host", &self.kernel_websocket_host)?;
        if self.kernel_websocket_port == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_websocket_port",
                message: "value must not be zero",
            });
        }
        if self.kernel_websocket_queue_capacity == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_websocket_queue_capacity",
                message: "value must not be zero",
            });
        }
        validate_non_empty("runtime_mcp_host", &self.runtime_mcp_host)?;
        if self.runtime_mcp_port == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "runtime_mcp_port",
                message: "value must not be zero",
            });
        }
        if self.session_history_root.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "session_history_root",
                message: "value must not be empty",
            });
        }
        Ok(())
    }
}

fn default_state_dir() -> PathBuf {
    if let Some(state_dir) = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return state_dir.join("arroba");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".local").join("state").join("arroba");
    }

    std::env::temp_dir().join("arroba")
}

fn default_runtime_dir() -> PathBuf {
    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return runtime_dir.join("arroba");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".arroba").join("run");
    }

    std::env::temp_dir().join("arroba")
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), DaemonError> {
    if value.trim().is_empty() {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be empty",
        });
    }

    Ok(())
}
