import {
  attachToSessionRequest, cancelActivePromptRequest, createSessionRequest, destroyAgentRequest, endSessionRequest, getSessionStateRequest,
  listProjectsRequest, listSessionsRequest, respondToInteractionRequest, searchWorkspaceDirectoriesRequest, spawnAgentRequest, submitPromptRequest,
} from "./ipc-requests.js"

type Flag = { name: string; value?: string; required?: true; repeat?: true }
type CommandSpec = { name: string; flags: readonly Flag[]; args?: readonly string[]; summary: string; brief?: string; note?: string }

const SOCKET: Flag = { name: "--socket", value: "PATH" }
const HOLDER: Flag = { name: "--holder-pid", value: "PID" }
const KERNEL: Flag = { name: "--kernel", value: "NAME" }
const TIMEOUT: Flag = { name: "--timeout", value: "SECONDS" }
const OVERWRITE: Flag = { name: "--overwrite" }
const SESSION: Flag = { name: "--session", value: "SESSION_ID" }
const DIRECTORY: Flag = { name: "--directory", value: "PATH" }
const TERMINAL: Flag = { name: "--terminal", value: "TERMINAL_ID" }
const HOST_AGENT: Flag = { name: "--host-agent", value: "AGENT_ID" }

export const COMMANDS: readonly CommandSpec[] = [
  { name: "agent settings", flags: [KERNEL, SESSION, SOCKET], args: ["AGENT"], summary: "read an agent's current settings and pending profile", brief: "Read an agent's settings" },
  {
    name: "agent settings set",
    flags: [KERNEL, SESSION, SOCKET, { name: "--command-id", value: "ID" }, { name: "--provider", value: "PROVIDER" }, { name: "--account-profile", value: "PROFILE" }, { name: "--model", value: "MODEL" }, { name: "--effort", value: "EFFORT" }, { name: "--clear-effort" }, { name: "--reason", value: "TEXT" }, { name: "--at-turn-end" }],
    args: ["AGENT"],
    summary: "change an agent's provider profile; --at-turn-end waits for a running turn to finish",
    brief: "Change an agent's provider profile",
  },
  { name: "agent delete", brief: "Delete an agent and its sub-agents", flags: [KERNEL, SESSION, { name: "--dry-run" }, SOCKET], args: ["AGENT"], summary: "delete an agent of the owner's Room, as the owner's own Delete does: the agent and every agent below it, running or not. Prints the list of what goes (running ones marked) and the result; --dry-run prints the list and deletes nothing" },
  { name: "agent deadline", brief: "Change or clear an agent deadline", flags: [KERNEL, SESSION, SOCKET], args: ["AGENT", "DURATION|none"], summary: "extend, shorten or clear an agent deadline; duration starts now (90s, 30m, 2h, 1d)" },
  { name: "agent parent", brief: "Move an agent under another parent", flags: [KERNEL, SESSION, SOCKET], args: ["AGENT", "PARENT|none"], summary: "move an agent of the owner's Room, with its sub-agents, history and context, under another agent of the Room, as the owner's own move in the agent tree; `none` makes it a Room agent. The agent keeps its id and moves from the old parent's tree to the new parent's; the owner's agents still control each other as peers, so this changes the tree, not who may steer it. A cycle and an agent of another Room are refused" },
  { name: "agent resume", brief: "Resume an agent a usage limit or a blocked task stopped", flags: [KERNEL, SESSION, SOCKET], args: ["AGENT"], summary: "go on with an agent paused by a usage limit or whose task is blocked (its row says which, and what unblocks it); `agent resume cancel AGENT` instead leaves it paused or cancels the blocked task" },
  { name: "exec", brief: "Run a command on this kernel, another kernel, or in a Room", flags: [KERNEL, SESSION, { name: "--workspace", value: "PATH" }, DIRECTORY, HOST_AGENT, TIMEOUT, { name: "--stdin-file", value: "KERNEL_PATH" }, { name: "--stdin" }, SOCKET],
    summary: "run argv after --. With no --session and no --workspace it runs on the kernel itself, in no Room, in a terminal of the kernel (stdout and stderr in one stream; --directory PATH, default the kernel user's home; --timeout SECONDS to wait). Its stdin is yours up to 64 KiB when it is a file (chariox exec -- bash -s < script.sh) or a pipe that has data at once (echo hi | chariox exec -- cat); a stdin that stays open and idle is ignored, and --stdin waits for a slow pipe to end. --stdin-file KERNEL_PATH gives a file already on that kernel (chariox files put it first), so a long script goes in one call; an argument is at most 4096 bytes; with --session it runs in that Room and with --workspace in a temporary Room, without a PTY, streaming binary stdin/stdout/stderr live. Returns the command exit code" },
  {
    name: "exec place", brief: "Move a queued command to another kernel",
    flags: [SESSION, { name: "--kernel", value: "NAME", required: true }, { name: "--directory", value: "PATH" }, SOCKET],
    args: ["EXEC_ID"],
    summary: "run a command exec still holds (queued) on another of your kernels; a started command cannot move",
  },
  { name: "whats-new", flags: [KERNEL, SOCKET], summary: "Read this kernel’s agent release note" },
  { name: "guide", brief: "Read the full agent command guide", flags: [], summary: "print this guide" },
  {
    name: "access request", brief: "Request permission to use the kernel",
    flags: [{ name: "--reason", value: "TEXT" }, { name: "--until-exit" }, { name: "--minutes", value: "N" }, { name: "--no-wait" }, HOLDER, SOCKET],
    summary: "ask the owner to approve access and show --reason on the approval card; prints the grant. In a shell with a command timeout use --no-wait, then poll access status until granted. The kernel picks your harness as holder (--holder-pid takes a host pid); in a sandbox, make the kernel socket's directory writable",
  },
  { name: "access status", brief: "Check whether your kernel access is active", flags: [HOLDER, SOCKET], summary: "granted, pending, ended (with the reason) or not_requested" },
  {
    name: "access cover", brief: "Extend your grant to a background process",
    flags: [{ name: "--pid", value: "PID", required: true }, SOCKET],
    summary: "a background process you start (nohup tool &) loses your access when its parent exits: run this with $! right after starting it, or from the process itself; not for a process that forks twice (its pid changes)",
  },
  {
    name: "secret request", brief: "Ask the owner to save a secret in the Vault",
    flags: [{ name: "--label", value: "TEXT", required: true }, { name: "--host", value: "HOST", required: true, repeat: true }, OVERWRITE, { name: "--no-wait" }, TIMEOUT, SOCKET],
    args: ["CREDENTIAL_ID"],
    summary: "ask the user for a value that goes straight into their Vault under CREDENTIAL_ID; prints the link as its first line, then waits for stored, cancelled or timed_out (--no-wait: exit 6)",
    note: "Name the host(s) that will receive the key with --host (repeat it, for example api.example.com or localhost:8080); the owner sees them before typing, and the handle works for no other host. Show the link to the user verbatim and do not open it. Never ask for the value in chat and never type it yourself: you do not receive it. Use the stored value only through its credential handle.",
  },
  { name: "interaction list", brief: "List the owner’s open cards", flags: [KERNEL, SOCKET], summary: "the open cards of the person you work for, in all their Rooms; person_only says why only the person answers a card" },
  {
    name: "interaction answer", brief: "Answer a card for the owner",
    flags: [{ name: "--reply", value: "TEXT" }, KERNEL, SOCKET],
    args: ["INTERACTION_ID", "CHOICE_ID"],
    summary: "answer one of those cards for the person; --reply TEXT with the card's reply_choice_id. Secret entry, Vault unlock, passkeys and payments stay the person's own"
  },
  { name: "notifications list", brief: "List notification subscriptions", flags: [KERNEL, SOCKET, { name: "--kernel-url", value: "URL" }], summary: "list notifications subscribed to on this kernel, with subscription IDs and pause state" },
  { name: "notifications pause", brief: "Pause a notification subscription", flags: [KERNEL, SOCKET, { name: "--kernel-url", value: "URL" }, { name: "--command-id", value: "ID" }], args: ["SUBSCRIPTION_ID"], summary: "pause delivery for a notification subscription" },
  { name: "notifications resume", brief: "Resume a notification subscription", flags: [KERNEL, SOCKET, { name: "--kernel-url", value: "URL" }, { name: "--command-id", value: "ID" }], args: ["SUBSCRIPTION_ID"], summary: "resume delivery for a paused notification subscription" },
  { name: "machine options", brief: "List the machines you can acquire", flags: [KERNEL, TIMEOUT, SOCKET], summary: "the machines you can acquire now: class, region, size, rate per minute and whether each is in stock; one row per class and region" },
  {
    name: "machine acquire", brief: "Acquire a machine",
    flags: [{ name: "--region", value: "auto|REGION" }, { name: "--request-id", value: "ID" }, KERNEL, TIMEOUT, SOCKET],
    args: ["CLASS"],
    summary: "acquire a machine of CLASS (and REGION when the class is offered in several); prints the allocation with its state. Reuse --request-id to retry after a lost answer or a refusal: it returns the machine the first try got and never acquires twice; use a new one only when the answer says the old one was closed. When the person asked to approve agent acquisitions, their card is raised and this waits for the answer",
    note: "Charges continue until the machine is deleted: run machine release when finished.",
  },
  { name: "machine limits", brief: "Read the machine limits", flags: [KERNEL, TIMEOUT, SOCKET], summary: "the account's limits (machines at once, spend per day) and the person's limits for acquiring without asking, with what applies after the account's bound them" },
  {
    name: "machine set-limits", brief: "Change a machine limit",
    flags: [{ name: "--scope", value: "account|no-ask" }, { name: "--machines", value: "N" }, { name: "--spend", value: "AMOUNT" }, KERNEL, TIMEOUT, SOCKET],
    summary: "change machines at once and/or spend per day (AMOUNT in the account's currency) of the account (default) or of the person's no-ask limits. A lower applies at once. A raise needs the person: their /sudo naming it, or their card showing the old and new values, which this waits for",
  },
  { name: "machine status", brief: "Read an acquired machine", flags: [KERNEL, TIMEOUT, SOCKET], args: ["ALLOCATION_REF"], summary: "the allocation's state and its verified kernel target; release it before it is no longer needed to stop charges" },
  { name: "machine release", brief: "Release an acquired machine", flags: [KERNEL, TIMEOUT, SOCKET], args: ["ALLOCATION_REF"], summary: "delete an acquired machine; read machine status until its state is deleted" },
  { name: "secret status", brief: "Check a pending secret request", flags: [SOCKET], args: ["INTERACTION_ID"], summary: "pending, stored, cancelled or timed_out for a secret request you made" },
  { name: "kernels", brief: "List reachable kernels", flags: [SOCKET], summary: "list the kernels this kernel can reach" },
  { name: "kernels restart", brief: "Restart a kernel, or only its browser", flags: [{ name: "--browser" }, KERNEL, TIMEOUT, SOCKET], summary: "restart this kernel, or --kernel NAME, as a hot reload: its terminals and running agents carry on, its Vault keeps its unlock and its deadline, clients reconnect. Answers KernelRestarting once the reload is prepared, or refuses with why and the kernel keeps running. A process needs its kernel access grant or a /sudo window for this, as for any kernel request; an agent without either restarts its own kernel with the chariox.kernels.restart tool (pass kernel for another one). With --browser it restarts only the browser: Chrome and its display start again and every Tab comes back at its URL with its opener, and it answers with the browser state once it is up" },
  {
    name: "request", brief: "Send a JSON request to a kernel",
    flags: [KERNEL, { name: "--command-id", value: "ID" }, { name: "--no-subscribe" }, TIMEOUT, SOCKET],
    summary: "send the one JSON request on stdin; prints the JSON response. --timeout SECONDS (default 30) is how long to wait: a RunShellCommand with timeout_ms needs timeout_ms/1000 + 30",
  },
  {
    name: "watch",
    brief: "Follow a Room’s events on another kernel",
    // watch follows another kernel's events; a local session is read with `request`.
    flags: [{ ...KERNEL, required: true }, { name: "--session", value: "SESSION_ID", required: true }, { name: "--attachment", value: "ATTACHMENT_ID", required: true }, { name: "--from", value: "EVENT_ID" }, SOCKET],
    summary: "print one JSON line per event of that machine's session until the connection is lost",
  },
  {
    name: "terminal open", brief: "Open a persistent terminal or run a command in one",
    flags: [KERNEL, SESSION, HOST_AGENT, DIRECTORY, { name: "--cols", value: "N" }, { name: "--rows", value: "N" }, { name: "--title", value: "TITLE" }, TIMEOUT, SOCKET],
    summary: "open a terminal in that Room (a login shell, or the argv after --); prints terminal.terminal_id; it keeps running when you disconnect. Without --session the terminal is the kernel's own, in no Room, starting in --directory (default the kernel user's home); only the owner and an agent the owner granted access use it",
  },
  { name: "terminal list", brief: "List terminals and their output offsets", flags: [KERNEL, SESSION, TIMEOUT, SOCKET], summary: "the Room's terminals (the kernel's own without --session) with their output offsets; a terminal that shows host_agent runs on that agent's machine: pass that id as --host-agent to send, read, wait and close" },
  {
    name: "terminal send", brief: "Type into a terminal",
    flags: [KERNEL, SESSION, TERMINAL, HOST_AGENT, { name: "--no-enter" }, { name: "--since", value: "OFFSET" }, { name: "--wait-ms", value: "MS" }, TIMEOUT, SOCKET],
    args: ["TEXT"],
    summary: "type TEXT and Enter; prints after_offset, the offset to read from. With --wait-ms it also waits for the output (as read does) in the same call; pass --since (the next_offset of your last read) to make that one round trip",
  },
  {
    name: "terminal read", brief: "Read a terminal’s output",
    flags: [KERNEL, SESSION, TERMINAL, HOST_AGENT, { name: "--since", value: "OFFSET" }, { name: "--wait-ms", value: "MS" }, SOCKET],
    summary: "the output after OFFSET as text and next_offset; gap true means it scrolled out: use list for a fresh offset",
  },
  {
    name: "terminal wait", brief: "Wait for a terminal condition",
    flags: [KERNEL, SESSION, TERMINAL, HOST_AGENT, { name: "--for", value: "finished|waiting_for_input|output_contains=TEXT" }, { name: "--since", value: "OFFSET" }, TIMEOUT, SOCKET],
    summary: "block until that happens or --timeout seconds pass (default 60); prints fired (null on timeout) and the screen. Only output after OFFSET counts: pass send's after_offset as --since",
  },
  { name: "terminal close", brief: "Close a terminal", flags: [KERNEL, SESSION, TERMINAL, HOST_AGENT, TIMEOUT, SOCKET], summary: "end the terminal and everything running in it" },
  { name: "files stat", brief: "Read file metadata", flags: [KERNEL, { name: "--sha256" }, TIMEOUT, SOCKET], args: ["PATH"], summary: "kind, size, modified time and optionally sha256 of a file; partial_size is an unfinished upload" },
  { name: "files ls", brief: "List a directory", flags: [KERNEL, TIMEOUT, SOCKET], args: ["PATH"], summary: "list a directory" },
  { name: "files get", brief: "Download a file", flags: [KERNEL, OVERWRITE, TIMEOUT, SOCKET], args: ["REMOTE_PATH", "LOCAL_PATH"], summary: "download a file, verified by sha256; prints size, bytes received and resumed_from" },
  { name: "files put", brief: "Upload a file", flags: [KERNEL, OVERWRITE, TIMEOUT, SOCKET], args: ["LOCAL_PATH", "REMOTE_PATH"], summary: "upload a file, verified by sha256; prints size, bytes sent and resumed_from" },
  { name: "events subscribe", brief: "Follow a Room, agent or notification source", flags: [KERNEL, SESSION, { name: "--agent", value: "AGENT_ID" }, { name: "--source", value: "SOURCE" }, { name: "--like", value: "GRANT_ID" }, SOCKET], summary: "follow a session or one agent (--kernel and --session), one notification source (--source ID or display name), or take over the targets an earlier live grant of yours follows (--like GRANT_ID), in the durable event inbox" },
  { name: "events unsubscribe", brief: "Stop following a target or source", flags: [KERNEL, { name: "--session", value: "SESSION_ID" }, { name: "--agent", value: "AGENT_ID" }, { name: "--source", value: "SOURCE" }, { name: "--all" }, SOCKET], summary: "remove targets or one followed source (--source ID, or all), or end the subscription with --all" },
  { name: "events subscription", brief: "Show your followed targets and their health", flags: [SOCKET], summary: "show followed targets, followed sources and per-kernel health" },
  { name: "events sources", brief: "List available notification sources", flags: [{ name: "--kernel", value: "KERNEL_ID" }, SOCKET], summary: "list the notification sources your grant can follow across the owner's kernels, optionally filtered by kernel id" },
  { name: "events list", brief: "Read pending events", flags: [{ name: "--since", value: "CURSOR" }, { name: "--limit", value: "N" }, SOCKET], summary: "read pending records, oldest first; save next_cursor" },
  { name: "events wait", brief: "Wait for pending events", flags: [{ name: "--since", value: "CURSOR" }, { name: "--limit", value: "N" }, TIMEOUT, SOCKET], summary: "wait for pending records (default 540 s, at most 540 s: a longer --timeout is refused at once); exit 5 on timeout; a grant that follows nothing is refused at once (exit 3) with how to follow" },
  { name: "events watch", brief: "Stream pending events", flags: [{ name: "--since", value: "CURSOR" }, SOCKET], summary: "print one JSON line per record until interrupted or the subscription ends" },
  { name: "events ack", brief: "Acknowledge events through a cursor", flags: [SOCKET], args: ["CURSOR"], summary: "acknowledge all records through CURSOR" },
  { name: "events mcp", brief: "Answer Chariox questions from a compatible harness", flags: [SOCKET], summary: "MCP server over stdio for a harness that supports elicitation: its tool chariox_next_choice shows the next pending question in the harness's own chooser and sends the answer" },

]

