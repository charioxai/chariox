//! `chariox.privileged.run`: the kernel runs one command with `sudo` as its own
//! user, outside the agent's sandbox, and feeds the sudo password from a Vault
//! entry by handle. The agent names the entry and the command; it never holds
//! the password, and the output it gets back is masked like every Vault tool's.
//! The sandbox is untouched: only the kernel's own launch can elevate.
use super::mask::masked;
use super::*;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use zeroize::Zeroizing;

pub(crate) const PRIVILEGED_RUN: &str = "chariox.privileged.run";
/// The program inside sudo ends the command itself at the timeout: the kernel
/// is not root and could not stop a root process. The kernel's own deadline
/// follows later, for sudo that never starts or never answers.
const KERNEL_GRACE: Duration = Duration::from_secs(15);
/// `timeout`'s exit code when it had to end the command.
const TIMED_OUT_CODE: i32 = 124;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    argv: serde_json::Value,
    credential_id: String,
    #[serde(default)]
    machine: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

pub(crate) fn privileged_run_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: PRIVILEGED_RUN.into(),
        description: "Run one command (argv, no shell) as root with sudo on this machine, outside your sandbox. The kernel types the sudo password from the Vault entry credential_id; you never see it, and it appears as [vault:<credential_id>] in the returned status, exit_code, stdout and stderr. The entry is the owner's sudo password for this machine: a command_stdin entry for the machine origin the refusal names when it does not match.".into(),
        input_schema: serde_json::json!({"type":"object","required":["argv","credential_id"],"properties":{
            "argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64,
                "description":"The program and its arguments, run as root"},
            "credential_id":{"type":"string","description":"Vault entry holding the sudo password of this machine"},
            "machine":{"type":"string","description":"this (default): the machine you run on"},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT_SECONDS,"default":DEFAULT_TIMEOUT_SECONDS}},
            "additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn privileged_run(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_leased_context(token).await?;
        self.recheck_vault_command_context(&context)?;
        let args: RunArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid privileged run arguments: {e}")))?;
        let argv = super::super::agent_process_watch::command_argv(&args.argv, true)?;
        let timeout = command_timeout(args.timeout_seconds)?;
        let machine = self.privileged_machine(args.machine.as_deref())?;
        let target = crate::config::VaultUseTarget::CommandStdin {
            origin: machine.origin.clone(),
        };
        self.check_entry_machine(&context, &args.credential_id, &target, &machine)?;
        let sudo = sudo_program()?;
        let _unlock = self
            .prepare_workspace_vault_uses(
                &context,
                &[(args.credential_id.as_str(), target.clone())],
                "privileged_run",
            )
            .await?;
        self.recheck_vault_command_context(&context)?;
        let password = self
            .read_workspace_vault_value(&context, &args.credential_id, &target)
            .await?;
        self.recheck_vault_command_context(&context)?;
        let stdin = Zeroizing::new(format!("{}\n", password.as_str()));
        let masks = vec![(args.credential_id.clone(), password)];
        let output = run_command(
            self,
            context,
            super::super::agent_process_watch::KernelCommand {
                launch: super::super::agent_process_watch::KernelLaunch::Privileged { sudo },
                argv: ending_at_timeout(argv, timeout),
                cwd: std::path::PathBuf::from("/"),
                env: Vec::new(),
                stdin: Some(stdin),
                timeout: timeout + KERNEL_GRACE,
            },
        )
        .await?;
        use super::super::agent_process_watch::KernelCommandExit;
        let (status, exit_code) = match output.exit {
            KernelCommandExit::Exited(TIMED_OUT_CODE) | KernelCommandExit::TimedOut => {
                ("timed_out", None)
            }
            KernelCommandExit::Exited(code) => ("exited", Some(code)),
            KernelCommandExit::Lost => ("lost", None),
        };
        if let Some(refusal) = sudo_refusal(&output, &machine.name) {
            return Err(error(refusal));
        }
        let (stdout, stdout_cut) = masked(&output.stdout, &masks);
        let (stderr, stderr_cut) = masked(&output.stderr, &masks);
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"status": status, "exit_code": exit_code,
                "stdout": stdout, "stderr": stderr, "machine": machine.name,
                "ran_as": "root",
                "truncated": output.truncated || stdout_cut || stderr_cut}),
        })
    }

    /// The machine a run is for: this kernel, named by its id. Another machine
    /// of the owner needs its own kernel to run the command.
    fn privileged_machine(&self, named: Option<&str>) -> Result<Machine, DaemonError> {
        let own = self.owned.config_projection.snapshot().daemon_id;
        match named {
            None | Some("this") => Machine::of(&own),
            Some(other) if other == own => Machine::of(&own),
            Some(other) => Err(error(format!(
                "machine_unknown: this kernel can run a privileged command only on its own machine ({own}); {other} is not it"
            ))),
        }
    }

    /// A sudo password is bound to one machine. Naming the origin tells the
    /// owner which entry to make when the one given is for another use.
    fn check_entry_machine(
        &self,
        context: &VaultAgentContext,
        id: &str,
        target: &crate::config::VaultUseTarget,
        machine: &Machine,
    ) -> Result<(), DaemonError> {
        // A leased run's entry is the home Vault's; the home checks its binding.
        if context.home.is_some() {
            return Ok(());
        }
        let service = self.home_runtime_secret_service()?;
        let declared = service
            .credential(id)?
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.use_binding.as_ref())
            .map(|binding| &binding.target);
        if declared == Some(target) {
            return Ok(());
        }
        Err(error(format!(
            "entry_wrong_machine: {id} is not the sudo password of {}. Make an entry for it (use: command_stdin, origin {}) or use the entry made for this machine",
            machine.name, machine.origin
        )))
    }
}

