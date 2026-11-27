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
/// What it returns when the command also ignored the stop request and was killed.
const KILLED_AFTER_TIMEOUT_CODE: i32 = 137;

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
        description: "Run one command (argv, no shell) as root with sudo on this machine. The kernel types the sudo password from credential_id, the owner's command_stdin Vault entry for this machine; you never see it.".into(),
        input_schema: serde_json::json!({"type":"object","required":["argv","credential_id"],"properties":{
            "argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},
            "credential_id":{"type":"string"},
            "machine":{"type":"string","description":"this (default)"},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT_SECONDS}},
            "additionalProperties":false}),
    }
}

/// One request, checked: what runs, where, and the entry's use it must match.
struct Run {
    argv: Vec<String>,
    timeout: Duration,
    machine: Machine,
    sudo: std::path::PathBuf,
    /// Ends the command at its deadline from inside sudo (`timeout`, or
    /// `gtimeout` of Homebrew coreutils), as an absolute path: sudo's own
    /// search path may not hold it.
    timeout_program: std::path::PathBuf,
}

impl Run {
    fn new(
        state: &KernelRuntimeState,
        argv: &serde_json::Value,
        machine: Option<&str>,
        timeout_seconds: Option<u64>,
    ) -> Result<Self, DaemonError> {
        let machine = state.privileged_machine(machine)?;
        Ok(Self {
            argv: super::super::agent_process_watch::command_argv(argv, true)?,
            timeout: command_timeout(timeout_seconds)?,
            // Both programs are found before the password is read.
            sudo: sudo_program()?,
            timeout_program: timeout_program(&machine.name)?,
            machine,
        })
    }

