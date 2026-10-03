use super::*;

pub(super) struct InertPtyCleanup {
    pub(super) app: Arc<Mutex<DaemonApp>>,
    pub(super) provider_run_id: String,
}

impl Drop for InertPtyCleanup {
    fn drop(&mut self) {
        if let Ok(mut app) = self.app.try_lock() {
            let _ = app.pty_mut().remove_process(&self.provider_run_id);
        }
    }
}

pub(super) fn spawn_inert_pty_for_run(app: &mut DaemonApp, provider_run_id: &str) {
    app.pty_mut()
        .spawn(crate::pty::PtySpawnRequest {
            process_key: provider_run_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            program: "/bin/sh".to_string(),
            args: ["-c", "exec sleep 300"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            env: Default::default(),
            env_remove: Vec::new(),
            working_directory: None,
            cols: 80,
            rows: 24,
        })
        .expect("inert provider fixture PTY should stay live");
}
