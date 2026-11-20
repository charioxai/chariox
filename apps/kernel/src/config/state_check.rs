use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::{
    env_loader::{daemon_id_override, kernel_websocket_endpoint},
    identity::read_registered_kernel_id,
    paths::{
        chariox_home_dir, default_config_dir, default_durable_state_path,
        expand_user_path_from_home,
    },
    read_user_config, CharioxUserConfig,
};

pub(crate) struct StateCheckTarget {
    pub(crate) home: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) daemon_id: String,
}

impl StateCheckTarget {
    pub(crate) fn resolve(home: Option<&Path>) -> Result<Self, String> {
        let configured_home = home.map(Path::to_path_buf).or_else(chariox_home_dir);
        let home = configured_home.clone().unwrap_or_else(default_config_dir);
        let metadata = fs::metadata(&home)
            .map_err(|error| format!("cannot access kernel home {} ({error})", home.display()))?;
        if !metadata.is_dir() {
            return Err(format!("kernel home {} is not a directory", home.display()));
        }

        let config_path = home.join("config.toml");
        let config = match read_user_config(&config_path) {
            Ok(text) => toml::from_str::<CharioxUserConfig>(&text).map_err(|_| {
                format!("kernel configuration {} is invalid", config_path.display())
            })?,
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    && fs::symlink_metadata(&config_path)
                        .is_err_and(|error| error.kind() == ErrorKind::NotFound) =>
            {
                CharioxUserConfig::default()
            }
            Err(error) => {
                return Err(format!(
                    "cannot read kernel configuration {} ({error})",
                    config_path.display()
                ));
            }
        };

        let daemon_id = match daemon_id_override() {
            Some(daemon_id) => daemon_id,
            None => {
                let (host, port) = kernel_websocket_endpoint();
                read_registered_kernel_id(&home, &host, port)?
            }
        };
        let database = if let Some(root) = env::var_os("CHARIOX_PUBLICATION_CONTROL_STATE_DIR") {
            PathBuf::from(root).join("state.db")
        } else if let Some(path) = config.state.path {
            expand_user_path_from_home(&path, configured_home.as_deref())
        } else {
            default_durable_state_path(&home, &daemon_id)
        };

        Ok(Self {
            home,
            database,
            daemon_id,
        })
    }
}
