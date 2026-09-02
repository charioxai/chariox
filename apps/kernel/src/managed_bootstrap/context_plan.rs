use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::state::valid_identifier;
use crate::managed_context::development::{
    DevelopmentRepositoryRole, DevelopmentSourceRepositoryBinding,
};
use crate::managed_context::package::{
    ManagedContextDevelopmentSelection, ManagedContextGitCredentialSelection,
    ManagedContextKernelSelection, ManagedContextPlanBinding, ManagedContextProviderAccount,
    ManagedContextProviderAccountSelection,
};

const MAX_CONTEXT_PLAN_BYTES: usize = 72 * 1024;
const MAX_REFERENCE_BYTES: usize = 4 * 1024;
const MAX_REPOSITORIES: usize = 32;
const MAX_PROVIDER_ACCOUNTS: usize = 16;
const MAX_GIT_CREDENTIALS: usize = 16;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedKernelContextPlan {
    schema_version: u32,
    context_id: String,
    plan_digest: String,
    source: Option<ManagedKernelContextSource>,
    kernel_context: ManagedKernelContextSelection,
    development_setup: ManagedKernelDevelopmentSetup,
    provider_accounts: ManagedKernelProviderAccounts,
    git_credentials: ManagedKernelGitCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedKernelContextSource {
    source_target_id: String,
    relay_realm_id: String,
    machine_id: String,
    kernel_id: String,
    key_thumbprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManagedKernelContextSelection {
    Empty,
    SourceKernel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ManagedKernelDevelopmentSetup {
    Empty,
    SourceProject {
        project_id: String,
        repositories: Vec<ManagedKernelRepositorySelection>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedKernelRepositorySelection {
    role: ManagedKernelRepositoryRole,
    workspace_id: String,
    worktree_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManagedKernelRepositoryRole {
    Primary,
    Supporting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ManagedKernelProviderAccounts {
    None,
    Selected {
        accounts: Vec<ManagedKernelProviderAccount>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedKernelProviderAccount {
    provider: String,
    account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ManagedKernelGitCredentials {
    None,
    Selected { credential_ids: Vec<String> },
}

impl ManagedKernelContextPlan {
    #[cfg(test)]
    pub(crate) fn context_id(&self) -> &str {
        &self.context_id
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let encoded =
            serde_json::to_vec(self).map_err(|_| "managed context plan is not serializable")?;
        if self.schema_version != 1
            || !valid_identifier(&self.context_id)
            || !valid_digest(&self.plan_digest)
            || encoded.len() > MAX_CONTEXT_PLAN_BYTES
        {
            return Err("managed context plan identity is invalid");
        }
        if let Some(source) = &self.source {
            if !valid_identifier(&source.source_target_id)
                || !valid_identifier(&source.relay_realm_id)
                || !valid_identifier(&source.machine_id)
                || !valid_identifier(&source.kernel_id)
                || !valid_thumbprint(&source.key_thumbprint)
            {
                return Err("managed context plan source is invalid");
            }
        }
        self.validate_development()?;
        self.validate_provider_accounts()?;
        self.validate_git_credentials()?;
        let source_required = self.kernel_context == ManagedKernelContextSelection::SourceKernel
            || matches!(
                self.development_setup,
                ManagedKernelDevelopmentSetup::SourceProject { .. }
            )
            || matches!(
                self.provider_accounts,
                ManagedKernelProviderAccounts::Selected { .. }
            )
            || matches!(
                self.git_credentials,
                ManagedKernelGitCredentials::Selected { .. }
            );
        if source_required != self.source.is_some() {
            return Err("managed context plan source selection is inconsistent");
        }
        if self.compute_digest()? != self.plan_digest {
            return Err("managed context plan digest is invalid");
        }
        Ok(())
    }

    pub(crate) fn source_binding(&self) -> Option<ManagedKernelContextSourceBinding<'_>> {
        self.source
            .as_ref()
            .map(|source| ManagedKernelContextSourceBinding {
                relay_realm_id: &source.relay_realm_id,
                machine_id: &source.machine_id,
                kernel_id: &source.kernel_id,
                key_thumbprint: &source.key_thumbprint,
            })
    }

    pub(crate) fn source_project_id(&self) -> Option<&str> {
        match &self.development_setup {
            ManagedKernelDevelopmentSetup::SourceProject { project_id, .. } => Some(project_id),
            ManagedKernelDevelopmentSetup::Empty => None,
        }
    }

    pub(crate) fn package_binding(&self) -> ManagedContextPlanBinding {
        ManagedContextPlanBinding {
            context_id: self.context_id.clone(),
            plan_digest: self.plan_digest.clone(),
            kernel_context: match self.kernel_context {
                ManagedKernelContextSelection::Empty => ManagedContextKernelSelection::Empty,
                ManagedKernelContextSelection::SourceKernel => {
                    ManagedContextKernelSelection::SourceKernel
                }
            },
            development: match &self.development_setup {
                ManagedKernelDevelopmentSetup::Empty => ManagedContextDevelopmentSelection::Empty,
                ManagedKernelDevelopmentSetup::SourceProject {
                    project_id,
                    repositories,
                } => ManagedContextDevelopmentSelection::SourceProject {
                    project_id: project_id.clone(),
                    repositories: repositories
                        .iter()
                        .map(|repository| DevelopmentSourceRepositoryBinding {
                            role: match repository.role {
                                ManagedKernelRepositoryRole::Primary => {
                                    DevelopmentRepositoryRole::Primary
                                }
                                ManagedKernelRepositoryRole::Supporting => {
                                    DevelopmentRepositoryRole::Supporting
                                }
                            },
                            workspace_id: repository.workspace_id.clone(),
                            worktree_id: repository.worktree_id.clone(),
                        })
                        .collect(),
                },
            },
            provider_accounts: match &self.provider_accounts {
                ManagedKernelProviderAccounts::None => ManagedContextProviderAccountSelection::None,
                ManagedKernelProviderAccounts::Selected { accounts } => {
                    ManagedContextProviderAccountSelection::Selected {
                        accounts: accounts
                            .iter()
                            .map(|account| ManagedContextProviderAccount {
                                provider: account.provider.clone(),
                                account_profile: account.account_profile.clone(),
                            })
                            .collect(),
                    }
                }
            },
            git_credentials: match &self.git_credentials {
                ManagedKernelGitCredentials::None => ManagedContextGitCredentialSelection::None,
                ManagedKernelGitCredentials::Selected { credential_ids } => {
                    ManagedContextGitCredentialSelection::Selected {
                        credential_ids: credential_ids.clone(),
                    }
                }
            },
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.source.is_none()
            && self.kernel_context == ManagedKernelContextSelection::Empty
            && matches!(self.development_setup, ManagedKernelDevelopmentSetup::Empty)
            && matches!(self.provider_accounts, ManagedKernelProviderAccounts::None)
            && matches!(self.git_credentials, ManagedKernelGitCredentials::None)
    }

    fn validate_development(&self) -> Result<(), &'static str> {
        let ManagedKernelDevelopmentSetup::SourceProject {
            project_id,
            repositories,
        } = &self.development_setup
        else {
            return Ok(());
        };
        if !valid_identifier(project_id)
            || repositories.is_empty()
            || repositories.len() > MAX_REPOSITORIES
        {
            return Err("managed context development selection is invalid");
        }
        let mut primary_count = 0;
        let mut identities = HashSet::new();
        for repository in repositories {
            if repository.role == ManagedKernelRepositoryRole::Primary {
                primary_count += 1;
            }
            if !valid_reference(&repository.workspace_id)
                || repository
                    .worktree_id
                    .as_deref()
                    .is_some_and(|value| !valid_reference(value))
                || !identities.insert((
                    repository.workspace_id.as_str(),
                    repository.worktree_id.as_deref(),
                ))
            {
                return Err("managed context repository selection is invalid");
            }
        }
        if primary_count != 1 {
            return Err("managed context repository selection must have one primary");
        }
        Ok(())
    }

    fn validate_provider_accounts(&self) -> Result<(), &'static str> {
        let ManagedKernelProviderAccounts::Selected { accounts } = &self.provider_accounts else {
            return Ok(());
        };
        if accounts.is_empty() || accounts.len() > MAX_PROVIDER_ACCOUNTS {
            return Err("managed context provider account selection is invalid");
        }
        let mut previous: Option<(&str, &str)> = None;
        for account in accounts {
            if !valid_identifier(&account.provider) || !valid_identifier(&account.account_profile) {
                return Err("managed context provider account selection is invalid");
            }
            let current = (account.provider.as_str(), account.account_profile.as_str());
            if previous.is_some_and(|value| value >= current) {
                return Err("managed context provider account selection is not canonical");
            }
            previous = Some(current);
        }
        Ok(())
    }

    fn validate_git_credentials(&self) -> Result<(), &'static str> {
        let ManagedKernelGitCredentials::Selected { credential_ids } = &self.git_credentials else {
            return Ok(());
        };
        if credential_ids.is_empty() || credential_ids.len() > MAX_GIT_CREDENTIALS {
            return Err("managed context Git credential selection is invalid");
        }
        let mut previous: Option<&str> = None;
        for credential_id in credential_ids {
            if !valid_identifier(credential_id)
                || previous.is_some_and(|value| value >= credential_id.as_str())
            {
                return Err("managed context Git credential selection is not canonical");
            }
            previous = Some(credential_id);
        }
        Ok(())
    }

    fn compute_digest(&self) -> Result<String, &'static str> {
        let value = serde_json::json!({
            "schemaVersion": self.schema_version,
            "source": self.source,
            "kernelContext": self.kernel_context,
            "developmentSetup": self.development_setup,
            "providerAccounts": self.provider_accounts,
            "gitCredentials": self.git_credentials,
        });
        let encoded = serde_json::to_vec(&canonical_json_value(&value))
            .map_err(|_| "managed context plan is not serializable")?;
        Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
    }

    #[cfg(test)]
    pub(crate) fn source_project_for_tests(
        context_id: &str,
        relay_realm_id: &str,
        source_kernel_id: &str,
        source_key_thumbprint: &str,
        project_id: &str,
    ) -> Self {
        let mut plan = Self {
            schema_version: 1,
            context_id: context_id.to_string(),
            plan_digest: format!("sha256:{}", "0".repeat(64)),
            source: Some(ManagedKernelContextSource {
                source_target_id: "source-target-test".to_string(),
                relay_realm_id: relay_realm_id.to_string(),
                machine_id: "source-machine-test".to_string(),
                kernel_id: source_kernel_id.to_string(),
                key_thumbprint: source_key_thumbprint.to_string(),
            }),
            kernel_context: ManagedKernelContextSelection::SourceKernel,
            development_setup: ManagedKernelDevelopmentSetup::SourceProject {
                project_id: project_id.to_string(),
                repositories: vec![ManagedKernelRepositorySelection {
                    role: ManagedKernelRepositoryRole::Primary,
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: None,
                }],
            },
            provider_accounts: ManagedKernelProviderAccounts::None,
            git_credentials: ManagedKernelGitCredentials::None,
        };
        plan.plan_digest = plan.compute_digest().expect("test plan digest");
        plan
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests(context_id: &str) -> Self {
        let mut plan = Self {
            schema_version: 1,
            context_id: context_id.to_string(),
            plan_digest: format!("sha256:{}", "0".repeat(64)),
            source: None,
            kernel_context: ManagedKernelContextSelection::Empty,
            development_setup: ManagedKernelDevelopmentSetup::Empty,
            provider_accounts: ManagedKernelProviderAccounts::None,
            git_credentials: ManagedKernelGitCredentials::None,
        };
        plan.plan_digest = plan.compute_digest().expect("test plan digest");
        plan
    }
}

pub(crate) struct ManagedKernelContextSourceBinding<'a> {
    pub(crate) relay_realm_id: &'a str,
    pub(crate) machine_id: &'a str,
    pub(crate) kernel_id: &'a str,
    pub(crate) key_thumbprint: &'a str,
}

impl fmt::Debug for ManagedKernelContextPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedKernelContextPlan")
            .field("schema_version", &self.schema_version)
            .field("context_id", &self.context_id)
            .field("plan_digest", &self.plan_digest)
            .field("has_source", &self.source.is_some())
            .field("kernel_context", &self.kernel_context)
            .field(
                "development_kind",
                &self
                    .source_project_id()
                    .map(|_| "source_project")
                    .unwrap_or("empty"),
            )
            .finish_non_exhaustive()
    }
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json_value(&values[key]));
            }
            Value::Object(canonical)
        }
        value => value.clone(),
    }
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_REFERENCE_BYTES
        && !value
            .bytes()
            .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
}

