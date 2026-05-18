use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLinkAttachment {
    link_id: String,
    user_id: String,
    machine_id: String,
    kernel_id: String,
    repo_root: String,
    branch: Option<String>,
    repo_fingerprint: Option<String>,
    attached_at_ms: u64,
}

impl WorkspaceLinkAttachment {
    pub fn new(
        link_id: impl Into<String>,
        user_id: impl Into<String>,
        machine_id: impl Into<String>,
        kernel_id: impl Into<String>,
        repo_root: impl Into<String>,
        branch: Option<String>,
        repo_fingerprint: Option<String>,
    ) -> Self {
        Self {
            link_id: link_id.into(),
            user_id: user_id.into(),
            machine_id: machine_id.into(),
            kernel_id: kernel_id.into(),
            repo_root: normalize_workspace_link_repo_root(repo_root.into()),
            branch,
            repo_fingerprint,
            attached_at_ms: unix_epoch_ms(),
        }
    }

    pub fn link_id(&self) -> &str {
        &self.link_id
    }
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }
    pub fn kernel_id(&self) -> &str {
        &self.kernel_id
    }
    pub fn repo_root(&self) -> &str {
        &self.repo_root
    }
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }
    pub fn repo_fingerprint(&self) -> Option<&str> {
        self.repo_fingerprint.as_deref()
    }
    pub fn attached_at_ms(&self) -> u64 {
        self.attached_at_ms
    }

    fn matches_repo_root(&self, repo_root: &Path) -> bool {
        self.repo_root == normalize_workspace_link_repo_root(path_to_workspace_link_root(repo_root))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLinkDefinition {
    link_id: String,
    session_id: String,
    name: String,
    created_by_user_id: String,
    created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<WorkspaceLinkAttachment>,
}

impl WorkspaceLinkDefinition {
    pub fn new(
        link_id: impl Into<String>,
        session_id: impl Into<String>,
        name: impl Into<String>,
        created_by_user_id: impl Into<String>,
    ) -> Self {
        Self {
            link_id: link_id.into(),
            session_id: session_id.into(),
            name: name.into(),
            created_by_user_id: created_by_user_id.into(),
            created_at_ms: unix_epoch_ms(),
            attachments: Vec::new(),
        }
    }

    pub fn link_id(&self) -> &str {
        &self.link_id
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn attachments(&self) -> &[WorkspaceLinkAttachment] {
        &self.attachments
    }

    pub fn attach(&mut self, attachment: WorkspaceLinkAttachment) -> WorkspaceLinkAttachment {
        self.attachments.retain(|existing| {
            !(existing.user_id == attachment.user_id && existing.repo_root == attachment.repo_root)
        });
        self.attachments.push(attachment.clone());
        attachment
    }

    pub fn detach(
        &mut self,
        user_id: &str,
        repo_root: Option<&Path>,
    ) -> Vec<WorkspaceLinkAttachment> {
        let mut removed = Vec::new();
        self.attachments.retain(|attachment| {
            let matches_user = attachment.user_id == user_id;
            let matches_root =
                repo_root.is_none_or(|repo_root| attachment.matches_repo_root(repo_root));
            if matches_user && matches_root {
                removed.push(attachment.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn attachment_for_repo_root(&self, repo_root: &Path) -> Option<&WorkspaceLinkAttachment> {
        self.attachments
            .iter()
            .find(|attachment| attachment.matches_repo_root(repo_root))
    }
}

pub fn normalize_workspace_link_repo_root(repo_root: impl Into<String>) -> String {
    let value = repo_root.into();
    let path = PathBuf::from(value);
    path.to_string_lossy().trim_end_matches('/').to_string()
}

pub fn path_to_workspace_link_root(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
