//! MP-08 / MP-10 / MP-11: registry identities and MCP connectivity only; never tools/call or grants.
use super::check::EnvironmentCheckContext;
use super::check_inventory as inventory;
use super::*;
use std::path::PathBuf;
use EnvironmentObservationStatus::{Conflict, Missing, NeedsYourInput, Ready};
fn result(
    s: EnvironmentObservationStatus,
    r: &str,
    m: &str,
) -> (EnvironmentObservationStatus, String, String) {
    (s, r.into(), m.into())
}
pub(super) fn check_tool(
    ctx: &EnvironmentCheckContext,
    requirement: &Requirement,
    roots: &[PathBuf],
    operation: &str,
    deadline: std::time::Instant,
) -> (EnvironmentObservationStatus, String, String) {
    let RequirementSpec::AgentTools {
        tool_kind: kind,
        registry_ref: name,
        package_digest: digest,
        runtime_requirements: runtimes,
        vault_refs,
        ..
    } = &requirement.spec
    else {
        unreachable!("named Check dispatch");
    };
    for runtime in runtimes {
        match inventory::version_until(runtime, roots, deadline) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return result(
                    Missing,
                    "tool_runtime_absent",
                    "Required Agent-tool runtime is absent",
                )
            }
            Err(_) => {
                return result(
                    NeedsYourInput,
                    "tool_runtime_unproven",
                    "Agent-tool runtime requires a supported named probe",
                )
            }
        }
    }
    if !vault_refs.is_empty() {
        let Ok(vault) = crate::secret::project_environment_vault(&ctx.config) else {
            return result(
                NeedsYourInput,
                "vault_required",
                "Agent-tool Vault authorization is unavailable",
            );
        };
        if vault_refs.iter().any(|v| {
            vault
                .find_secret(&v.service, &v.key)
                .ok()
                .flatten()
                .map(zeroize::Zeroizing::new)
                .is_none_or(|v| v.is_empty())
        }) {
            return result(
                NeedsYourInput,
                "vault_required",
                "Required named Agent-tool Vault input is unavailable",
            );
        }
    }
    match kind {
        AgentToolKind::Mcp => {
            let Ok(paths) = crate::runtime::capability_registry::mcp_registry_roots(None) else {
                return result(Missing, "tool_absent", "Agent-tool registry unavailable");
            };
            let registry = crate::mcp::CharioxMcpRegistry::new(paths);
            let Ok(Some(backing)) = registry.get(name) else {
                return result(
                    Missing,
                    "tool_absent",
                    "Required MCP is not registered on this kernel",
                );
            };
            if let crate::mcp::CharioxMcpTransportConfig::Stdio { command, args, .. } =
                &backing.transport
            {
                let name = std::path::Path::new(command)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if matches!(name, "npx" | "bunx" | "uvx" | "pipx")
                    || (matches!(
                        name,
                        "npm" | "pnpm" | "yarn" | "bun" | "uv" | "pip" | "pip3"
                    ) && args.iter().any(|a| {
                        matches!(a.as_str(), "install" | "add" | "exec" | "dlx" | "x" | "run")
                    }))
                {
                    return result(NeedsYourInput,"tool_runtime_install_required","MCP launcher may install or run package scripts; provide an existing admitted runtime before Check");
                }
            }
            if !backing.enabled {
                return result(
                    NeedsYourInput,
                    "tool_disabled",
                    "Registered MCP is disabled; Check does not enable it",
                );
            }
            if digest
                .as_ref()
                .is_some_and(|d| backing.definition_hash().ok().as_ref() != Some(d))
            {
                return result(
                    Conflict,
                    "tool_identity_conflict",
                    "Same-name MCP differs from the required package identity",
                );
            }
            let Ok(credentials) = crate::credential::load_user_credentials() else {
                return result(
                    NeedsYourInput,
                    "vault_registry_unavailable",
                    "MCP credential registry unavailable",
                );
            };
            let Ok(secrets) = crate::secret::RuntimeSecretService::with_vault_config(
                credentials,
                &ctx.config.user_config.credential_vault,
            ) else {
                return result(
                    NeedsYourInput,
                    "vault_required",
                    "MCP credential service unavailable",
                );
            };
            let Ok(mut backing) = backing.resolve_credential_bindings(&secrets) else {
                return result(
                    NeedsYourInput,
                    "vault_required",
                    "MCP named credentials require authorization",
                );
            };
            // Reserve one second for bounded lease release and a bounded
            // interval for each handshake dispatch. The
            // existing adapter expresses seconds, so never round up a fraction
            // past the operation's remaining budget or create another authority.
            let seconds = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_secs()
                .saturating_sub(1)
                / 3;
            if seconds == 0 {
                return result(
                    NeedsYourInput,
                    "check_budget_exceeded",
                    "Check has insufficient remaining MCP handshake budget; check fewer items",
                );
            }
            let seconds = seconds.min(3);
            backing.startup_timeout_sec = Some(seconds);
            backing.tool_timeout_sec = Some(seconds);
            // Distinct check ownership reuses the provider-native MCP adapter and settles its own leases.
            let owner = format!(
                "environment-check-{operation}-{:032x}",
                rand::random::<u128>()
            );
            // A Check lease never waits behind an agent's tool call or
            // inherits its MCP initialization state.
            backing.name = owner.clone();
            let measured = (|| {
                let initialized=crate::provider::dispatch_provider_mcp_proxy_request_with_excluded_roots(&owner,&owner,&backing,serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"chariox-environment-check","version":"1"}}}),roots).ok()?;
                if initialized.get("error").is_some() || initialized.get("result").is_none() {
                    return None;
                }
                crate::provider::dispatch_provider_mcp_proxy_request_with_excluded_roots(
                    &owner,
                    &owner,
                    &backing,
                    serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                    roots,
                )
                .ok()?;
                let listed = crate::provider::dispatch_provider_mcp_proxy_request_with_excluded_roots(
                    &owner,
                    &owner,
                    &backing,
                    serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
                    roots,
                )
                .ok()?;
                listed.get("result")?.get("tools")?.as_array().map(|_| ())
            })();
            crate::provider::shutdown_provider_mcp_proxy_session(&owner);
            if measured.is_some() {
                result(Ready,"mcp_connected","MCP initialize/tools/list passed; upstream data authorization not tested; no tool called")
            } else {
                result(
                    Missing,
                    "mcp_unreachable",
                    "MCP handshake/tools/list failed; no tool called or repair performed",
                )
            }
        }
        AgentToolKind::Skill | AgentToolKind::InstructionFile => {
            let Ok(paths) = crate::runtime::capability_registry::skill_registry_roots(None) else {
                return result(Missing, "tool_absent", "Agent-tool registry unavailable");
            };
            let registry = crate::skill::CharioxSkillRegistry::new(paths);
            let Ok(Some(package)) = registry.package(name) else {
                return result(
                    Missing,
                    "tool_absent",
                    "Required skill/instruction package is not admitted to this kernel registry",
                );
            };
            if digest.as_ref().is_some_and(|d| d != &package.version_hash) {
                return result(
                    Conflict,
                    "tool_identity_conflict",
                    "Same-name skill/instruction package has a different reviewed digest",
                );
            }
            result(Ready,"tool_registered","Current admitted skill/instruction package identity matches; Check does not grant or activate it")
        }
    }
}