const CLI_COMMANDS = [
  { name: "help", summary: "List commands (also --help or -h)" },
  { name: "--version", summary: "Print the installed version (also -V)" },
  { name: "context", summary: "Read the current agent and Room context" },
  { name: "sudo", summary: "Ask the owner to approve a privileged task" },
  { name: "terminal", summary: "Attach to a terminal" },
  { name: "logs", summary: "Read Chariox logs" },
  { name: "app", summary: "Manage apps and app development" },
  { name: "claude", summary: "Open a native Claude session" },
  { name: "codex", summary: "Open a native Codex session" },
  { name: "opencode", summary: "Open a native OpenCode session" },
  { name: "publication", summary: "Deploy a publication or manage its deployment" },
  { name: "deployments", summary: "Manage deployed workflows (also deployed)" },
  { name: "cloud login", summary: "Sign in to Chariox Cloud (also login)" },
  { name: "cloud logout", summary: "Sign out of Chariox Cloud" },
  { name: "cloud status", summary: "Show the Cloud sign-in status" },
  { name: "cloud kernels", summary: "List kernels on your Cloud account" },
  { name: "cloud setup", summary: "Set up this computer (also setup)" },
]

const COMMAND_GROUPS: readonly [string, readonly string[]][] = [
  ["Run commands", ["exec", "terminal"]],
  ["Files", ["files"]],
  ["Agent events", ["agent", "events", "interaction", "watch"]],
  ["Access and secrets", ["access", "secret", "sudo"]],
  ["Cloud and apps", ["app", "cloud", "deployments", "publication"]],
  ["Terminal client", ["claude", "codex", "logs", "opencode"]],
]

