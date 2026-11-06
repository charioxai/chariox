//! Chariox registrations (MCP servers and skills) as entries of the Environment: whether this kernel holds one,
//! the keys an MCP reads, and the reads and writes an Import makes. A key's value is a Secret and never appears here.
use super::ExtensionEntry;
use crate::error::DaemonError;
use crate::local::EnvironmentEntryKind;
use crate::mcp::{CharioxMcpRegistry, CharioxMcpServerConfig, CharioxMcpTransportConfig};
use crate::skill::{materialize_skill_package, CharioxSkillPackage, CharioxSkillRegistry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A skill travels in one peer message, so it stays small: text and a few assets.
const MAX_SKILL_BYTES: usize = 3 * 1024 * 1024;

/// The registrations the agents of a Project's rooms hold grants for, as Environment entries, and the
/// variable names the MCPs among them read their keys from.
pub(crate) struct ProjectRegistrations {
    pub mcps: Vec<ExtensionEntry>,
    pub skills: Vec<ExtensionEntry>,
    pub key_names: Vec<String>,
}

/// What an Import moves for an MCP or skill entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Registration {
    Mcp(CharioxMcpServerConfig),
    Skill(CharioxSkillPackage),
}

/// Where a kernel keeps its registrations.
pub(crate) trait RegistrationHost: Send + Sync {
    fn has(&self, kind: EnvironmentEntryKind, name: &str) -> bool;
    /// The registration as another kernel can write it, or `None` when this kernel holds none of that name.
    fn read(&self, kind: EnvironmentEntryKind, name: &str) -> Result<Option<Registration>, String>;
    /// Registers the entry unless its name is held by the time it is published; `false` means it was held and is
    /// left exactly as it is.
    fn install_if_absent(&self, registration: Registration) -> Result<bool, String>;
}

/// The registrations of the user: `~/.chariox/mcps` and `~/.chariox/skills`.
pub(crate) struct UserRegistrations;

fn mcp_registry() -> Result<CharioxMcpRegistry, DaemonError> {
    CharioxMcpRegistry::user_root()
        .map(|root| CharioxMcpRegistry::new(vec![root]))
        .ok_or(DaemonError::InvalidConfig {
            field: "MCP registry root",
            message: "HOME must be set to resolve ~/.chariox/mcps",
        })
}

fn skill_registry() -> Result<CharioxSkillRegistry, DaemonError> {
    CharioxSkillRegistry::user_root()
        .map(|root| CharioxSkillRegistry::new(vec![root]))
        .ok_or(DaemonError::InvalidConfig {
            field: "skill registry root",
            message: "HOME must be set to resolve ~/.chariox/skills",
        })
}

impl RegistrationHost for UserRegistrations {
    fn has(&self, kind: EnvironmentEntryKind, name: &str) -> bool {
        match kind {
            EnvironmentEntryKind::Mcp => mcp_registry()
                .and_then(|registry| registry.get(name))
                .is_ok_and(|found| found.is_some()),
            EnvironmentEntryKind::Skill => skill_registry()
                .and_then(|registry| registry.get(name))
                .is_ok_and(|found| found.is_some()),
            EnvironmentEntryKind::Secret | EnvironmentEntryKind::File => false,
        }
    }

    fn read(&self, kind: EnvironmentEntryKind, name: &str) -> Result<Option<Registration>, String> {
        match kind {
            EnvironmentEntryKind::Mcp => mcp_registry()
                .and_then(|registry| registry.get(name))
                .map(|found| found.map(|config| Registration::Mcp(portable_mcp(config))))
                .map_err(|error| error.to_string()),
            EnvironmentEntryKind::Skill => {
                let package = skill_registry()
                    .and_then(|registry| registry.package(name))
                    .map_err(|error| error.to_string())?;
                if package.as_ref().is_some_and(|package| {
                    package
                        .files
                        .iter()
                        .map(|file| file.content_base64.len())
                        .sum::<usize>()
                        > MAX_SKILL_BYTES * 4 / 3
                }) {
                    return Err("The skill is too large to import".into());
                }
                Ok(package.map(Registration::Skill))
            }
            EnvironmentEntryKind::Secret | EnvironmentEntryKind::File => Ok(None),
        }
    }

