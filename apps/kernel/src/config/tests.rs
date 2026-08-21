use super::*;
use std::env;

unsafe fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => env::set_var(key, value),
        None => env::remove_var(key),
    }
}

mod runtime_identity;
mod user_config_policy;
mod workflow_history_state;
