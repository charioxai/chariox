//! VAUDIT S4: `chariox.vault.capture_file` stores a secret a tool already wrote
//! into a workspace file (`.env.local`, a downloaded JSON key); optionally the
//! plaintext in the file becomes a `${chariox:<credential_id>}` handle.
//! `chariox.vault.write_file` writes a file from a template whose placeholders
//! the kernel fills with Vault values. The agent's own later reads of a file
//! it asked for are its own action (UX-first rule), so nothing is scrubbed.
use super::capture::{
    capture_binding_schema, captured_handle, committed, extract, extract_pattern, service_host,
    Captured, Extract,
};
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use zeroize::Zeroizing;

pub(crate) const VAULT_CAPTURE_FILE: &str = "chariox.vault.capture_file";
pub(crate) const VAULT_WRITE_FILE: &str = "chariox.vault.write_file";
const FILE_LIMIT: u64 = 1 << 20;
const MAX_PLACEHOLDERS: usize = 32;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureFileArgs {
    request_id: String,
    path: String,
    host: String,
    use_binding: crate::config::VaultUseBinding,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    extract: Extract,
    #[serde(default)]
    replace_with_handle: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFileArgs {
    path: String,
    template: String,
    values: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    mode: Option<String>,
}

pub(crate) fn capture_file_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_CAPTURE_FILE.into(),
        description: "store a secret that a tool wrote into a file in your workspace (for example .env.local or a downloaded key) in the owner's Vault, without reading the file yourself. The result is an opaque credential_id with your label. extract picks the value: {\"kind\":\"whole\"} (whole file, trimmed; the default), {\"kind\":\"json\",\"pointer\":\"/private_key\"} or {\"kind\":\"regex\",\"pattern\":\"^API_KEY=(.+)$\",\"group\":1}. With replace_with_handle the value in the file becomes ${chariox:<credential_id>}. host names the source service. use_binding declares one exact command environment, stdin or workspace file destination. Retrying with the same request_id returns the same credential.".into(),
        input_schema: serde_json::json!({"type":"object","required":["request_id","path","host","use_binding"],"properties":{
            "use_binding": capture_binding_schema(),
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "path":{"type":"string","description":"File inside your workspace, relative to it"},
            "host":{"type":"string","description":"Host of the service the secret is for, such as api.example.com"},
            "label":{"type":"string","maxLength":200},
            "extract":{"type":"object","required":["kind"],"properties":{
                "kind":{"type":"string","enum":["whole","json","regex"]},
                "pointer":{"type":"string"},
                "pattern":{"type":"string","maxLength":1024},
                "group":{"type":"integer","minimum":0}},"additionalProperties":false},
            "replace_with_handle":{"type":"boolean","default":false}},
            "additionalProperties":false}),
    }
}

pub(crate) fn write_file_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_WRITE_FILE.into(),
        description: "write a file in your workspace (for example .env) from a template: every {{NAME}} in template is replaced by the Vault value of values[NAME] (a credential_id). You never see the values. Works with credentials whose allowed uses include file, such as those from chariox.vault.capture_command or chariox.vault.capture_file. mode is octal, default \"600\". Returns the path and size.".into(),
        input_schema: serde_json::json!({"type":"object","required":["path","template","values"],"properties":{
            "path":{"type":"string","description":"File inside your workspace, relative to it; replaced if it exists"},
            "template":{"type":"string","maxLength":65536},
            "values":{"type":"object","additionalProperties":{"type":"string"},"maxProperties":MAX_PLACEHOLDERS,
                "description":"Placeholder NAME -> credential_id"},
            "mode":{"type":"string","pattern":"^0?[0-7]{3}$","default":"600"}},
            "additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn vault_capture_file(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_agent_context(token)?;
        self.recheck_vault_command_context(&context)?;
        let args: CaptureFileArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault capture arguments: {e}")))?;
        if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
            return Err(error("request_id must be 1-128 characters"));
        }
        if args.label.as_ref().is_some_and(|label| label.len() > 200) {
            return Err(error("label must be at most 200 characters"));
        }
        args.use_binding.target.validate()?;
        let host = service_host(&args.host)?;
        let pattern = extract_pattern(&args.extract)?;
        let turn = &context;
        let handle = captured_handle(turn, &args.request_id);
        let _claim = self.owned.vault_capture_claims.claim(&handle).await?;
        self.recheck_vault_command_context(&context)?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            return committed(&existing, turn, &host, &args.use_binding);
        }
        validate_capture_binding(&context.run, &args.use_binding)?;
        let path = existing_workspace_file(&context.run, &args.path)?;
        let content = Zeroizing::new(
            std::fs::read(&path).map_err(|e| error(format!("cannot read {}: {e}", args.path)))?,
        );
        let secret = extract(&content, &args.extract, pattern.as_ref())?;
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&turn.session_id, &turn.agent_id, "vault_capture")
            .await?;
        self.recheck_vault_command_context(&context)?;
        let mut result = self
            .store_captured(
                &context,
                &registry,
                Captured {
                    handle: handle.clone(),
                    host,
                    label: args.label,
                    use_binding: args.use_binding,
                },
                &secret,
            )
            .await?;
        if args.replace_with_handle {
            // The file was read before the Vault wait; replace in what is there now.
            let current = Zeroizing::new(std::fs::read_to_string(&path).map_err(|e| {
                error(format!(
                    "stored as {handle}, but cannot reread {}: {e}",
                    args.path
                ))
            })?);
            let replaced = current.replace(secret.as_str(), &format!("${{chariox:{handle}}}"));
            let mode = std::fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600);
            write_atomically(&path, replaced.as_bytes(), mode).map_err(|e| {
                error(format!(
                    "stored as {handle}, but cannot rewrite {}: {e}",
                    args.path
                ))
            })?;
            result.payload["replaced_with_handle"] = serde_json::json!(true);
        }
        Ok(result)
    }

    pub(crate) async fn vault_write_file(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_agent_context(token)?;
        self.recheck_vault_command_context(&context)?;
        let args: WriteFileArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault file arguments: {e}")))?;
        if args.values.is_empty() || args.values.len() > MAX_PLACEHOLDERS {
            return Err(error(format!(
                "values names 1-{MAX_PLACEHOLDERS} placeholders"
            )));
        }
        if args.template.len() > 65_536 {
            return Err(error("template must be at most 64 KiB"));
        }
        if let Some(name) = args.values.keys().find(|name| {
            !placeholder_name(name) || !args.template.contains(&format!("{{{{{name}}}}}"))
        }) {
            return Err(error(format!(
                "placeholder `{name}` must be letters, digits or _ and appear in template as {{{{{name}}}}}"
            )));
        }
        let mode = file_mode(args.mode.as_deref())?;
        let path = new_workspace_file(&context.run, &args.path)?;
        let turn = &context;
        let target = crate::config::VaultUseTarget::File {
            path: path.to_string_lossy().into_owned(),
        };
        for id in args.values.values() {
            self.preflight_workspace_vault_use(&turn.agent_id, id, &target)?;
        }
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&turn.session_id, &turn.agent_id, "vault_write_file")
            .await?;
        self.recheck_vault_command_context(&context)?;
        let mut content = Zeroizing::new(args.template.clone());
        for (name, id) in &args.values {
            let value = self
                .read_workspace_vault_value(&context, id, &target)
                .await?;
            content = Zeroizing::new(content.replace(&format!("{{{{{name}}}}}"), &value));
        }
        self.recheck_vault_command_context(&context)?;
        if new_workspace_file(&context.run, &args.path)? != path {
            return Err(error(
                "the destination changed while waiting; retry the file write",
            ));
        }
        write_atomically(&path, content.as_bytes(), mode)
            .map_err(|e| error(format!("cannot write {}: {e}", args.path)))?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"path": args.path, "bytes": content.len(),
                "mode": format!("{mode:o}"), "placeholders": args.values.keys().collect::<Vec<_>>()}),
        })
    }
}

