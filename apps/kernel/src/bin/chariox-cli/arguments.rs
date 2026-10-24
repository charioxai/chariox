//! Validate launcher syntax before a build, logger or runtime can be started.

pub const USAGE: &str = "usage: chariox-cli [options]
       chariox-cli access|sudo|guide|kernels|request|events|watch|files|terminal|app|context|logs|codex|claude|opencode|publication|deployments|deployed|cloud [args]
       chariox-cli serve <package> <port> [options]

Options:
  -h, --help                    Print usage and exit
  --detached                    Open a detached waiting room
  --kernel-url URL              Attach to a kernel WebSocket
  --socket PATH                 Attach to a local kernel socket
  --automation-socket PATH      Expose terminal automation
  --terminal-pairing-link LINK   Pair this terminal (--pairing-link is an alias)
  --relay-url URL               Attach through a relay
  --relay-token TOKEN           Relay authentication (--relay-token-env NAME preferred)
  --target-daemon-id ID          Select the relay kernel
  --target-daemon-alias NAME     Select the relay kernel by alias
  --session REF                 Attach to an existing session
  --create-session              Create a session
  --alias NAME                  Name the new session
  --delete-session REF          Delete a session
  --client-id ID                Select the terminal identity
  --provider NAME               Select the provider
  --model MODEL                 Select the model
  --account-profile PROFILE     Select the provider account
  --effort LEVEL                Select reasoning effort
  --workspace PATH              Select the workspace
  --worktree PATH               Select the worktree

Run a subcommand with --help for its usage.";

// Subcommands validate their own arguments. Ordinary options match the public
// TypeScript parser; only syntax is checked here, with semantic validation there.
pub fn help_requested(args: &[String]) -> Result<bool, String> {
    if args.first().is_some_and(|arg| {
        matches!(
            arg.as_str(),
            "access"
                | "sudo"
                | "guide"
                | "kernels"
                | "request"
                | "events"
                | "watch"
                | "files"
                | "terminal"
                | "app"
                | "context"
                | "logs"
                | "codex"
                | "claude"
                | "opencode"
                | "publication"
                | "deployments"
                | "deployed"
                | "cloud"
                | "serve"
        )
    }) {
        return Ok(false);
    }
    let mut help = false;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--help" | "-h" => help = true,
            "--detached" | "--create-session" => {}
            "--socket"
            | "--automation-socket"
            | "--kernel-url"
            | "--session"
            | "--relay-url"
            | "--relay-token"
            | "--relay-token-env"
            | "--target-daemon-id"
            | "--target-daemon-alias"
            | "--terminal-pairing-link"
            | "--pairing-link"
            | "--delete-session"
            | "--alias"
            | "--client-id"
            | "--model"
            | "--provider"
            | "--account-profile"
            | "--effort"
            | "--workspace"
            | "--worktree" => {
                index += 1;
                if args
                    .get(index)
                    .is_none_or(|value| value.is_empty() || value.starts_with('-'))
                {
                    return Err(format!("missing value for {arg}"));
                }
            }
            _ if arg.trim().starts_with("chariox-terminal-pair-v1.") => {}
            _ => return Err(format!("unknown argument {arg}")),
        }
        index += 1;
    }
    Ok(help)
}

/// `access request` and `access status` name the same holder: the nearest
/// provider harness, unless the caller passes `--holder-pid`.
pub fn needs_access_holder(args: &[String]) -> bool {
    args.first().map(String::as_str) == Some("access")
        && matches!(args.get(1).map(String::as_str), Some("request" | "status"))
        && !args.iter().any(|arg| arg == "--holder-pid")
}

#[cfg(test)]
mod tests {
    use super::{help_requested, needs_access_holder};

    // MP-08 / MP-11: Cloud commands use the ordinary TypeScript parser.
    #[test]
    fn cloud_commands_reach_their_own_parser() {
        for args in [
            vec!["cloud", "login", "--api-url", "https://cloud.example.test"],
            vec!["cloud", "status"],
            vec!["cloud", "kernels"],
            vec!["cloud", "logout"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(help_requested(&args), Ok(false));
        }
    }

    // EA2-26: the event commands of `chariox guide` use the TypeScript parser.
    #[test]
    fn event_commands_reach_their_own_parser() {
        for args in [
            vec!["events", "subscription"],
            vec!["events", "subscribe", "--kernel", "k", "--session", "s"],
            vec!["events", "unsubscribe", "--all"],
            vec!["events", "list", "--since", "3", "--limit", "10"],
            vec!["events", "wait", "--since", "3", "--timeout", "5"],
            vec!["events", "ack", "7"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(help_requested(&args), Ok(false), "{args:?}");
        }
    }

    // EA2-04/05/19: the agent commands of `chariox guide` use the TypeScript parser.
    #[test]
    fn agent_commands_reach_their_own_parser() {
        for args in [
            vec!["guide"],
            vec!["kernels", "--socket", "/tmp/kernel.sock"],
            vec!["request", "--kernel", "b3", "--command-id", "c-1"],
            vec![
                "watch",
                "--kernel",
                "b3",
                "--session",
                "s",
                "--attachment",
                "a",
            ],
            vec!["files", "put", "--kernel", "b3", "./a.bin", "/tmp/a.bin"],
            vec![
                "terminal",
                "send",
                "--kernel",
                "b3",
                "--session",
                "s",
                "--terminal",
                "t",
                "--no-enter",
                "ls",
            ],
            vec![
                "files",
                "get",
                "--kernel",
                "b3",
                "--overwrite",
                "/tmp/a.bin",
                "./a.bin",
            ],
            vec!["access", "status"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(help_requested(&args), Ok(false), "{args:?}");
        }
    }

    // The public usage names exactly the subcommands the launcher forwards.
    #[test]
    fn usage_names_every_forwarded_subcommand_and_no_removed_one() {
        let line = super::USAGE.lines().nth(1).expect("subcommand usage line");
        let listed: Vec<_> = line
            .trim()
            .strip_prefix("chariox-cli ")
            .and_then(|rest| rest.strip_suffix(" [args]"))
            .expect("subcommand list")
            .split('|')
            .collect();
        for name in &listed {
            assert_eq!(
                help_requested(&[(*name).to_owned()]),
                Ok(false),
                "{name} is listed but not forwarded"
            );
        }
        for name in ["kernels", "terminal", "events"] {
            assert!(listed.contains(&name), "usage omits {name}");
        }
        assert!(!listed.contains(&"machines"));
    }

    #[test]
    fn access_status_names_the_same_holder_as_access_request() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert!(needs_access_holder(&args(&[
            "access",
            "request",
            "--no-wait"
        ])));
        assert!(needs_access_holder(&args(&["access", "status"])));
        assert!(!needs_access_holder(&args(&[
            "access",
            "status",
            "--holder-pid",
            "7"
        ])));
        assert!(!needs_access_holder(&args(&["access", "list"])));
    }
}
