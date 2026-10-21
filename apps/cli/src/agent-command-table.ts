import {
  attachToSessionRequest, cancelActivePromptRequest, createSessionRequest, endSessionRequest, getSessionStateRequest,
  listProjectsRequest, listSessionsRequest, searchWorkspaceDirectoriesRequest, spawnAgentRequest, submitPromptRequest,
} from "./ipc-requests.js"

type Flag = { name: string; value?: string }
type CommandSpec = { name: string; flags: readonly Flag[]; args?: readonly string[]; summary: string }

const SOCKET: Flag = { name: "--socket", value: "PATH" }
const HOLDER: Flag = { name: "--holder-pid", value: "PID" }
const MACHINE: Flag = { name: "--machine", value: "NAME" }
const TIMEOUT: Flag = { name: "--timeout", value: "SECONDS" }
const OVERWRITE: Flag = { name: "--overwrite" }
const SESSION: Flag = { name: "--session", value: "SESSION_ID" }
const TERMINAL: Flag = { name: "--terminal", value: "TERMINAL_ID" }

export const COMMANDS: readonly CommandSpec[] = [
  { name: "guide", flags: [], summary: "print this guide" },
  {
    name: "access request",
    flags: [{ name: "--until-exit" }, { name: "--minutes", value: "N" }, { name: "--no-wait" }, HOLDER, SOCKET],
    summary: "ask the user to approve access for you; prints the grant",
  },
  { name: "access status", flags: [HOLDER, SOCKET], summary: "granted, pending, ended or not_requested" },
  { name: "machines", flags: [SOCKET], summary: "list the machines this kernel can reach" },
  {
    name: "request",
    flags: [MACHINE, { name: "--command-id", value: "ID" }, TIMEOUT, SOCKET],
    summary: "send the one JSON request on stdin; prints the JSON response",
  },
  {
    name: "watch",
    flags: [MACHINE, { name: "--session", value: "SESSION_ID" }, { name: "--attachment", value: "ATTACHMENT_ID" }, { name: "--from", value: "EVENT_ID" }, SOCKET],
    summary: "print one JSON line per event of that machine's session until the connection is lost",
  },
  {
    name: "terminal open",
    flags: [MACHINE, SESSION, { name: "--cols", value: "N" }, { name: "--rows", value: "N" }, { name: "--title", value: "TITLE" }, TIMEOUT, SOCKET],
    summary: "open a terminal in that Room (a login shell, or the argv after --); it keeps running when you disconnect",
  },
  { name: "terminal list", flags: [MACHINE, SESSION, TIMEOUT, SOCKET], summary: "the Room's terminals with their output offsets" },
  {
    name: "terminal send",
    flags: [MACHINE, SESSION, TERMINAL, { name: "--no-enter" }, TIMEOUT, SOCKET],
    args: ["TEXT"],
    summary: "type TEXT and Enter; prints after_offset, the offset to read from",
  },
  {
    name: "terminal read",
    flags: [MACHINE, SESSION, TERMINAL, { name: "--since", value: "OFFSET" }, { name: "--wait-ms", value: "MS" }, SOCKET],
    summary: "the output after OFFSET as text and next_offset; gap true means it scrolled out: use list for a fresh offset",
  },
  {
    name: "terminal wait",
    flags: [MACHINE, SESSION, TERMINAL, { name: "--for", value: "finished|waiting_for_input|output_contains=TEXT" }, { name: "--since", value: "OFFSET" }, TIMEOUT, SOCKET],
    summary: "block until that happens or --timeout seconds pass (default 60); prints fired (null on timeout) and the screen",
  },
  { name: "terminal close", flags: [MACHINE, SESSION, TERMINAL, TIMEOUT, SOCKET], summary: "end the terminal and everything running in it" },
  { name: "files stat", flags: [MACHINE, { name: "--sha256" }, TIMEOUT, SOCKET], args: ["PATH"], summary: "kind, size, modified time and optionally sha256 of a file; partial_size is an unfinished upload" },
  { name: "files ls", flags: [MACHINE, TIMEOUT, SOCKET], args: ["PATH"], summary: "list a directory" },
  { name: "files get", flags: [MACHINE, OVERWRITE, TIMEOUT, SOCKET], args: ["REMOTE_PATH", "LOCAL_PATH"], summary: "download a file, verified by sha256; prints size, bytes received and resumed_from" },
  { name: "files put", flags: [MACHINE, OVERWRITE, TIMEOUT, SOCKET], args: ["LOCAL_PATH", "REMOTE_PATH"], summary: "upload a file, verified by sha256; prints size, bytes sent and resumed_from" },
]

export const REQUEST_EXAMPLES: readonly { body: object; note: string }[] = [
  { body: listSessionsRequest(), note: "SessionsListed.sessions" },
  { body: listProjectsRequest(), note: "ProjectsListed.projects" },
  { body: searchWorkspaceDirectoriesRequest("NAME", 20), note: "WorkspaceDirectoriesSearched.directories: absolute directory paths on that machine" },
  { body: createSessionRequest("/PATH", "/PATH"), note: "workspace_id and worktree_id are one such directory path; SessionCreated.session.id" },
  { body: attachToSessionRequest("SESSION_ID", "CLIENT_NAME"), note: "SessionAttached.attachment.id; client_id is any name you pick" },
  { body: getSessionStateRequest("SESSION_ID"), note: "SessionState: agents, prompts, output" },
  { body: spawnAgentRequest("SESSION_ID", "codex"), note: "provider is claude, codex or opencode; AgentSpawned.agent.id" },
  { body: submitPromptRequest("SESSION_ID", "ATTACHMENT_ID", null, "PROMPT", []), note: "target_agent_id is an agent id, or null for the default agent" },
  { body: cancelActivePromptRequest("SESSION_ID", "ATTACHMENT_ID", null), note: "stops the running prompt" },
  {
    body: { RunShellCommand: { session_id: "SESSION_ID", attachment_id: "ATTACHMENT_ID", command: "ls", args: ["-la"], working_directory: "/PATH", timeout_ms: 60000 } },
    note: "runs on that machine; ShellCommandCompleted.result has exit_code, stdout, stderr",
  },
  { body: endSessionRequest("SESSION_ID"), note: "SessionEnded" },
]

