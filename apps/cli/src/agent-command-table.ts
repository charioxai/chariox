type Flag = { name: string; value?: string }
type CommandSpec = { name: string; flags: readonly Flag[]; summary: string }

const SOCKET: Flag = { name: "--socket", value: "PATH" }
const HOLDER: Flag = { name: "--holder-pid", value: "PID" }

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
    flags: [{ name: "--machine", value: "NAME" }, { name: "--command-id", value: "ID" }, { name: "--timeout", value: "SECONDS" }, SOCKET],
    summary: "send the one JSON request on stdin; prints the JSON response",
  },
]

const REFUSED = ["kernel_access_human_only", "kernel_access_ended", "kernel_access_not_covered", "kernel_access_already_granted", "command_id_required", "duplicate_command_conflict"]
const MACHINE_UNAVAILABLE = ["machine_unknown", "machine_ambiguous", "machine_offline", "machine_unreachable", "machine_unpaired", "machine_incompatible", "machine_not_available"]
const OUTCOME_UNCERTAIN = ["machine_request_timeout", "machine_connection_lost", "machine_outcome_unknown"]

export const EXIT_CODES: readonly { exit: number; meaning: string; codes: readonly string[] }[] = [
  { exit: 0, meaning: "ok", codes: [] },
  { exit: 2, meaning: "usage error", codes: ["usage"] },
  { exit: 3, meaning: "refused: the user or the grant does not allow it", codes: REFUSED },
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
  return `Usage: chariox ${[spec.name, ...flags].join(" ")}`
}

export function parseFlags(command: string, argv: readonly string[]): Map<string, string | true> {
  const spec = commandSpec(command)
  const values = new Map<string, string | true>()
  for (let i = 0; i < argv.length; i++) {
    const flag = spec.flags.find(entry => entry.name === argv[i])
    const value = flag?.value ? argv[++i] : true
    if (!flag || !value || values.has(flag.name)) throw new CommandFailure("usage", usage(spec))
    values.set(flag.name, value)
  }
  return values
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
    "",
    "Commands:",
    ...commands,
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
