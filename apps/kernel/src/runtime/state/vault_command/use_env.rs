//! `chariox.vault.use_env`: run a command with Vault values in its environment
//! or stdin. The tool call names only credential ids; this command's own
//! output is masked for the injected values before it is returned. Later
//! reads elsewhere are not scrubbed (UX-first rule: what the agent does with
//! a value it chose to use is its own action).
use super::mask::masked;
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

pub(crate) const VAULT_USE_ENV: &str = "chariox.vault.use_env";
pub(super) const MAX_VARIABLES: usize = 16;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UseArgs {
    argv: serde_json::Value,
    origin: String,
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
        description: "Run one command (argv, no shell) in your workspace with Vault values injected for the declared service origin: env maps variable names to credential_ids, stdin sends one credential's value to standard input. You never see the values; they appear as [vault:<credential_id>] in the returned exit_code, stdout and stderr. Works with credentials whose allowed uses include env, such as those from chariox.vault.capture_command.".into(),
        input_schema: serde_json::json!({"type":"object","required":["argv","origin"],"properties":{
            "origin":{"type":"string","description":"Exact service origin this command uses, such as https://api.example.com"},
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
        let context = self.vault_leased_context(token).await?;
        self.recheck_vault_command_context(&context)?;
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
        let argv = super::super::agent_process_watch::command_argv(&args.argv, true)?;
        let timeout = command_timeout(args.timeout_seconds)?;
        let cwd =
            super::super::agent_process_watch::workspace_cwd(&context.run, args.cwd.as_deref())?;
        let origin = crate::config::vault_use::vault_origin(&args.origin)?;
        let env_target = |name: &str| crate::config::VaultUseTarget::CommandEnvironment {
            name: name.into(),
            origin: origin.clone(),
        };
        let stdin_target = crate::config::VaultUseTarget::CommandStdin {
            origin: origin.clone(),
        };
        let uses = args
            .env
            .iter()
            .map(|(name, id)| (id.as_str(), env_target(name)))
            .chain(
                args.stdin
                    .iter()
                    .map(|id| (id.as_str(), stdin_target.clone())),
            )
            .collect::<Vec<_>>();
        let _unlock = self
            .prepare_workspace_vault_uses(&context, &uses, "vault_use_env")
            .await?;
        self.recheck_vault_command_context(&context)?;
        let mut env = Vec::new();
        for (name, id) in &args.env {
            env.push((
                name.clone(),
                self.read_workspace_vault_value(&context, id, &env_target(name))
                    .await?,
            ));
        }
        let stdin = match &args.stdin {
            Some(id) => Some(
                self.read_workspace_vault_value(&context, id, &stdin_target)
                    .await?,
            ),
            None => None,
        };
        self.recheck_vault_command_context(&context)?;
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
            context.clone(),
            super::super::agent_process_watch::KernelCommand {
                launch: super::super::agent_process_watch::KernelLaunch::Isolated,
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
pub(super) fn variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn variable_names_follow_the_shell_rule() {
        assert!(variable_name("API_KEY") && variable_name("_x1"));
        for refused in ["", "1X", "A-B", "A B", "A=B"] {
            assert!(!variable_name(refused), "{refused}");
        }
    }
}