    fn install_if_absent(&self, registration: Registration) -> Result<bool, String> {
        let mcps = mcp_registry().map_err(|error| error.to_string())?;
        let skills = skill_registry().map_err(|error| error.to_string())?;
        install_if_absent(&mcps, &skills, registration)
    }
}

/// The write behind `RegistrationHost::install_if_absent`, over the registries it lands in.
pub(crate) fn install_if_absent(
    mcps: &CharioxMcpRegistry,
    skills: &CharioxSkillRegistry,
    registration: Registration,
) -> Result<bool, String> {
    match registration {
        Registration::Mcp(config) => mcps
            .install_if_absent(&config)
            .map(|installed| installed.is_some())
            .map_err(|error| error.to_string()),
        Registration::Skill(package) => {
            let root = skills
                .primary_root()
                .map_err(|error| error.to_string())?
                .clone();
            std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
            let incoming = root.join(format!(
                ".incoming-{}-{}",
                std::process::id(),
                crate::session::unix_epoch_ms()
            ));
            std::fs::create_dir_all(&incoming).map_err(|error| error.to_string())?;
            let installed = materialize_skill_package(&incoming, &package)
                .and_then(|dir| skills.install_if_absent_from_path(&dir));
            let _ = std::fs::remove_dir_all(&incoming);
            installed
                .map(|installed| installed.is_some())
                .map_err(|error| error.to_string())
        }
    }
}

fn variable_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

// Project Secret names allow 128 bytes; leave room for any usize collision suffix.
fn key_variable(name: &str) -> String {
    const BASE_BYTES: usize = 100;
    let mut variable = variable_name(name);
    if variable.as_bytes().first().is_some_and(u8::is_ascii_digit)
        || super::record::project_environment_protected_name(&variable)
    {
        variable.insert(0, '_');
    }
    if variable.len() > BASE_BYTES {
        // Retain distinct long names instead of collapsing their shared prefixes.
        let hash: String = Sha256::digest(variable.as_bytes())
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02X}"))
            .collect();
        variable.truncate(BASE_BYTES - 17);
        variable.push('_');
        variable.push_str(&hash);
    }
    variable
}

/// A registration whose keys come from the environment of the process that runs it, which on every kernel carries the
/// Project's Secrets: a credential handle exists on one kernel only, an environment variable name travels.
pub(crate) fn portable_mcp(mut config: CharioxMcpServerConfig) -> CharioxMcpServerConfig {
    let prefix = variable_name(&config.name);
    let token = key_variable(&format!("{prefix}_TOKEN"));
    match &mut config.transport {
        CharioxMcpTransportConfig::Stdio {
            credential_env,
            env_vars,
            ..
        } => {
            for (variable, _) in std::mem::take(credential_env) {
                if !env_vars.contains(&variable) {
                    env_vars.push(variable);
                }
            }
        }
        CharioxMcpTransportConfig::StreamableHttp {
            bearer_token_env_var,
            bearer_token_credential,
            credential_http_headers,
            env_http_headers,
            ..
        } => {
            if bearer_token_credential.take().is_some() && bearer_token_env_var.is_none() {
                *bearer_token_env_var = Some(token);
            }
            let mut names: std::collections::BTreeSet<_> = bearer_token_env_var
                .iter()
                .chain(env_http_headers.values())
                .cloned()
                .collect();
            for (header, _) in std::mem::take(credential_http_headers) {
                // An explicit environment header already takes precedence over a credential header at launch.
                if env_http_headers.contains_key(&header) {
                    continue;
                }
                let base = key_variable(&format!("{prefix}_HEADER_{}", variable_name(&header)));
                let mut variable = base.clone();
                let mut suffix = 2;
                while !names.insert(variable.clone()) {
                    variable = format!("{base}_{suffix}");
                    suffix += 1;
                }
                env_http_headers.insert(header, variable);
            }
        }
    }
    config
}

