//! `chariox.vault.use_env`: run a command with Vault values in its environment
//! or stdin. The tool call names only credential ids; this command's own
//! output is masked for the injected values before it is returned. Later
//! reads elsewhere are not scrubbed (UX-first rule: what the agent does with
//! a value it chose to use is its own action).
use super::elevation::VaultElevationKind;
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use zeroize::Zeroizing;

pub(crate) const VAULT_USE_ENV: &str = "chariox.vault.use_env";
/// Returned per stream; the tail is kept, where failures usually are.
const RETURNED_BYTES: usize = 32 * 1024;
const MAX_VARIABLES: usize = 16;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UseArgs {
    argv: serde_json::Value,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

pub(crate) fn use_env_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_USE_ENV.into(),
        description: "During your sudo window, run one command (argv, no shell) in your workspace with Vault values injected by the kernel: env maps environment variable names to credential_ids, stdin sends one credential's value to the command's standard input. You never see the values; in this command's returned stdout and stderr they appear as [vault:<credential_id>]. Works with credentials whose allowed uses include env, such as those from chariox.vault.capture_command. Returns exit_code, stdout and stderr.".into(),
        input_schema: serde_json::json!({"type":"object","required":["argv"],"properties":{
            "argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},
            "cwd":{"type":"string","description":"Directory inside your workspace, relative to it"},
            "env":{"type":"object","additionalProperties":{"type":"string"},"maxProperties":MAX_VARIABLES,
                "description":"Environment variable name -> credential_id"},
            "stdin":{"type":"string","description":"credential_id whose value is written to stdin"},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT_SECONDS,"default":DEFAULT_TIMEOUT_SECONDS}},
            "additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn vault_use_env(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let elevation = self.vault_command_context(token, VaultElevationKind::Use)?;
        let args: UseArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault use arguments: {e}")))?;
        if args.env.is_empty() && args.stdin.is_none() {
            return Err(error("name at least one credential in env or stdin"));
        }
        if args.env.len() > MAX_VARIABLES {
            return Err(error(format!(
                "at most {MAX_VARIABLES} environment variables"
            )));
        }
        if let Some(name) = args.env.keys().find(|name| !variable_name(name)) {
            return Err(error(format!(
                "`{name}` is not an environment variable name"
            )));
        }
        let argv = super::super::agent_process_watch::command_argv(&args.argv)?;
        let timeout = command_timeout(args.timeout_seconds)?;
        let cwd =
            super::super::agent_process_watch::workspace_cwd(&elevation.run, args.cwd.as_deref())?;
        let turn = &elevation.turn;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        for id in args.env.values().chain(&args.stdin) {
            require_session_scope(&registry, id, &turn.session_id)?;
        }
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&turn.session_id, &turn.agent_id, "vault_use_env")
            .await?;
        self.recheck_vault_elevation(token, &elevation.turn)?;
        let service = self.home_runtime_secret_service()?;
        let value = |id: &str| service.command_secret_input(id).map(Zeroizing::new);
        let env = args
            .env
            .iter()
            .map(|(name, id)| Ok((name.clone(), value(id)?)))
            .collect::<Result<Vec<_>, DaemonError>>()?;
        let stdin = args.stdin.as_deref().map(value).transpose()?;
        let mut masks = args
            .env
            .values()
            .zip(env.iter().map(|(_, v)| v))
            .chain(args.stdin.iter().zip(stdin.iter()))
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect::<Vec<_>>();
        // Longest first, so a value containing another is masked whole.
        masks.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
        let output = run_command(
            self,
            elevation.run.clone(),
            super::super::agent_process_watch::KernelCommand {
                argv,
                cwd,
                env,
                stdin,
                timeout,
            },
        )
        .await?;
        use super::super::agent_process_watch::KernelCommandExit;
        let (status, exit_code) = match output.exit {
            KernelCommandExit::Exited(code) => ("exited", Some(code)),
            KernelCommandExit::TimedOut => ("timed_out", None),
            KernelCommandExit::Lost => ("lost", None),
        };
        let (stdout, stdout_cut) = masked(&output.stdout, &masks);
        let (stderr, stderr_cut) = masked(&output.stderr, &masks);
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"status": status, "exit_code": exit_code,
                "stdout": stdout, "stderr": stderr,
                "truncated": output.truncated || stdout_cut || stderr_cut}),
        })
    }
}

/// An agent's captured values stay in the session that captured them.
pub(super) fn require_session_scope(
    registry: &crate::credential::CharioxCredentialRegistry,
    id: &str,
    session_id: &str,
) -> Result<(), DaemonError> {
    let credential = registry
        .get(id)?
        .ok_or_else(|| error(format!("credential `{id}` does not exist")))?;
    let metadata = credential.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) == Some(super::capture::CAPTURED_KIND)
        && metadata.and_then(|m| m.session_id.as_deref()) != Some(session_id)
    {
        return Err(error(format!(
            "credential `{id}` belongs to another session"
        )));
    }
    Ok(())
}

fn variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Masks the injected values in the whole stream before cutting it, so no
/// partial value survives at the cut; then sanitizes each line.
fn masked(raw: &[u8], masks: &[(String, Zeroizing<String>)]) -> (String, bool) {
    let mut text = Zeroizing::new(String::from_utf8_lossy(raw).into_owned());
    for (id, value) in masks {
        for value in [value.as_str(), value.trim()] {
            if !value.is_empty() && text.contains(value) {
                text = Zeroizing::new(text.replace(value, &format!("[vault:{id}]")));
            }
        }
    }
    let text = text
        .split('\n')
        .map(|line| super::super::agent_process_output::sanitize(line.as_bytes()))
        .collect::<Vec<_>>()
        .join("\n");
    match text
        .char_indices()
        .rev()
        .find(|(i, _)| text.len() - i >= RETURNED_BYTES)
    {
        Some((start, _)) => (text[start..].to_string(), true),
        None => (text, false),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn use_env_masks_every_injected_value_before_cutting() {
        let masks = vec![
            (
                "long".to_string(),
                Zeroizing::new("abc-secret-xyz".to_string()),
            ),
            ("short".to_string(), Zeroizing::new("secret".to_string())),
        ];
        let (text, cut) = masked(b"got abc-secret-xyz and secret\n\x1b[31mred\x1b[0m", &masks);
        assert_eq!(text, "got [vault:long] and [vault:short]\nred");
        assert!(!cut);
        let mut long = vec![b'a'; RETURNED_BYTES * 2];
        long.extend_from_slice(b"secret");
        let (text, cut) = masked(&long, &masks);
        assert!(cut && text.ends_with("[vault:short]") && !text.contains("secret"));
        assert!(variable_name("API_KEY") && variable_name("_x1"));
        for refused in ["", "1X", "A-B", "A B", "A=B"] {
            assert!(!variable_name(refused), "{refused}");
        }
    }
}