/// The machine a sudo password belongs to: the origin the entry is bound to.
struct Machine {
    name: String,
    origin: String,
}

impl Machine {
    fn of(kernel_id: &str) -> Result<Self, DaemonError> {
        Ok(Self {
            origin: crate::config::vault_use::vault_origin(&format!(
                "https://{kernel_id}.machine.chariox.invalid"
            ))?,
            name: kernel_id.to_string(),
        })
    }
}

/// `timeout` runs as root inside sudo and ends the command at the deadline,
/// three seconds after asking it to stop.
fn ending_at_timeout(argv: Vec<String>, timeout: Duration) -> Vec<String> {
    ["timeout", "-k", "3", &timeout.as_secs().to_string()]
        .map(str::to_string)
        .into_iter()
        .chain(argv)
        .collect()
}

/// `sudo` from the kernel's own PATH, then the standard system folders.
fn sudo_program() -> Result<std::path::PathBuf, DaemonError> {
    let system = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"].map(std::path::PathBuf::from);
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .chain(system)
        .map(|folder| folder.join("sudo"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            error("sudo_missing: sudo is not installed on this machine. The owner installs it (apt install sudo, dnf install sudo) and you retry")
        })
}

/// Sudo's own refusal, read from its first words: it ran nothing, so the
/// command's output stays empty. The exit code is returned raw in every other
/// case. Each refusal says what to do next and never repeats a value.
pub(in crate::runtime::state) fn sudo_refusal(
    output: &super::super::agent_process_watch::KernelCommandOutput,
    machine: &str,
) -> Option<String> {
    use super::super::agent_process_watch::KernelCommandExit;
    if !matches!(output.exit, KernelCommandExit::Exited(code) if code != 0)
        || !output.stdout.is_empty()
    {
        return None;
    }
    let said = String::from_utf8_lossy(&output.stderr).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| said.contains(word));
    let name = machine;
    if has(&["incorrect password attempt", "sorry, try again"]) {
        Some(format!("wrong_password: the Vault entry is not the sudo password of the kernel's user on {name}. Ask the owner to update the entry, then retry"))
    } else if has(&["is not in the sudoers file", "may not run sudo"]) {
        Some(format!("not_in_sudoers: the kernel's user may not use sudo on {name}. The owner adds that user to sudoers on the machine"))
    } else if has(&["is not allowed to execute", "is not allowed to run"]) {
        Some(format!("command_not_allowed: sudoers on {name} does not allow this command for the kernel's user. The owner widens the rule or you use an allowed command"))
    } else if has(&["you must have a tty", "a terminal is required"]) {
        Some(format!("needs_terminal: sudoers on {name} requires a terminal for the kernel's user. The owner adds `Defaults:<user> !requiretty` and you retry"))
    } else if has(&["no new privileges"]) {
        Some(format!("kernel_cannot_elevate: the kernel service on {name} may not gain privileges (NoNewPrivileges in its service unit). The host operator removes it and restarts the kernel"))
    } else {
        None
    }
}