fn valid_thumbprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71 && value.starts_with("sha256:") && valid_thumbprint(&value[7..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_plan_digest_matches_the_cloud_canonical_contract() {
        let plan = ManagedKernelContextPlan::source_project_for_tests(
            "managed_ctx_test",
            "realm-1",
            "source-kernel",
            &"a".repeat(64),
            "project-chariox",
        );
        assert_eq!(
            plan.plan_digest,
            "sha256:a824b83bf0ebad38bfb6e5966d194ff6fdf70b53575257792d0328b59eb1c426"
        );
        plan.validate().expect("valid Cloud-compatible plan");
    }

    #[test]
    fn context_plan_rejects_tampered_digest_and_ambiguous_sources() {
        let mut plan = ManagedKernelContextPlan::empty_for_tests("managed_ctx_empty");
        assert!(plan.is_empty());
        plan.plan_digest = format!("sha256:{}", "f".repeat(64));
        assert_eq!(
            plan.validate(),
            Err("managed context plan digest is invalid")
        );

        let mut source_plan = ManagedKernelContextPlan::source_project_for_tests(
            "managed_ctx_source",
            "realm-1",
            "source-kernel",
            &"a".repeat(64),
            "project-chariox",
        );
        assert!(!source_plan.is_empty());
        source_plan.source = None;
        assert_eq!(
            source_plan.validate(),
            Err("managed context plan source selection is inconsistent")
        );
    }

    #[test]
    fn context_plan_parses_the_complete_cloud_wire_shape() {
        let plan: ManagedKernelContextPlan = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "contextId": "managed_ctx_complete",
            "planDigest": "sha256:c6f2783e1b75d88360fe4e2b4fb8431f827fa05e4dc365f6c98602555ceb168c",
            "source": {
                "sourceTargetId": "source-target-test",
                "relayRealmId": "realm-1",
                "machineId": "source-machine-test",
                "kernelId": "source-kernel",
                "keyThumbprint": "a".repeat(64),
            },
            "kernelContext": "source_kernel",
            "developmentSetup": {
                "kind": "source_project",
                "projectId": "project-chariox",
                "repositories": [
                    {
                        "role": "primary",
                        "workspaceId": "/repo/cloud",
                        "worktreeId": "feature-cloud",
                    },
                    {
                        "role": "supporting",
                        "workspaceId": "/repo/oss",
                        "worktreeId": null,
                    },
                ],
            },
            "providerAccounts": {
                "kind": "selected",
                "accounts": [
                    { "provider": "a", "accountProfile": "a-z" },
                    { "provider": "a", "accountProfile": "b:c" },
                    { "provider": "a:b", "accountProfile": "c" },
                ],
            },
            "gitCredentials": {
                "kind": "selected",
                "credentialIds": ["github", "work"],
            },
        }))
        .expect("parse Cloud context plan");
        plan.validate().expect("validate Cloud context plan");
        let binding = plan.package_binding();
        assert_eq!(binding.context_id, "managed_ctx_complete");
        assert!(matches!(
            binding.kernel_context,
            ManagedContextKernelSelection::SourceKernel
        ));
        assert!(matches!(
            binding.development,
            ManagedContextDevelopmentSelection::SourceProject {
                ref project_id,
                ref repositories,
            } if project_id == "project-chariox"
                && repositories.len() == 2
                && repositories[0].workspace_id == "/repo/cloud"
                && repositories[0].worktree_id.as_deref() == Some("feature-cloud")
                && repositories[1].role == DevelopmentRepositoryRole::Supporting
        ));
        assert!(matches!(
            binding.provider_accounts,
            ManagedContextProviderAccountSelection::Selected { ref accounts }
                if accounts.len() == 3
        ));
        assert!(matches!(
            binding.git_credentials,
            ManagedContextGitCredentialSelection::Selected { ref credential_ids }
                if credential_ids == &["github".to_string(), "work".to_string()]
        ));
        let encoded = serde_json::to_string(&plan).expect("serialize context plan");
        assert!(encoded.contains("\"credentialIds\""));
        assert!(!encoded.contains("credential_ids"));
    }
}
