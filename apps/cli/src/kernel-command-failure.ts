import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure } from "./agent-command-table.js"

const LOCAL_CONNECTION_LOSS = ["connection_closed", "outcome_unknown", "write_failed"]
const CONNECT_OPERATION = "connect kernel websocket"

export function lostInFlight(error: unknown): boolean {
  return error instanceof LocalIpcError && error.operation !== CONNECT_OPERATION && error.code !== null && LOCAL_CONNECTION_LOSS.includes(error.code)
}

export function failureOf(error: unknown): CommandFailure {
  if (error instanceof CommandFailure) return error
  if (error instanceof LocalIpcError) {
    const message = error.message.replace(/^kernel transport `[^`]*` failed: /, "")
    if (error.operation === CONNECT_OPERATION) return new CommandFailure("kernel_unreachable", `${message}; start the kernel or pass --socket PATH`)
    if (lostInFlight(error)) return new CommandFailure("kernel_connection_lost", message)
    return new CommandFailure(error.code ?? "cli_error", message)
  }
  return new CommandFailure("cli_error", error instanceof Error ? error.message : String(error))
}

export function lostConnectionFailure(probeError: unknown): CommandFailure {
  if (probeError instanceof LocalIpcError && probeError.code === "kernel_access_ended") {
    return new CommandFailure("kernel_access_ended", "the access grant ended while this request was in flight; the request may or may not have run")
  }
  return new CommandFailure("kernel_connection_lost", "the connection to the local kernel was lost before the answer; the request may or may not have run")
}

