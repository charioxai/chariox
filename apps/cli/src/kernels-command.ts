import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure, exitCodeFor, guideText, parseArguments, usageOf } from "./agent-command-table.js"
import { kernelUnixClient } from "./kernel-unix-client.js"
import { FILES_VERBS, runFilesVerb, type SendRequest } from "./kernel-files.js"
import { runTerminalVerb, TERMINAL_VERBS, waitMsOf, type TerminalFlags, type TerminalVerb } from "./terminal-verbs.js"

import { runExec } from "./exec-command.js"

import { EVENT_VERBS, eventTimeoutSeconds, runEventsVerb } from "./events-command.js"

import { failureOf, lostInFlight, lostConnectionFailure } from "./kernel-command-failure.js"
export { failureOf, lostConnectionFailure } from "./kernel-command-failure.js"

const DEFAULT_TIMEOUT_SECONDS = 30
const CONNECT_OPERATION = "connect kernel websocket"

export function reportFailure(error: unknown, kernel: string | null = null): void {
  const failure = failureOf(error)
  process.stderr.write(`${JSON.stringify({ error: { code: failure.code, message: failure.message, kernel, ...failure.details } })}\n`)
  process.exitCode = exitCodeFor(failure.code)
}

export async function probeKernel(socket: string | undefined): Promise<unknown> {
  const client = kernelUnixClient(socket)
  try { return await client.send({ ListGatewayKernels: {} }) } finally { await client.close() }
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
    timer = setTimeout(() => reject(new CommandFailure("kernel_request_timeout", `no answer within ${seconds} seconds; the request may or may not have run. For a state-changing request retry once with the same --command-id`)), seconds * 1000)
  })
  try { return await Promise.race([send, timeout]) } finally { clearTimeout(timer) }
}

// An older kernel ignores `kernel` and would run the request on this computer.
async function requireKernelGateway(socket: string | undefined, timeoutSeconds: number): Promise<void> {
  const unsupported = new CommandFailure("kernel_not_available", "This computer's Chariox kernel cannot forward to other kernels; update it.")
  const client = kernelUnixClient(socket)
  try {
    const response = await sendWithin(client.send<{ RelayStatus?: { status?: { capabilities?: unknown } } }>({ RelayStatus: null }), timeoutSeconds)
    const capabilities = response.RelayStatus?.status?.capabilities
    if (!Array.isArray(capabilities) || !capabilities.includes("kernel_gateway_v1")) throw unsupported
  } catch (error) {
    // A grant refusal is the answer itself; only an older kernel's failure means "update it".
    const refusal = error instanceof LocalIpcError && error.code?.startsWith("kernel_access_")
    throw error instanceof LocalIpcError && error.operation !== CONNECT_OPERATION && !refusal ? unsupported : error
  } finally { await client.close() }
}

// Every request of `use` shares one connection and its own timeout; a transfer is many requests.
export async function withKernel<T>(
  options: { socket: string | undefined; kernel?: string | undefined; commandId?: string | undefined; timeoutSeconds: number },
  use: (send: SendRequest) => Promise<T>,
): Promise<T> {
  if (options.kernel !== undefined) await requireKernelGateway(options.socket, options.timeoutSeconds)
  const client = kernelUnixClient(options.socket, { kernel: options.kernel, commandId: options.commandId })
  try {
    return await use(body => sendWithin(client.send<unknown>(body), options.timeoutSeconds).catch(async error => {
      // Normalize transport loss before a streaming command attaches its resume
      // cursors; an outer catch would lose those details or the IPC error type.
      if (!lostInFlight(error)) throw error
      throw lostConnectionFailure(await probeKernel(options.socket).then(() => null, probeError => probeError))
    }))
  } finally { await client.close() }
}

export function sendRequest(
  body: Record<string, unknown>,
  options: { socket: string | undefined; kernel?: string | undefined; commandId?: string | undefined; timeoutSeconds: number },
): Promise<unknown> {
  if ("AttachToSession" in body && !options.commandId?.trim()) {
    return Promise.reject(new CommandFailure("command_id_required", "AttachToSession changes Room state; supply --command-id ID and reuse it only to retry this attachment. Use the returned SessionAttached.attachment.id."))
  }
  return withKernel(options, send => send(body))
}

