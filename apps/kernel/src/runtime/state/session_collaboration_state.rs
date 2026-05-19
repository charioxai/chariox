use std::path::Path;

use super::*;

impl KernelRuntimeState {
    pub(crate) fn list_session_members(
        &self,
        session_id: &str,
    ) -> Result<
        (
            Vec<crate::session::SessionMember>,
            Vec<crate::session::SessionInvite>,
        ),
        DaemonError,
    > {
        self.owned
            .session_store
            .read()
            .list_session_members(session_id)
    }

    pub(crate) fn create_session_invite(
        &self,
        session_id: &str,
        invite_id: String,
        created_by_user_id: String,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::SessionInvite,
        ),
        DaemonError,
    > {
        let (session, invite) = self.owned.session_store.write().create_session_invite(
            session_id,
            invite_id,
            created_by_user_id,
            expires_at_ms,
            max_uses,
        )?;
        self.owned.session_projection.update(session.clone());
        Ok((session, invite))
    }

    pub(crate) fn join_session_invite(
        &self,
        session_id: &str,
        invite_id: &str,
        user_id: String,
        now_ms: u64,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::SessionMember,
        ),
        DaemonError,
    > {
        let (session, member) = self
            .owned
            .session_store
            .write()
            .join_session_invite(session_id, invite_id, user_id, now_ms)?;
        self.owned.session_projection.update(session.clone());
        Ok((session, member))
    }

    pub(crate) fn revoke_session_invite(
        &self,
        session_id: &str,
        invite_ref: &str,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::SessionInvite,
        ),
        DaemonError,
    > {
        let (session, invite) = self
            .owned
            .session_store
            .write()
            .revoke_session_invite(session_id, invite_ref)?;
        self.owned.session_projection.update(session.clone());
        Ok((session, invite))
    }

    pub(crate) fn create_workspace_link(
        &self,
        session_id: &str,
        name: String,
        created_by_user_id: String,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::WorkspaceLinkDefinition,
        ),
        DaemonError,
    > {
        let (session, link) = self.owned.session_store.write().create_workspace_link(
            session_id,
            name,
            created_by_user_id,
        )?;
        self.owned.session_projection.update(session.clone());
        Ok((session, link))
    }

    pub(crate) fn list_workspace_links(
        &self,
        session_id: &str,
    ) -> Result<Vec<crate::session::WorkspaceLinkDefinition>, DaemonError> {
        self.owned
            .session_store
            .read()
            .list_workspace_links(session_id)
    }

    pub(crate) fn resolve_workspace_link_ref(
        &self,
        session_id: &str,
        link_ref: &str,
    ) -> Result<crate::session::WorkspaceLinkDefinition, DaemonError> {
        self.owned
            .session_store
            .read()
            .resolve_workspace_link_ref(session_id, link_ref)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn attach_workspace_link(
        &self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        machine_id: String,
        kernel_id: String,
        repo_root: String,
        branch: Option<String>,
        repo_fingerprint: Option<String>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::WorkspaceLinkDefinition,
            crate::session::WorkspaceLinkAttachment,
        ),
        DaemonError,
    > {
        let result = self.owned.session_store.write().attach_workspace_link(
            session_id,
            link_ref,
            user_id,
            machine_id,
            kernel_id,
            repo_root,
            branch,
            repo_fingerprint,
        )?;
        self.owned.session_projection.update(result.0.clone());
        Ok(result)
    }

    pub(crate) fn detach_workspace_link(
        &self,
        session_id: &str,
        link_ref: &str,
        user_id: String,
        repo_root: Option<&Path>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            crate::session::WorkspaceLinkDefinition,
            Vec<crate::session::WorkspaceLinkAttachment>,
        ),
        DaemonError,
    > {
        let result = self
            .owned
            .session_store
            .write()
            .detach_workspace_link(session_id, link_ref, user_id, repo_root)?;
        self.owned.session_projection.update(result.0.clone());
        Ok(result)
    }
}
