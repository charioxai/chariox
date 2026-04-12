use crate::agent::AgentInstance;
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;

pub(crate) struct KernelSessionService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelSessionService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn attach(
        &mut self,
        request: AttachRequest,
    ) -> Result<RuntimeAttachment, DaemonError> {
        self.app.attach(request)
    }

    pub(crate) fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        self.app.detach(attachment_id)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        self.app.focus_agent(session_id, agent_id)
    }

    pub(crate) fn cycle_agent_focus(
        &mut self,
        session_id: &str,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        self.app.cycle_agent_focus(session_id)
    }

    pub(crate) fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        self.app.resize_terminal(session_id, cols, rows)
    }
}
