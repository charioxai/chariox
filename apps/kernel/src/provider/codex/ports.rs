use std::env;
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};

use crate::error::DaemonError;

const CODEX_PORT_OVERRIDE: &str = "CHARIOX_CODEX_PORT";
const CODEX_PORT_RANGE_OVERRIDE: &str = "CHARIOX_CODEX_PORT_RANGE";
static CODEX_MANAGED_CATALOG_PORT: OnceLock<Mutex<Option<u16>>> = OnceLock::new();

pub(super) fn resolve_codex_launch_port(is_session_launch: bool) -> Result<u16, DaemonError> {
    if is_session_launch {
        return match reserve_unused_port_from_range(
            CODEX_PORT_RANGE_OVERRIDE,
            "codex_reserve_port_range",
        )? {
            Some(port) => Ok(port),
            None => reserve_unused_port(),
        };
    }
    resolve_codex_catalog_port()
}

pub(super) fn resolve_codex_catalog_port() -> Result<u16, DaemonError> {
    if let Some(value) = env::var_os(CODEX_PORT_OVERRIDE) {
        let value = value.to_string_lossy().into_owned();
        return value
            .parse::<u16>()
            .map_err(|_| DaemonError::InvalidConfig {
                field: "CHARIOX_CODEX_PORT",
                message: "must be a valid TCP port",
            });
    }

    let port = CODEX_MANAGED_CATALOG_PORT.get_or_init(|| Mutex::new(None));
    let mut guard = port.lock().map_err(|error| DaemonError::LocalTransport {
        operation: "codex_managed_catalog_port",
        message: error.to_string(),
    })?;
    if let Some(port) = *guard {
        return Ok(port);
    }
    let reserved = reserve_unused_port()?;
    *guard = Some(reserved);
    Ok(reserved)
}

fn reserve_unused_port() -> Result<u16, DaemonError> {
    TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_reserve_port",
            message: error.to_string(),
        })?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_reserve_port",
            message: error.to_string(),
        })
}

fn reserve_unused_port_from_range(
    env_name: &'static str,
    operation: &'static str,
) -> Result<Option<u16>, DaemonError> {
    let Some(value) = env::var_os(env_name) else {
        return Ok(None);
    };
    let value = value.to_string_lossy();
    let Some((start, end)) = value.split_once('-') else {
        return Err(DaemonError::InvalidConfig {
            field: env_name,
            message: "must use START-END TCP port range syntax",
        });
    };
    let start = start
        .parse::<u16>()
        .map_err(|_| DaemonError::InvalidConfig {
            field: env_name,
            message: "range start must be a valid TCP port",
        })?;
    let end = end.parse::<u16>().map_err(|_| DaemonError::InvalidConfig {
        field: env_name,
        message: "range end must be a valid TCP port",
    })?;
    if start > end {
        return Err(DaemonError::InvalidConfig {
            field: env_name,
            message: "range start must be less than or equal to range end",
        });
    }
    for port in start..=end {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(Some(port));
        }
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!("no available port in {env_name}={value}"),
    })
}