export const ONE_OFF_COMMAND_HINT = "For a one-off command, use chariox exec --kernel NAME -- argv... (omit --kernel for this kernel)."

export function topLevelHelpText(): string {
  const groups = new Map(COMMAND_GROUPS.map(([group]) => [group, [] as string[]]))
  const general: string[] = []
  for (const { name, summary } of [...COMMANDS.map(spec => ({ name: spec.name, summary: spec.brief ?? spec.summary })), ...CLI_COMMANDS]) {
    const verb = name.split(" ", 1)[0]!
    const group = COMMAND_GROUPS.find(([, verbs]) => verbs.includes(verb))?.[0]
    const row = `  ${name.padEnd(22)}  ${summary}`
    ;(group ? groups.get(group)! : general).push(row)
  }
  return [
    "Usage: chariox <command> [arguments]",
    "Run chariox without a command to open the terminal client.",
    "Run chariox guide for agent command details and examples.",
    "",
    "General", ...general,
    ...Array.from(groups, ([title, rows]) => ["", title, ...rows]).flat(),
    "",
  ].join("\n")
}

export const REQUEST_EXAMPLES: readonly { body: object; note: string }[] = [
  { body: listSessionsRequest(), note: "SessionsListed.sessions" },
  { body: listProjectsRequest(), note: "ProjectsListed.projects" },
  { body: searchWorkspaceDirectoriesRequest("NAME", 20), note: "WorkspaceDirectoriesSearched.directories: absolute directory paths on that machine" },
  { body: createSessionRequest("/PATH", "/PATH"), note: "workspace_id and worktree_id are one such directory path. The Room id is SessionCreated.session_id through the gateway and SessionCreated.session.id on this kernel." },
  { body: attachToSessionRequest("SESSION_ID", "CLIENT_NAME"), note: "SessionAttached.attachment.id; client_id is any name you pick" },
  { body: getSessionStateRequest("SESSION_ID"), note: "SessionState: agents, prompts, output" },
  { body: { SpawnAgent: { ...spawnAgentRequest("SESSION_ID", "codex").SpawnAgent, account_profile: "PROFILE_ID" } }, note: "provider is claude, codex or opencode; use the chosen profile_id from ListProviderAccountProfiles, and set model and effort; AgentSpawned.agent.id" },
  { body: submitPromptRequest("SESSION_ID", "ATTACHMENT_ID", null, "PROMPT", []), note: "target_agent_id is an agent id, or null for the default agent" },
  { body: cancelActivePromptRequest("SESSION_ID", "ATTACHMENT_ID", null), note: "stops the running prompt" },
  { body: destroyAgentRequest("SESSION_ID", "AGENT_ID"), note: "removes that agent and EVERY agent below it, running or not (the owner's Delete); AgentDestroyed. `chariox agent delete` lists the subtree first and sends it" },
  {
    body: { RunShellCommand: { session_id: "SESSION_ID", attachment_id: "ATTACHMENT_ID", command: "ls", args: ["-la"], working_directory: "/PATH", timeout_ms: 60000 } },
    note: "runs on that machine; ShellCommandCompleted.result has exit_code, stdout, stderr",
  },
  { body: endSessionRequest("SESSION_ID"), note: "SessionEnded" },
  {
    body: { RequestKernelSudo: { agent_id: "AGENT_ID", prompt: "TASK" } },
    note: "elevates that agent's task on the kernel; waits while the user answers its popup there, so pass --timeout 660; KernelSudoRequested",
  },
  { body: respondToInteractionRequest("SESSION_ID", "INTERACTION_ID", "CHOICE_ID"), note: "InteractionResponded; set custom_reply to the person's text when question.text_reply is present" },
]