/// Names of the environment variables an MCP reads its keys from: the Secret rows it needs.
pub(crate) fn mcp_key_names(config: &CharioxMcpServerConfig) -> Vec<String> {
    let mut names = match portable_mcp(config.clone()).transport {
        CharioxMcpTransportConfig::Stdio { env_vars, .. } => env_vars,
        CharioxMcpTransportConfig::StreamableHttp {
            bearer_token_env_var,
            env_http_headers,
            ..
        } => bearer_token_env_var
            .into_iter()
            .chain(env_http_headers.into_values())
            .collect(),
    };
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::CharioxMcpCredentialBinding;

    #[test]
    fn a_credential_handle_becomes_the_variable_that_carries_the_key() {
        let mut config = CharioxMcpServerConfig::stdio("notion", "npx", vec!["-y".into()]);
        if let CharioxMcpTransportConfig::Stdio { credential_env, .. } = &mut config.transport {
            credential_env.insert(
                "NOTION_TOKEN".into(),
                CharioxMcpCredentialBinding {
                    credential: "cred-1".into(),
                },
            );
        }
        let portable = portable_mcp(config.clone());
        assert!(matches!(
            &portable.transport,
            CharioxMcpTransportConfig::Stdio { credential_env, env_vars, .. }
                if credential_env.is_empty() && env_vars == &["NOTION_TOKEN".to_string()]
        ));
        assert_eq!(mcp_key_names(&config), vec!["NOTION_TOKEN".to_string()]);

        let mut http =
            CharioxMcpServerConfig::streamable_http("My-Api", "https://example.invalid/mcp");
        if let CharioxMcpTransportConfig::StreamableHttp {
            bearer_token_credential,
            ..
        } = &mut http.transport
        {
            *bearer_token_credential = Some("cred-2".into());
        }
        assert_eq!(mcp_key_names(&http), vec!["MY_API_TOKEN".to_string()]);
    }
    #[test]
    fn credential_headers_get_distinct_portable_names_and_keep_explicit_environment_bindings() {
        let mut config =
            CharioxMcpServerConfig::streamable_http("api", "https://example.invalid/mcp");
        if let CharioxMcpTransportConfig::StreamableHttp {
            credential_http_headers,
            env_http_headers,
            ..
        } = &mut config.transport
        {
            for header in ["X-Api-Key", "X_Api_Key", "X-Explicit"] {
                credential_http_headers.insert(
                    header.into(),
                    CharioxMcpCredentialBinding {
                        credential: "source-only-handle".into(),
                    },
                );
            }
            env_http_headers.insert("X-Explicit".into(), "EXPLICIT_KEY".into());
        }
        let portable = portable_mcp(config);
        if let CharioxMcpTransportConfig::StreamableHttp {
            credential_http_headers,
            env_http_headers,
            ..
        } = &portable.transport
        {
            assert!(credential_http_headers.is_empty());
            assert_eq!(env_http_headers["X-Api-Key"], "API_HEADER_X_API_KEY");
            assert_eq!(env_http_headers["X_Api_Key"], "API_HEADER_X_API_KEY_2");
            assert_eq!(env_http_headers["X-Explicit"], "EXPLICIT_KEY");
        } else {
            panic!("HTTP registration");
        }
        assert_eq!(mcp_key_names(&portable).len(), 3);
    }
    #[test]
    fn generated_http_key_names_fit_the_project_secret_contract() {
        for name in [
            "9-api".to_string(),
            "a".repeat(128),
            "chariox-tools".to_string(),
            "dyld-service".to_string(),
        ] {
            let mut config =
                CharioxMcpServerConfig::streamable_http(name, "https://example.invalid/mcp");
            if let CharioxMcpTransportConfig::StreamableHttp {
                bearer_token_credential,
                credential_http_headers,
                ..
            } = &mut config.transport
            {
                *bearer_token_credential = Some("source-token".into());
                for header in [
                    "X-Api-Key".to_string(),
                    format!("X-{}-One", "A".repeat(200)),
                    format!("X-{}-Two", "A".repeat(200)),
                ] {
                    credential_http_headers.insert(
                        header,
                        CharioxMcpCredentialBinding {
                            credential: "source-header".into(),
                        },
                    );
                }
            }
            let names = mcp_key_names(&config);
            assert_eq!(names.len(), 4, "each credential keeps its own binding");
            for variable in &names {
                assert!(
                    super::super::record::environment_variable_name(variable)
                        && !super::super::record::project_environment_protected_name(variable),
                    "generated Secret name is invalid: {variable}"
                );
            }
            let portable = portable_mcp(config);
            assert_eq!(mcp_key_names(&portable), names);
            assert_eq!(portable_mcp(portable.clone()), portable);
        }
    }
}
