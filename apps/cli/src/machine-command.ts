import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure, exitCodeFor, guideText, parseFlags } from "./agent-command-table.js"
import { kernelUnixClient } from "./kernel-unix-client.js"

const LOCAL_CONNECTION_LOSS = ["connection_closed", "outcome_unknown", "write_failed"]
const CONNECT_OPERATION = "connect kernel websocket"
const DEFAULT_TIMEOUT_SECONDS = 30

function lostInFlight(error: unknown): boolean {
  return error instanceof LocalIpcError && error.operation !== CONNECT_OPERATION && error.code !== null && LOCAL_CONNECTION_LOSS.includes(error.code)
}

export function failureOf(error: unknown): CommandFailure {
  if (error instanceof CommandFailure) return error
  if (error instanceof LocalIpcError) {
    const message = error.message.replace(/^kernel transport `[^`]*` failed: /, "")
    if (error.operation === CONNECT_OPERATION) return new CommandFailure("kernel_unreachable", `${message}; start the kernel or pass --socket PATH`)
    if (lostInFlight(error)) return new CommandFailure("machine_connection_lost", message)
    return new CommandFailure(error.code ?? "cli_error", message)
  }
  return new CommandFailure("cli_error", error instanceof Error ? error.message : String(error))
}

export function reportFailure(error: unknown, machine: string | null = null): void {
  const failure = failureOf(error)
  process.stderr.write(`${JSON.stringify({ error: { code: failure.code, message: failure.message, machine } })}\n`)
  process.exitCode = exitCodeFor(failure.code)
}

export async function probeKernel(socket: string | undefined): Promise<unknown> {
  const client = kernelUnixClient(socket)
  try { return await client.send({ ListGatewayMachines: {} }) } finally { await client.close() }
}

export function lostConnectionFailure(probeError: unknown): CommandFailure {
  if (probeError instanceof LocalIpcError && probeError.code === "kernel_access_ended") {
    return new CommandFailure("kernel_access_ended", "the access grant ended while this request was in flight; the request may or may not have run")
  }
  return new CommandFailure("machine_connection_lost", "the connection to the local kernel was lost before the answer; the request may or may not have run")
}

export function parseRequestBody(text: string): Record<string, unknown> {
  let body: unknown
  try { body = JSON.parse(text) } catch { throw new CommandFailure("usage", "chariox request reads one JSON request object on stdin") }
  if (!body || typeof body !== "object" || Array.isArray(body) || "__kernel_transport" in body) {
    throw new CommandFailure("usage", "chariox request reads one JSON request object on stdin; subscriptions are not supported")
  }
  return body as Record<string, unknown>
}

function parseTimeout(value: string | true | undefined): number {
  if (value === undefined) return DEFAULT_TIMEOUT_SECONDS
  const seconds = Number(value)
  if (!Number.isFinite(seconds) || seconds <= 0) throw new CommandFailure("usage", "--timeout needs a positive number of seconds")
  return seconds
}

async function sendWithin<T>(send: Promise<T>, seconds: number): Promise<T> {
  let timer: NodeJS.Timeout | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new CommandFailure("machine_request_timeout", `no answer within ${seconds} seconds; the request may or may not have run`)), seconds * 1000)
  })
  try { return await Promise.race([send, timeout]) } finally { clearTimeout(timer) }
}

export async function sendRequest(
  body: Record<string, unknown>,
  options: { socket: string | undefined; machine?: string | undefined; commandId?: string | undefined; timeoutSeconds: number },
): Promise<unknown> {
  const client = kernelUnixClient(options.socket, { machine: options.machine, commandId: options.commandId })
  try {
    return await sendWithin(client.send<unknown>(body), options.timeoutSeconds)
  } catch (error) {
    if (!lostInFlight(error)) throw error
    throw lostConnectionFailure(await probeKernel(options.socket).then(() => null, probeError => probeError))
  } finally { await client.close() }
}

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = []
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer)
  return Buffer.concat(chunks).toString("utf8")
}

export async function runMachineCommand(argv: string[]): Promise<boolean> {
  const [command, ...rest] = argv
  if (command !== "guide" && command !== "machines" && command !== "request") return false
  let machine: string | null = null
  try {
    const flags = parseFlags(command, rest)
    const socket = flags.get("--socket") as string | undefined
    if (command === "guide") {
      console.log(guideText())
    } else if (command === "machines") {
      const listed = await sendRequest({ ListGatewayMachines: {} }, { socket, timeoutSeconds: DEFAULT_TIMEOUT_SECONDS }) as { GatewayMachinesListed: unknown }
      console.log(JSON.stringify(listed.GatewayMachinesListed))
    } else {
      machine = (flags.get("--machine") as string | undefined) ?? null
      const body = parseRequestBody(await readStdin())
      const timeoutSeconds = parseTimeout(flags.get("--timeout"))
      const commandId = flags.get("--command-id") as string | undefined
      const response = await sendRequest(body, { socket, timeoutSeconds, machine: machine ?? undefined, commandId })
      console.log(JSON.stringify(response))
    }
  } catch (error) {
    reportFailure(error, machine)
  }
  return true
}