const REFUSED = ["events_refused", "acquisition_denied", "approval_required", "secret_link_unavailable", "kernel_sudo_refused", "kernel_sudo_room_owner_local", "kernel_access_human_only", "kernel_access_ended", "kernel_access_not_covered", "kernel_access_already_granted", "kernel_access_cover_gone", "kernel_access_cover_identity_changed", "kernel_access_cover_kernel_process", "kernel_access_cover_outside", "kernel_access_cover_limit", "kernel_access_agent_unavailable", "kernel_access_local_only", "kernel_access_request_denied", "command_id_required", "duplicate_command_conflict"]
const KERNEL_UNAVAILABLE = ["kernel_unknown", "kernel_ambiguous", "kernel_offline", "kernel_unreachable", "kernel_unpaired", "kernel_incompatible", "kernel_update_needed", "kernel_not_available"]
const TIMEOUT_OR_UNKNOWN = ["kernel_sudo_expired", "kernel_access_request_expired", "kernel_request_timeout", "kernel_connection_lost", "kernel_outcome_unknown", "kernel_access_unconfirmed", "steer_unconfirmed"]

export const EXIT_CODES: readonly { exit: number; meaning: string; codes: readonly string[] }[] = [
  { exit: 0, meaning: "ok", codes: [] },
  { exit: 1, meaning: "request failed: the kernel answered with its own error code (session_not_found, file_not_found, ledger_unavailable, ...); read code and message", codes: [] },
  { exit: 2, meaning: "usage error", codes: ["usage", "file_path_invalid"] },
  { exit: 3, meaning: "refused: the user, the grant or the file permissions do not allow it", codes: [...REFUSED, "file_permission_denied", "subscription_ended"] },
  { exit: 4, meaning: "kernel unavailable", codes: KERNEL_UNAVAILABLE },
  { exit: 5, meaning: "timeout or unknown outcome", codes: TIMEOUT_OR_UNKNOWN },
  { exit: 6, meaning: "approval pending (access request --no-wait, secret request --no-wait)", codes: [] },
  { exit: 7, meaning: "event cursor expired: resync followed targets", codes: ["cursor_expired"] },
]

