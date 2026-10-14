//! MP-08 / MP-10 / MP-11: wire standalone resolution through the normal Project lane.
use super::*;
impl SessionRuntimeStore {
    pub(in crate::runtime::session_actor) fn resolve_project(
        &self,
        request: crate::local::ResolveProjectRequest,
        caller: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.state.resolve_project(request, caller)
    }
}
