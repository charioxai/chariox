import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure } from "./agent-command-table.js"

const LOCAL_CONNECTION_LOSS = ["connection_closed", "outcome_unknown", "write_failed"]
const CONNECT_OPERATION = "connect kernel websocket"

export function lostInFlight(error: unknown): boolean {
  return error instanceof LocalIpcError && error.operation !== CONNECT_OPERATION && error.code !== null && LOCAL_CONNECTION_LOSS.includes(error.code)
}

// The kernel cannot read the identity of a process inside an agent's sandbox, and says "retry". Retrying does not
// help there: the socket route needs a grant, and an agent's own shell has the agent credential instead.
const AGENT_SHELL_UNCONFIRMED = "This command runs in an agent's shell without that agent's credential, so the kernel cannot tell who is asking, and trying again will not change that. Run it from the agent's own shell (the one that has CHARIOX_AGENT_CLI_TOKEN), or ask the owner to grant access with `chariox access request`."

/** The kernel named its endpoint for this process, or confined it: it runs in an agent's shell. */
function inAgentShell(): boolean {
  return Boolean(process.env.CHARIOX_KERNEL_URL?.trim() || process.env.CHARIOX_KERNEL_STATE_CONFINED?.trim())
}

export function failureOf(error: unknown): CommandFailure {
  if (error instanceof CommandFailure) return error
  if (error instanceof LocalIpcError) {
    const message = error.message.replace(/^kernel transport `[^`]*` failed: /, "")
    if (error.operation === CONNECT_OPERATION) return new CommandFailure("kernel_unreachable", `${message}; start the kernel or pass --socket PATH`)
    if (lostInFlight(error)) return new CommandFailure("kernel_connection_lost", message)
    if (error.code === "kernel_access_unconfirmed" && inAgentShell()) return new CommandFailure(error.code, AGENT_SHELL_UNCONFIRMED)
    return new CommandFailure(error.code ?? "cli_error", message)
  }
  return new CommandFailure("cli_error", error instanceof Error ? error.message : String(error))
}

export function lostConnectionFailure(probeError: unknown): CommandFailure {
  if (probeError instanceof LocalIpcError && probeError.code === "kernel_access_ended") {
    return new CommandFailure("kernel_access_ended", "Access ended while the request was in flight. Check whether it ran, then run `chariox access request --no-wait` before retrying.")
  }
  return new CommandFailure("kernel_connection_lost", "the connection to the local kernel was lost before the answer; the request may or may not have run")
}