export function exitCodeFor(code: string): number {
  return EXIT_CODES.find(entry => entry.codes.includes(code))?.exit ?? 1
}

export class CommandFailure extends Error {
  constructor(readonly code: string, message: string, readonly details?: Record<string, unknown>) {
    super(message)
    this.name = "CommandFailure"
  }
}

/** `--help` before any `--` (what follows is a program's argv). */
export function wantsHelp(argv: readonly string[]): boolean {
  const end = argv.indexOf("--")
  return (end < 0 ? argv : argv.slice(0, end)).includes("--help")
}

export function familyUsageOf(family: string, verbs: readonly string[]): string {
  return `Usage: chariox ${family} ${verbs.join("|")} (chariox guide lists the arguments)`
}

export function usageOf(command: string): string {
  return usage(commandSpec(command))
}

function commandSpec(command: string): CommandSpec {
  const spec = COMMANDS.find(entry => entry.name === command)
  if (!spec) throw new Error(`unknown command ${command}`)
  return spec
}

function usage(spec: CommandSpec): string {
  const flags = spec.flags.map(flag => {
    const text = `${flag.name}${flag.value ? ` ${flag.value}` : ""}`
    return flag.required ? (flag.repeat ? `${text} [${text} ...]` : text) : `[${text}]`
  })
  return `Usage: chariox ${[spec.name, ...flags, ...spec.args ?? []].join(" ")}`
}

