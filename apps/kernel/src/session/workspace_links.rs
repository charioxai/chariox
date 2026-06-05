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
        let requested = normalize_workspace_link_repo_root(path_to_workspace_link_root(repo_root));
        if self.repo_root == requested {
            return true;
        }
        canonical_workspace_link_root(&self.repo_root)
            .zip(canonical_workspace_link_root(&requested))
            .is_some_and(|(left, right)| left == right)
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

fn canonical_workspace_link_root(repo_root: &str) -> Option<String> {
    PathBuf::from(repo_root)
        .canonicalize()
        .ok()
        .map(|path| normalize_workspace_link_repo_root(path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn workspace_link_attachment_matches_canonical_equivalent_repo_roots() {
        let base = std::env::temp_dir().join(format!(
            "arroba-workspace-link-canonical-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let real_root = base.join("real").join("repo");
        let alias_root = base.join("alias-repo");
        std::fs::create_dir_all(&real_root).expect("real repo root should exist");
        std::os::unix::fs::symlink(&real_root, &alias_root).expect("repo alias should link");
        let mut link = WorkspaceLinkDefinition::new("link-1", "session-1", "shared", "user-1");
        link.attach(WorkspaceLinkAttachment::new(
            "link-1",
            "user-1",
            "machine-1",
            "kernel-1",
            alias_root.to_string_lossy(),
            None,
            None,
        ));

        let attachment = link.attachment_for_repo_root(&real_root);

        let _ = std::fs::remove_dir_all(&base);
        assert!(attachment.is_some());
    }
}