// Ends only by failing: the stream has no end of its own, and a lost connection says where to resume.
export async function watchKernel(
  options: { socket: string | undefined; kernel: string; sessionId: string; attachmentId: string; from: number | null },
  print: (line: string) => void,
): Promise<never> {
  await requireKernelGateway(options.socket, DEFAULT_TIMEOUT_SECONDS)
  let lastEventId = options.from
  let lose!: (failure: unknown) => void
  const lost = new Promise<never>((_, reject) => { lose = reject })
  const client = kernelUnixClient(options.socket, { kernel: options.kernel, onSubscriptionEnded: lose })
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
          lose(new CommandFailure("kernel_connection_lost", `the connection to the local kernel was lost; events may have been missed. Run chariox watch again${lastEventId === null ? "" : ` with --from ${lastEventId}`}.`))
        }
      })
    }
  })
  try {
    await Promise.race([client.subscribeToKernelEvents(options.sessionId, options.attachmentId, { resumeFromEventId: options.from }), lost])
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

export async function runKernelsCommand(argv: string[]): Promise<boolean> {
  const [command, ...rest] = argv
  if (command === undefined) return false
  // `chariox terminal --session ...` is the interactive attach for a person; a verb first is the agent's.
  const terminalVerb = command === "terminal" ? TERMINAL_VERBS.find(name => name === rest[0]) : undefined
  if (command !== "guide" && command !== "kernels" && command !== "request" && command !== "watch" && command !== "files" && command !== "events" && command !== "exec" && !terminalVerb) return false
  let kernel: string | null = null
  try {
    if (command === "events") {
      const verb = EVENT_VERBS.find(name => name === rest[0])
      if (!verb) throw new CommandFailure("usage", `Usage: chariox events ${EVENT_VERBS.join("|")} (chariox guide lists the arguments)`)
      const { flags, positionals } = parseArguments(`events ${verb}`, rest.slice(1))
      const socket = flags.get("--socket") as string | undefined
      if (verb === "watch") for (const signal of ["SIGINT", "SIGTERM"] as const) process.once(signal, () => process.exit(0))
      process.exitCode = await withKernel({ socket, timeoutSeconds: eventTimeoutSeconds(verb, flags) + DEFAULT_TIMEOUT_SECONDS }, send => runEventsVerb(verb, flags, positionals, send, value => console.log(JSON.stringify(value))))
      return true
    }
    // `files` is a command family: the verb is part of the table name.
    const verb = command === "files" ? FILES_VERBS.find(name => name === rest[0]) : undefined
    if (command === "files" && !verb) throw new CommandFailure("usage", `Usage: chariox files ${FILES_VERBS.join("|")} (chariox guide lists the arguments)`)
    // What follows `--` of `terminal open` is the program to run, not options.
    const split = terminalVerb === "open" || command === "exec" ? rest.indexOf("--") : -1
    const program = split < 0 ? [] : rest.slice(split + 1)
    const options = split < 0 ? rest : rest.slice(0, split)
    const { flags, positionals } = parseArguments(verb ? `files ${verb}` : terminalVerb ? `terminal ${terminalVerb}` : command, verb ? options.slice(1) : terminalVerb ? options.slice(1) : options)
    const socket = flags.get("--socket") as string | undefined
    if (command === "exec") {
      if (positionals.length || program.length === 0 || Boolean(flags.get("--session")) === Boolean(flags.get("--workspace")) || (flags.has("--host-agent") && !flags.has("--session"))) {
        throw new CommandFailure("usage", "Usage: chariox exec [--kernel NAME] (--session ROOM | --workspace PATH) [--host-agent AGENT] -- argv...")
      }
      kernel = (flags.get("--kernel") as string | undefined) ?? null
      process.exitCode = await withKernel({ socket, timeoutSeconds: DEFAULT_TIMEOUT_SECONDS, kernel: kernel ?? undefined }, send =>
        runExec({ session: flags.get("--session") as string | undefined, workspace: flags.get("--workspace") as string | undefined,
          hostAgent: flags.get("--host-agent") as string | undefined, command: program }, send,
          { stdin: process.stdin, stdout: process.stdout, stderr: process.stderr, signals: process }))
    } else if (command === "guide") {
      console.log(guideText())
    } else if (command === "kernels") {
      const listed = await sendRequest({ ListGatewayKernels: {} }, { socket, timeoutSeconds: DEFAULT_TIMEOUT_SECONDS }) as { GatewayKernelsListed: unknown }
      console.log(JSON.stringify(listed.GatewayKernelsListed))
    } else if (command === "watch") {
      // Stopping a watch is the normal way to end it, not a failure.
      for (const signal of ["SIGINT", "SIGTERM"] as const) process.once(signal, () => process.exit(0))
      kernel = (flags.get("--kernel") as string | undefined) ?? null
      const sessionId = flags.get("--session") as string | undefined
      const attachmentId = flags.get("--attachment") as string | undefined
      if (kernel === null || sessionId === undefined || attachmentId === undefined) throw new CommandFailure("usage", usageOf("watch"))
      await watchKernel({ socket, kernel, sessionId, attachmentId, from: parseEventId(flags.get("--from")) }, line => console.log(line))
    } else if (terminalVerb) {
      kernel = (flags.get("--kernel") as string | undefined) ?? null
      const terminalFlags: TerminalFlags = {
        cols: flags.get("--cols"), rows: flags.get("--rows"), title: flags.get("--title"), session: flags.get("--session"),
        terminal: flags.get("--terminal"), since: flags.get("--since"), waitMs: flags.get("--wait-ms"), for: flags.get("--for"),
        timeout: flags.get("--timeout"), hostAgent: flags.get("--host-agent"), noEnter: flags.has("--no-enter"),
      }
      // A wait, a long read or a send that waits for its output is answered when the wait ends:
      // the request may take that long, plus the usual margin.
      const waitSeconds = Math.ceil(waitMsOf(terminalVerb, terminalFlags) / 1000)
      const timeoutSeconds = terminalVerb === "wait" || terminalVerb === "read" || waitSeconds > 0
        ? waitSeconds + DEFAULT_TIMEOUT_SECONDS
        : parseTimeout(flags.get("--timeout"))
      const result = await withKernel({ socket, timeoutSeconds: Math.min(timeoutSeconds, 240 + DEFAULT_TIMEOUT_SECONDS), kernel: kernel ?? undefined }, send =>
        runTerminalVerb(terminalVerb as TerminalVerb, terminalVerb === "open" ? program : positionals, terminalFlags, send))
      console.log(JSON.stringify(result))
    } else if (command === "files" && verb) {
      kernel = (flags.get("--kernel") as string | undefined) ?? null
      const timeoutSeconds = parseTimeout(flags.get("--timeout"))
      const result = await withKernel({ socket, timeoutSeconds, kernel: kernel ?? undefined }, send =>
        runFilesVerb(verb, positionals, { sha256: flags.has("--sha256"), overwrite: flags.has("--overwrite") }, send))
      console.log(JSON.stringify(result))
    } else {
      kernel = (flags.get("--kernel") as string | undefined) ?? null
      const body = parseRequestBody(await readStdin())
      const timeoutSeconds = parseTimeout(flags.get("--timeout"))
      const commandId = flags.get("--command-id") as string | undefined
      if (flags.has("--no-subscribe")) {
        const variant = ["CreateSession", "SpawnAgent", "SpawnAgents"].find(name => name in body)
        if (!variant || !body[variant] || typeof body[variant] !== "object") throw new CommandFailure("usage", "--no-subscribe applies to CreateSession, SpawnAgent or SpawnAgents")
        ;(body[variant] as Record<string, unknown>).no_subscribe = true
      }
      const response = await sendRequest(body, { socket, timeoutSeconds, kernel: kernel ?? undefined, commandId })
      console.log(JSON.stringify(response))
    }
  } catch (error) {
    reportFailure(error, kernel)
  }
  return true
}