const REFUSED = ["kernel_access_human_only", "kernel_access_ended", "kernel_access_not_covered", "kernel_access_already_granted", "command_id_required", "duplicate_command_conflict"]
const MACHINE_UNAVAILABLE = ["machine_unknown", "machine_ambiguous", "machine_offline", "machine_unreachable", "machine_unpaired", "machine_incompatible", "machine_not_available"]
const OUTCOME_UNCERTAIN = ["machine_request_timeout", "machine_connection_lost", "machine_outcome_unknown"]

export const EXIT_CODES: readonly { exit: number; meaning: string; codes: readonly string[] }[] = [
  { exit: 0, meaning: "ok", codes: [] },
  { exit: 2, meaning: "usage error", codes: ["usage", "file_path_invalid"] },
  { exit: 3, meaning: "refused: the user, the grant or the file permissions do not allow it", codes: [...REFUSED, "file_permission_denied"] },
  { exit: 4, meaning: "machine unavailable", codes: MACHINE_UNAVAILABLE },
  { exit: 5, meaning: "timeout or unknown outcome: the request may have run", codes: OUTCOME_UNCERTAIN },
  { exit: 6, meaning: "approval pending (access request --no-wait)", codes: [] },
]

export function exitCodeFor(code: string): number {
  return EXIT_CODES.find(entry => entry.codes.includes(code))?.exit ?? 1
}

export class CommandFailure extends Error {
  constructor(readonly code: string, message: string) {
    super(message)
    this.name = "CommandFailure"
  }
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
  const flags = spec.flags.map(flag => `[${flag.name}${flag.value ? ` ${flag.value}` : ""}]`)
  return `Usage: chariox ${[spec.name, ...flags, ...spec.args ?? []].join(" ")}`
}

export function parseFlags(command: string, argv: readonly string[]): Map<string, string | true> {
  return parseArguments(command, argv).flags
}

export function parseArguments(command: string, argv: readonly string[]): { flags: Map<string, string | true>; positionals: string[] } {
  const spec = commandSpec(command)
  const values = new Map<string, string | true>()
  const positionals: string[] = []
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i]!.startsWith("--")) { positionals.push(argv[i]!); continue }
    const flag = spec.flags.find(entry => entry.name === argv[i])
    const value = flag?.value ? argv[++i] : true
    if (!flag || !value || values.has(flag.name)) throw new CommandFailure("usage", usage(spec))
    values.set(flag.name, value)
  }
  if (positionals.length !== (spec.args?.length ?? 0)) throw new CommandFailure("usage", usage(spec))
  return { flags: values, positionals }
}

export function guideText(): string {
  const commands = COMMANDS.map(spec => `  ${usage(spec).replace("Usage: ", "")}\n      ${spec.summary}`)
  const exits = EXIT_CODES.map(entry => `  ${entry.exit} ${entry.meaning}`)
  return [
    "Chariox guide for agents. You talk only to your local kernel; nothing is installed in your harness.",
    "",
    "1. Ask once: chariox access request --until-exit",
    "   It waits while the user decides; exit 0 and a printed grant mean approved: go on to step 2.",
    "   If a command prints a pointer or says the user must approve, show it to the user verbatim and wait.",
    "   If your tool calls time out, add --no-wait (exit 6), then poll chariox access status.",
    "2. List machines: chariox machines",
    "3. Send a request (one JSON object on stdin): chariox request --machine NAME < request.json",
    "   Always pass --machine for another machine; without it the request runs on this one.",
    "4. Watch events on a machine: chariox watch --machine NAME --session ID --attachment ID",
    "   After machine_connection_lost run it again with --from N, the N in the message.",
    "5. Move files: chariox files put|get|stat|ls --machine NAME ...",
    "   put and get resume after an interruption: run the same command again.",
    "6. Run long commands: chariox terminal open|send|read|wait|close --machine NAME --session ID ...",
    "   Terminals belong to the Room and keep running if you disconnect; read them later with --since.",
    "",
    "Commands:",
    ...commands,
    "",
    "Requests for chariox request (replace the UPPERCASE values; ids come from earlier answers):",
    ...REQUEST_EXAMPLES.map(({ body, note }) => `  ${JSON.stringify(body)}  | ${note}`),
    "A request the kernel cannot parse comes back with a message naming the expected variants and fields: read it and fix the request.",
    "Give every request that changes state (CreateSession, SpawnAgent, SubmitPrompt, RunShellCommand, EndSession) its own --command-id ID; reuse one only to retry.",
    "An AttachToSession you make on another machine is kept alive for you while your access lasts; send DetachFromSession when you are done with it.",
    "",
    "Output is one JSON document on stdout. Errors are JSON on stderr: {\"error\":{\"code\",\"message\",\"machine\"}}.",
    "Exit codes:",
    ...exits,
    "",
    "Retry rule: on machine_request_timeout or machine_connection_lost retry ONCE with the same --command-id.",
    "On machine_outcome_unknown read the state first; never retry blindly.",
    "Human-only, never attempted by you: passkey approvals, grants, sudo approvals, secrets and credentials, relay and pairing configuration.",
  ].join("\n")
}
