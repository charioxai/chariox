import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure, exitCodeFor, guideText, parseArguments, usageOf } from "./agent-command-table.js"
import { kernelUnixClient } from "./kernel-unix-client.js"
import { FILES_VERBS, runFilesVerb, type SendRequest } from "./machine-files.js"
import { runTerminalVerb, TERMINAL_VERBS, waitMsOf, type TerminalFlags, type TerminalVerb } from "./terminal-verbs.js"

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
    timer = setTimeout(() => reject(new CommandFailure("machine_request_timeout", `no answer within ${seconds} seconds; the request may or may not have run. For a state-changing request retry once with the same --command-id`)), seconds * 1000)
  })
  try { return await Promise.race([send, timeout]) } finally { clearTimeout(timer) }
}

// An older kernel ignores `machine` and would run the request on this computer.
async function requireMachineGateway(socket: string | undefined, timeoutSeconds: number): Promise<void> {
  const unsupported = new CommandFailure("machine_not_available", "This computer's Chariox kernel cannot forward to other machines; update it.")
  const client = kernelUnixClient(socket)
  try {
    const response = await sendWithin(client.send<{ RelayStatus?: { status?: { capabilities?: unknown } } }>({ RelayStatus: null }), timeoutSeconds)
    const capabilities = response.RelayStatus?.status?.capabilities
    if (!Array.isArray(capabilities) || !capabilities.includes("machine_gateway_v1")) throw unsupported
  } catch (error) {
    // A grant refusal is the answer itself; only an older kernel's failure means "update it".
    const refusal = error instanceof LocalIpcError && error.code?.startsWith("kernel_access_")
    throw error instanceof LocalIpcError && error.operation !== CONNECT_OPERATION && !refusal ? unsupported : error
  } finally { await client.close() }
}

// Every request of `use` shares one connection and its own timeout; a transfer is many requests.
export async function withKernel<T>(
  options: { socket: string | undefined; machine?: string | undefined; commandId?: string | undefined; timeoutSeconds: number },
  use: (send: SendRequest) => Promise<T>,
): Promise<T> {
  if (options.machine !== undefined) await requireMachineGateway(options.socket, options.timeoutSeconds)
  const client = kernelUnixClient(options.socket, { machine: options.machine, commandId: options.commandId })
  try {
    return await use(body => sendWithin(client.send<unknown>(body), options.timeoutSeconds))
  } catch (error) {
    if (!lostInFlight(error)) throw error
    throw lostConnectionFailure(await probeKernel(options.socket).then(() => null, probeError => probeError))
  } finally { await client.close() }
}

export function sendRequest(
  body: Record<string, unknown>,
  options: { socket: string | undefined; machine?: string | undefined; commandId?: string | undefined; timeoutSeconds: number },
): Promise<unknown> {
  return withKernel(options, send => send(body))
}

// Ends only by failing: the stream has no end of its own, and a lost connection says where to resume.
export async function watchMachine(
  options: { socket: string | undefined; machine: string; sessionId: string; attachmentId: string; from: number | null },
  print: (line: string) => void,
): Promise<never> {
  await requireMachineGateway(options.socket, DEFAULT_TIMEOUT_SECONDS)
  let lastEventId = options.from
  let lose!: (failure: unknown) => void
  const lost = new Promise<never>((_, reject) => { lose = reject })
  const client = kernelUnixClient(options.socket, { machine: options.machine, onSubscriptionEnded: lose })
  client.onKernelEvent((event, frame) => {
    if (frame) {
      lastEventId = frame.event_id
      print(JSON.stringify(frame))
    } else if (event.event === "transport_closed") {
      // The client would reconnect on its own; a watcher that missed events must say so instead.
      // A revoked or expired grant closes the socket too: a fresh connection tells which.
      void probeKernel(options.socket).then(() => null, probeError => probeError).then(probeError => {
        if (probeError instanceof LocalIpcError && probeError.code === "kernel_access_ended") {
          lose(new CommandFailure("kernel_access_ended", "the access grant ended; this watch stopped"))
        } else {
          lose(new CommandFailure("machine_connection_lost", `the connection to the local kernel was lost; events may have been missed. Run chariox watch again${lastEventId === null ? "" : ` with --from ${lastEventId}`}.`))
        }
      })
    }
  })
  try {
    await Promise.race([client.subscribeToKernelEvents(options.sessionId, options.attachmentId, options.from), lost])
    return await lost
  } finally { await client.close() }
}

