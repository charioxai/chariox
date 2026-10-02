//! Disposable public identity fixtures for hosted slice context regressions.
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) const MACHINE: &str = "machine-a";
pub(crate) const SLICE: &str = "slice-local-1";

pub(crate) fn canonical_worker(case: usize) -> String {
    let value: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/slice-worker-identity.json"
    )))
    .unwrap();
    value["cases"][case]["workerKernelRef"]
        .as_str()
        .unwrap()
        .to_string()
}

pub(crate) struct Environment {
    previous: Vec<(&'static str, Option<OsString>)>,
    pub(crate) root: PathBuf,
    _lock: crate::env_lock::EnvGuard,
}

impl Environment {
    pub(crate) fn new(machine: &str, daemon: &str, slice: Option<&str>) -> Self {
        let lock = crate::env_lock::lock();
        let names = [
            "CHARIOX_MACHINE_ID",
            "CHARIOX_SLICE_MACHINE_ID",
            "CHARIOX_DAEMON_ID",
            "CHARIOX_SLICE_DAEMON_ID",
            "CHARIOX_DAEMON_ALIAS",
            "CHARIOX_SLICE_ID",
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            "CHARIOX_SLICE_SCREEN_TOOL",
            "CHARIOX_BROWSER_CONTROLLER_SCRIPT",
        ];
        let previous = names
            .into_iter()
            .map(|name| {
                let value = std::env::var_os(name);
                std::env::remove_var(name);
                (name, value)
            })
            .collect();
        std::env::set_var("CHARIOX_MACHINE_ID", machine);
        std::env::set_var("CHARIOX_SLICE_MACHINE_ID", machine);
        std::env::set_var("CHARIOX_DAEMON_ID", daemon);
        std::env::set_var("CHARIOX_DAEMON_ALIAS", "slice:friendly-name");
        if let Some(slice) = slice {
            std::env::set_var("CHARIOX_SLICE_ID", slice);
        }
        let root = std::env::temp_dir().join(format!(
            "chariox-hosted-worker-context-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        std::fs::create_dir(&root).unwrap();
        Self {
            previous,
            root,
            _lock: lock,
        }
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        for (name, previous) in self.previous.drain(..).rev() {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        // Exact disposable UUID fixture contains only our synthetic public files.
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

pub(crate) fn provider_run(session: &str, agent: &str) -> crate::provider::RuntimeProviderRun {
    use crate::provider::*;
    let request = LaunchProviderRequest::new(session, "agent", "codex", "default", "model")
        .with_agent_id(agent);
    RuntimeProviderRun::new(
        "run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "synthetic-provider".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    )
}
