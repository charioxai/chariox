use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;

#[derive(Clone)]
pub(crate) struct CompatibilityRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
}

impl CompatibilityRuntimeState {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app }
    }

    pub(crate) fn app(&self) -> Arc<Mutex<DaemonApp>> {
        Arc::clone(&self.app)
    }

    pub(crate) async fn with_app_mut<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }
}