fn placeholder_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn file_mode(raw: Option<&str>) -> Result<u32, DaemonError> {
    let raw = raw.unwrap_or("600");
    Some(raw)
        .filter(|raw| (1..=4).contains(&raw.len()) && raw.chars().all(|c| ('0'..='7').contains(&c)))
        .and_then(|raw| u32::from_str_radix(raw, 8).ok())
        .filter(|mode| *mode <= 0o777)
        .ok_or_else(|| error("mode must be octal such as 600 or 640"))
}

fn workspace_root(run: &crate::provider::RuntimeProviderRun) -> Result<PathBuf, DaemonError> {
    run.working_directory()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| error("agent workspace unavailable"))
}

/// A regular file inside the workspace (symlinks resolved), at most 1 MiB.
fn existing_workspace_file(
    run: &crate::provider::RuntimeProviderRun,
    relative: &str,
) -> Result<PathBuf, DaemonError> {
    let root = workspace_root(run)?;
    root.join(relative)
        .canonicalize()
        .ok()
        .filter(|p| p.starts_with(&root))
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= FILE_LIMIT))
        .ok_or_else(|| error("path must be a file of at most 1 MiB inside the agent workspace"))
}

/// A file to (re)write: its directory is inside the workspace and the name is
/// not a symlink, so the write cannot land outside.
pub(super) fn new_workspace_file(
    run: &crate::provider::RuntimeProviderRun,
    relative: &str,
) -> Result<PathBuf, DaemonError> {
    let root = workspace_root(run)?;
    let refused = || error("path must name a file inside the agent workspace");
    let joined = root.join(relative);
    let name = joined
        .file_name()
        .filter(|n| *n != "..")
        .ok_or_else(refused)?;
    let directory = joined
        .parent()
        .and_then(|d| d.canonicalize().ok())
        .filter(|d| d.starts_with(&root))
        .ok_or_else(refused)?;
    let path = directory.join(name);
    if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(refused());
    }
    Ok(path)
}

/// Written beside the target and renamed, so a reader never sees half a file.
fn write_atomically(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let temporary = path.with_file_name(format!(
        ".{}.chariox-{:016x}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        rand::random::<u64>()
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn write_file_modes_and_placeholders_are_checked() {
        assert_eq!(file_mode(None).unwrap(), 0o600);
        assert_eq!(file_mode(Some("0640")).unwrap(), 0o640);
        assert_eq!(file_mode(Some("000")).unwrap(), 0);
        for refused in ["", "8", "1777", "rw", "77777"] {
            assert!(file_mode(Some(refused)).is_err(), "{refused}");
        }
        assert!(placeholder_name("API_KEY") && placeholder_name("db1"));
        for refused in ["", "A-B", "{{X}}", "a b"] {
            assert!(!placeholder_name(refused), "{refused}");
        }
    }
}