    fn target(&self) -> crate::config::VaultUseTarget {
        crate::config::VaultUseTarget::CommandStdin {
            origin: self.machine.origin.clone(),
        }
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
        let run = Run::new(
            self,
            &args.argv,
            args.machine.as_deref(),
            args.timeout_seconds,
        )?;
        let target = run.target();
        // A leased run's entry is the home Vault's; the home checks its binding.
        if context.home.is_none() {
            check_entry_machine(
                &self.home_runtime_secret_service()?,
                &args.credential_id,
                &run,
            )?;
        }
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
        let outcome = self
            .run_with_password(Some(context), run, &args.credential_id, password)
            .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::to_value(outcome)
                .map_err(|e| error(format!("privileged run result: {e}")))?,
        })
    }

    /// The owner's own run, from the Vault menu or the CLI. The caller has
    /// checked that the request comes from the owner and that the Vault is open.
    pub(crate) async fn owner_privileged_run(
        &self,
        id: &str,
        argv: Vec<String>,
        machine: Option<String>,
        timeout_seconds: Option<u64>,
    ) -> Result<crate::local::PrivilegedRunOutcome, DaemonError> {
        let run = Run::new(
            self,
            &serde_json::json!(argv),
            machine.as_deref(),
            timeout_seconds,
        )?;
        let service = self.home_runtime_secret_service()?;
        check_entry_machine(&service, id, &run)?;
        let password =
            zeroize::Zeroizing::new(service.workspace_secret_input_for_target(id, &run.target())?);
        service.record_vault_activity(
            id,
            &format!("Used by owner for sudo on {}", run.machine.name),
        )?;
        self.run_with_password(None, run, id, password).await
    }

    /// The run itself: the password goes to sudo's stdin and nowhere else, and
    /// everything that comes back is masked for it.
    async fn run_with_password(
        &self,
        context: Option<VaultAgentContext>,
        run: Run,
        credential_id: &str,
        password: Zeroizing<String>,
    ) -> Result<crate::local::PrivilegedRunOutcome, DaemonError> {
        let stdin = Zeroizing::new(format!("{}\n", password.as_str()));
        let masks = vec![(credential_id.to_string(), password)];
        let started = std::time::Instant::now();
        let output = run_command(
            self,
            context,
            super::super::agent_process_watch::KernelCommand {
                launch: super::super::agent_process_watch::KernelLaunch::Privileged {
                    sudo: run.sudo,
                },
                argv: inside_sudo(&run.timeout_program, run.argv, run.timeout),
                cwd: std::path::PathBuf::from("/"),
                env: Vec::new(),
                stdin: Some(stdin),
                timeout: run.timeout + KERNEL_GRACE,
            },
        )
        .await?;
        use super::super::agent_process_watch::KernelCommandExit;
        // `timeout` ends the command with 124, or 137 when it needed SIGKILL; either
        // is a timeout only if the whole time had passed (a command may kill itself).
        let waited = started.elapsed() >= run.timeout;
        let (status, exit_code) = match output.exit {
            KernelCommandExit::Exited(TIMED_OUT_CODE | KILLED_AFTER_TIMEOUT_CODE) if waited => {
                ("timed_out", None)
            }
            KernelCommandExit::TimedOut => ("timed_out", None),
            KernelCommandExit::Exited(code) => ("exited", Some(code)),
            KernelCommandExit::Lost => ("lost", None),
        };
        if let Some(refusal) = sudo_refusal(&output, &run.machine.name) {
            return Err(error(refusal));
        }
        let (stdout, stdout_cut) = masked(&output.stdout, &masks, output.truncated);
        let (stderr, stderr_cut) = masked(&output.stderr, &masks, output.truncated);
        Ok(crate::local::PrivilegedRunOutcome {
            status: status.into(),
            exit_code,
            stdout,
            stderr,
            machine: run.machine.name,
            ran_as: "root".into(),
            truncated: output.truncated || stdout_cut || stderr_cut,
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
}

/// A sudo password is bound to one machine. Naming the origin tells the
/// owner which entry to make when the one given is for another use.
fn check_entry_machine(
    service: &crate::secret::RuntimeSecretService,
    id: &str,
    run: &Run,
) -> Result<(), DaemonError> {
    let declared = service
        .credential(id)?
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.use_binding.as_ref())
        .map(|binding| &binding.target);
    if declared == Some(&run.target()) {
        return Ok(());
    }
    Err(error(format!(
        "entry_wrong_machine: {id} is not the sudo password of {}. Make an entry for it (use: command_stdin, origin {}) or use the entry made for this machine",
        run.machine.name, run.machine.origin
    )))
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

/// What runs inside sudo: `timeout` ends the command at the deadline, three
/// seconds after asking it to stop, and the shell in front gives the program
/// no input. Sudo reads the password from the same pipe only when its rules ask
/// for one (not with NOPASSWD, nor in a kernel that is root), and the program
/// must never get the rest of that pipe either way.
fn inside_sudo(
    timeout_program: &std::path::Path,
    argv: Vec<String>,
    timeout: Duration,
) -> Vec<String> {
    [
        "sh",
        "-c",
        "exec \"$@\" </dev/null",
        "sh",
        &timeout_program.display().to_string(),
        "-k",
        "3",
        &timeout.as_secs().to_string(),
    ]
    .map(str::to_string)
    .into_iter()
    .chain(argv)
    .collect()
}

/// The first of `names` that is a file in `folders`, folders in order and names
/// in order within a folder.
fn find_program(names: &[&str], folders: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    folders
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The kernel's own PATH, then the standard system folders and the places a
/// Mac keeps Homebrew programs.
fn search_folders() -> Vec<std::path::PathBuf> {
    let system = [
        "/usr/bin",
        "/usr/sbin",
        "/bin",
        "/sbin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ]
    .map(std::path::PathBuf::from);
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .chain(system)
        .collect()
}

/// `sudo` from the kernel's own PATH, then the standard system folders.
fn sudo_program() -> Result<std::path::PathBuf, DaemonError> {
    find_program(&["sudo"], &search_folders()).ok_or_else(|| {
        error("sudo_missing: sudo is not installed on this machine. The owner installs it (apt install sudo, dnf install sudo) and you retry")
    })
}

/// `timeout` (GNU coreutils) or `gtimeout` (Homebrew coreutils on a Mac).
fn timeout_program(machine: &str) -> Result<std::path::PathBuf, DaemonError> {
    find_program(&["timeout", "gtimeout"], &search_folders())
        .ok_or_else(|| error(timeout_missing(machine)))
}

fn timeout_missing(machine: &str) -> String {
    format!("timeout_missing: {machine} has no timeout program, which ends a privileged command at its deadline. The owner installs coreutils (apt install coreutils, dnf install coreutils; on a Mac brew install coreutils) and you retry")
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

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn program(dir: &std::path::Path, name: &str) {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_program_that_ends_a_command_is_found_by_either_name_before_sudo_runs() {
        let root = tempfile::tempdir().unwrap();
        let (first, second) = (root.path().join("a"), root.path().join("b"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let folders = [first.clone(), second.clone()];
        // Nothing installed: no program, so no authentication starts.
        assert_eq!(find_program(&["timeout", "gtimeout"], &folders), None);
        // A Mac with Homebrew coreutils has only gtimeout.
        program(&second, "gtimeout");
        assert_eq!(
            find_program(&["timeout", "gtimeout"], &folders),
            Some(second.join("gtimeout"))
        );
        // The first folder wins; within one folder timeout comes before gtimeout.
        program(&first, "gtimeout");
        assert_eq!(
            find_program(&["timeout", "gtimeout"], &folders),
            Some(first.join("gtimeout"))
        );
        program(&first, "timeout");
        assert_eq!(
            find_program(&["timeout", "gtimeout"], &folders),
            Some(first.join("timeout"))
        );
        // A folder entry that is not a file is not a program.
        std::fs::create_dir(second.join("timeout")).unwrap();
        std::fs::remove_file(first.join("timeout")).unwrap();
        std::fs::remove_file(first.join("gtimeout")).unwrap();
        assert_eq!(
            find_program(&["timeout", "gtimeout"], &folders),
            Some(second.join("gtimeout"))
        );
    }

    #[test]
    fn a_missing_timeout_program_is_refused_with_the_next_step() {
        let refusal = timeout_missing("kernel-1");
        assert!(refusal.starts_with("timeout_missing:"), "{refusal}");
        assert!(
            refusal.contains("brew install coreutils") && refusal.contains("apt install coreutils")
        );
        assert!(refusal.contains("kernel-1"));
    }
}
