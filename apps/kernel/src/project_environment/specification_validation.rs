//! MP-08 / MP-10 / MP-11: typed edits declare needs; Save grants and executes nothing.
use super::*;
use crate::error::DaemonError;
fn text(v: &str) -> Result<(), DaemonError> {
    if v.is_empty()
        || v.len() > 4096
        || v.chars().any(char::is_control)
        || super::detect_index::credential_metadata(v)
    {
        return Err(environment_error(
            "invalid or credential-shaped requirement; choose Vault for private input",
        ));
    }
    Ok(())
}
fn optional(v: &Option<String>) -> Result<(), DaemonError> {
    if let Some(v) = v {
        text(v)?;
    }
    Ok(())
}
fn url(v: &str) -> Result<(), DaemonError> {
    let u = url::Url::parse(v).map_err(|_| environment_error("invalid health URL"))?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(environment_error(
            "health URL must be HTTP(S) without credentials, query or fragment",
        ));
    }
    Ok(())
}
fn timeout(v: u32) -> Result<(), DaemonError> {
    if v == 0 || v > 120_000 {
        Err(environment_error(
            "probe/command timeout must be 1–120000 ms",
        ))
    } else {
        Ok(())
    }
}
fn probe(p: &EnvironmentProbe) -> Result<(), DaemonError> {
    match p {
        EnvironmentProbe::Executable {
            name,
            arguments,
            timeout_ms,
        } => {
            text(name)?;
            timeout(*timeout_ms)?;
            if arguments.len() > 128 {
                return Err(environment_error("probe arguments exceed bounds"));
            }
            for a in arguments {
                text(a)?;
            }
        }
        EnvironmentProbe::Tcp {
            host,
            port,
            timeout_ms,
        } => {
            text(host)?;
            if *port == 0 {
                return Err(environment_error("invalid health port"));
            }
            timeout(*timeout_ms)?;
        }
        EnvironmentProbe::Http {
            url: v,
            expected_status,
            timeout_ms,
        } => {
            url(v)?;
            if !(100..=599).contains(expected_status) {
                return Err(environment_error("invalid expected HTTP status"));
            }
            timeout(*timeout_ms)?;
        }
    }
    Ok(())
}
pub(super) fn validate_typed_requirement(r: &Requirement) -> Result<(), DaemonError> {
    text(&r.title)?;
    match &r.spec {
        RequirementSpec::Services {
            identity,
            probe: p,
            health_expectation,
            target_binding,
            ..
        } => {
            text(identity)?;
            if let Some(p) = p {
                probe(p)?;
            }
            if let Some(EnvironmentHealthExpectation::HttpStatus { status }) = health_expectation {
                if !(100..=599).contains(status) {
                    return Err(environment_error("invalid HTTP health expectation"));
                }
            }
            if let Some(t) = target_binding {
                text(&t.machine_id)?;
                text(&t.target_instance_generation)?;
            }
        }
        RequirementSpec::Accounts {
            linked_profile_ref,
            required_capabilities,
            ..
        } => {
            optional(linked_profile_ref)?;
            for s in required_capabilities {
                text(s)?;
            }
        }
        RequirementSpec::AgentTools {
            registry_ref,
            package_digest,
            scope,
            runtime_requirements,
            vault_refs,
            ..
        } => {
            text(registry_ref)?;
            optional(package_digest)?;
            if *scope != r.scope {
                return Err(environment_error(
                    "Agent-tool scope must match requirement scope",
                ));
            }
            for s in runtime_requirements {
                text(s)?;
            }
            for v in vault_refs {
                text(&v.service)?;
                text(&v.key)?;
            }
        }
        RequirementSpec::CharioxApps {
            app_id,
            version,
            content_digest,
            required_grants,
        } => {
            text(app_id)?;
            optional(version)?;
            optional(content_digest)?;
            for s in required_grants {
                text(s)?;
            }
        }
        RequirementSpec::SetupChecks {
            commands,
            probes,
            source_path,
            source_digest,
            legacy,
            ..
        } => {
            // Legacy recipe contents remain immutable evidence; public typed commands are future execution inputs.
            if legacy.is_none() {
                if commands.len() > 128 || probes.len() > 128 {
                    return Err(environment_error("too many Setup/check steps"));
                }
                for c in commands {
                    text(&c.executable)?;
                    timeout(c.timeout_ms)?;
                    if c.output_limit_bytes == 0
                        || c.output_limit_bytes > 1024 * 1024
                        || c.arguments.len() > 128
                    {
                        return Err(environment_error("command bounds invalid"));
                    }
                    for a in &c.arguments {
                        text(a)?;
                    }
                }
                for p in probes {
                    probe(p)?;
                }
            }
            if let Some(p) = source_path {
                relative_environment_path(p).map_err(environment_error)?;
            }
            optional(source_digest)?;
        }
        RequirementSpec::MachineNeeds {
            platforms,
            minimum_cpus,
            minimum_memory_bytes,
            minimum_free_disk_bytes,
            gpu,
        } => {
            if *minimum_cpus == Some(0)
                || *minimum_memory_bytes == Some(0)
                || *minimum_free_disk_bytes == Some(0)
            {
                return Err(environment_error("Machine needs must be positive"));
            }
            for p in platforms {
                text(&p.os)?;
                text(&p.architecture)?;
            }
            optional(gpu)?;
        }
        RequirementSpec::Software {
            identity,
            version_constraint,
            install_source,
            platform,
            ..
        } => {
            text(identity)?;
            optional(version_constraint)?;
            optional(install_source)?;
            if let Some(p) = platform {
                text(&p.os)?;
                text(&p.architecture)?;
            }
        }
        RequirementSpec::Variables { name, locator } => {
            text(name)?;
            if project_environment_protected_name(name) {
                return Err(environment_error(
                    "protected kernel/provider variable cannot be declared",
                ));
            }
            if let ProjectEnvironmentLocator::ConfigFile { path }
            | ProjectEnvironmentLocator::EnvFile { path, .. } = locator
            {
                text(path)?;
            }
        }
        RequirementSpec::Secrets { name, vault } => {
            text(name)?;
            if let Some(v) = vault {
                text(&v.service)?;
                text(&v.key)?;
            }
        }
        RequirementSpec::Files { .. } => {}
    }
    Ok(())
}
