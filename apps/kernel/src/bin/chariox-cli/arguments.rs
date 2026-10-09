//! Validate launcher syntax before a build, logger or runtime can be started.

pub const USAGE: &str = "usage: chariox-cli [options]
       chariox-cli access|sudo|app|logs|codex|claude|opencode|publication|deployments|deployed [args]
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
                | "app"
                | "logs"
                | "codex"
                | "claude"
                | "opencode"
                | "publication"
                | "deployments"
                | "deployed"
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