export function parseFlags(command: string, argv: readonly string[]): Map<string, string | true> {
  return parseArguments(command, argv).flags
}

export function parseArguments(command: string, argv: readonly string[]): { flags: Map<string, string | true>; repeated: Map<string, string[]>; positionals: string[] } {
  const spec = commandSpec(command)
  const values = new Map<string, string | true>()
  const repeated = new Map<string, string[]>()
  const positionals: string[] = []
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i]!.startsWith("--")) { positionals.push(argv[i]!); continue }
    const flag = spec.flags.find(entry => entry.name === argv[i])
    const value = flag?.value ? argv[++i] : true
    if (!flag || !value) throw new CommandFailure("usage", usage(spec))
    if (flag.repeat) { repeated.set(flag.name, [...repeated.get(flag.name) ?? [], String(value)]); continue }
    if (values.has(flag.name)) throw new CommandFailure("usage", usage(spec))
    values.set(flag.name, value)
  }
  if (positionals.length !== (spec.args?.length ?? 0)) throw new CommandFailure("usage", usage(spec))
  return { flags: values, repeated, positionals }
}

export function guideText(): string {
  const commands = COMMANDS.map(spec => `  ${usage(spec).replace("Usage: ", "")}\n      ${spec.summary}${spec.note ? `\n      ${spec.note}` : ""}`)
  const exits = EXIT_CODES.map(entry => `  ${entry.exit} ${entry.meaning}`)
  return [
    "Chariox guide for agents. You talk only to your local kernel; nothing is installed in your harness.",
    "1. Ask once: chariox access request --until-exit",
    "   It waits while the user decides; exit 0 and a printed grant mean approved: go on to step 2.",
    "   If a command prints a pointer or says the user must approve, show it to the user verbatim and wait.",
    "   If your tool calls time out, add --no-wait (exit 6), then poll chariox access status until granted (lasts until your harness exits).",
    "2. List kernels: chariox kernels (a name is the machine name, or <machine>/<label> when the machine runs several of your kernels)",
    "3. Send a request (one JSON object on stdin): chariox request --kernel NAME < request.json (always pass --kernel for another kernel; without it the request runs on this one)",
    "4. Watch events on a kernel: chariox watch --kernel NAME --session ID --attachment ID (after kernel_connection_lost run it again with --from N, the N in the message)",
    "5. Move files: chariox files put|get|stat|ls --kernel NAME ... (put and get resume after an interruption: run the same command again)",
    "6. Run long commands: chariox terminal open|send|read|wait|close --kernel NAME --session ID ...",
    "   Terminals keep running if you disconnect; read them later with --since. For one-off commands, run chariox exec --kernel NAME -- argv...",
    "",
    "Follow sessions or single agents with chariox events subscribe. Use chariox events wait --timeout 540 in the foreground or your harness background option, or poll chariox events list. Keep next_cursor. A new grant follows nothing, and a wait on it is refused at once. To continue an earlier grant's follows, run chariox events subscribe --like OLD_GRANT_ID.",
    "On exit 7, reread the followed targets and resume from oldest_cursor - 1; ack after handling records.",
    "To be woken instead of polling when a workflow finishes or an agent needs the person: chariox events sources, then chariox events subscribe --source ID; each occurrence is one record of type notification (value.source_id, display_name, occurrence_id, output.message, deadline_ms); a gap record means unread ones were dropped.",
    "A record with value.question is a decision the person may answer through you: ask the person with your own question tool (options are question.choices, a free reply is question.text_reply when present), then send RespondToInteraction with the chosen id and refs.interaction_id (omit --kernel for the kernel you are on).",
    "interaction_closed means someone else answered or it expired: drop the question. Do not choose for the person.",
    "A record with no question but a pointer is the person's alone: show pointer.one_line verbatim and do not answer it.",
    "Or add chariox events mcp as an MCP server: its tool chariox_next_choice waits, asks and answers. A subscription lasts until its grant ends or you cancel it.",
    "Commands:",
    ...commands,
    "Requests for chariox request (replace the UPPERCASE values; ids come from earlier answers):",
    ...REQUEST_EXAMPLES.map(({ body, note }) => `  ${JSON.stringify(body)}  | ${note}`),
    "If the kernel cannot parse a request, read its message naming the expected variants and fields.",
    "Give every request that changes state (CreateSession, AttachToSession, SpawnAgent, DestroyAgent, SubmitPrompt, RunShellCommand, EndSession, RequestKernelSudo) its own --command-id ID; reuse one only to retry.",
    "An AttachToSession you make on another kernel is kept alive for you while your access lasts; send DetachFromSession when you are done with it.",
    "Exec streams raw binary stdout and stderr and returns the remote command's exit code (128 + signal). Requests print one JSON document on stdout. Errors are JSON on stderr: {\"error\":{\"code\",\"message\",\"kernel\"}}.",
    "Exit codes:",
    ...exits,
    "Sudo outcomes: kernel_sudo_refused means the person refused; kernel_sudo_expired means the approval deadline passed; kernel_sudo_room_owner_local means the Room predates enrollment (re-create the Room after enrollment). None starts elevation or retries automatically. Submit a new request only when needed and approve it in a Chariox client.",
    "Retry rule: on kernel_request_timeout, kernel_connection_lost or steer_unconfirmed retry ONCE with the same --command-id. On kernel_outcome_unknown read the state first; never retry blindly.",
    "Human-only, never attempted by you: passkey approvals, grants, sudo approvals, secrets and credentials, relay and pairing configuration. You answer the person's other cards for them with interaction list and interaction answer.",
  ].join("\n")
}