function parseEventId(value: string | true | undefined): number | null {
  if (value === undefined) return null
  const id = Number(value)
  if (!Number.isSafeInteger(id) || id < 0) throw new CommandFailure("usage", "--from needs an event id: a whole number from 0")
  return id
}

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = []
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer)
  return Buffer.concat(chunks).toString("utf8")
}

export async function runMachineCommand(argv: string[]): Promise<boolean> {
  const [command, ...rest] = argv
  // `chariox terminal --session ...` is the interactive attach for a person; a verb first is the agent's.
  const terminalVerb = command === "terminal" ? TERMINAL_VERBS.find(name => name === rest[0]) : undefined
  if (command !== "guide" && command !== "machines" && command !== "request" && command !== "watch" && command !== "files" && !terminalVerb) return false
  let machine: string | null = null
  try {
    // `files` is a command family: the verb is part of the table name.
    const verb = command === "files" ? FILES_VERBS.find(name => name === rest[0]) : undefined
    if (command === "files" && !verb) throw new CommandFailure("usage", `Usage: chariox files ${FILES_VERBS.join("|")} (chariox guide lists the arguments)`)
    // What follows `--` of `terminal open` is the program to run, not options.
    const split = terminalVerb === "open" ? rest.indexOf("--") : -1
    const program = split < 0 ? [] : rest.slice(split + 1)
    const options = split < 0 ? rest : rest.slice(0, split)
    const { flags, positionals } = parseArguments(verb ? `files ${verb}` : terminalVerb ? `terminal ${terminalVerb}` : command, verb ? options.slice(1) : terminalVerb ? options.slice(1) : options)
    const socket = flags.get("--socket") as string | undefined
    if (command === "guide") {
      console.log(guideText())
    } else if (command === "machines") {
      const listed = await sendRequest({ ListGatewayMachines: {} }, { socket, timeoutSeconds: DEFAULT_TIMEOUT_SECONDS }) as { GatewayMachinesListed: unknown }
      console.log(JSON.stringify(listed.GatewayMachinesListed))
    } else if (command === "watch") {
      // Stopping a watch is the normal way to end it, not a failure.
      for (const signal of ["SIGINT", "SIGTERM"] as const) process.once(signal, () => process.exit(0))
      machine = (flags.get("--machine") as string | undefined) ?? null
      const sessionId = flags.get("--session") as string | undefined
      const attachmentId = flags.get("--attachment") as string | undefined
      if (machine === null || sessionId === undefined || attachmentId === undefined) throw new CommandFailure("usage", usageOf("watch"))
      await watchMachine({ socket, machine, sessionId, attachmentId, from: parseEventId(flags.get("--from")) }, line => console.log(line))
    } else if (terminalVerb) {
      machine = (flags.get("--machine") as string | undefined) ?? null
      const terminalFlags: TerminalFlags = {
        cols: flags.get("--cols"), rows: flags.get("--rows"), title: flags.get("--title"), session: flags.get("--session"),
        terminal: flags.get("--terminal"), since: flags.get("--since"), waitMs: flags.get("--wait-ms"), for: flags.get("--for"),
        timeout: flags.get("--timeout"), noEnter: flags.has("--no-enter"),
      }
      // A wait or long read is answered when it ends: the request may take that long, plus the usual margin.
      const timeoutSeconds = terminalVerb === "wait" || terminalVerb === "read"
        ? Math.ceil(waitMsOf(terminalVerb, terminalFlags) / 1000) + DEFAULT_TIMEOUT_SECONDS
        : parseTimeout(flags.get("--timeout"))
      const result = await withKernel({ socket, timeoutSeconds: Math.min(timeoutSeconds, 240 + DEFAULT_TIMEOUT_SECONDS), machine: machine ?? undefined }, send =>
        runTerminalVerb(terminalVerb as TerminalVerb, terminalVerb === "open" ? program : positionals, terminalFlags, send))
      console.log(JSON.stringify(result))
    } else if (command === "files" && verb) {
      machine = (flags.get("--machine") as string | undefined) ?? null
      const timeoutSeconds = parseTimeout(flags.get("--timeout"))
      const result = await withKernel({ socket, timeoutSeconds, machine: machine ?? undefined }, send =>
        runFilesVerb(verb, positionals, { sha256: flags.has("--sha256"), overwrite: flags.has("--overwrite") }, send))
      console.log(JSON.stringify(result))
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
