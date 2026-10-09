#!/usr/bin/env node
// MP-08 / MP-10 / MP-11: Mac/Linux clients for the live owner acceptance kit.
// No passkey arguments, stdin fields, vault access or raw response dumps.
import { spawn, execFileSync } from "node:child_process"
import { createInterface } from "node:readline/promises"
import { resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { createRequire } from "node:module"

const [mode, ...args] = process.argv.slice(2)
const options = new Map()
for (let i = 0; i < args.length; i += 2) {
  if (!args[i]?.startsWith("--") || !args[i + 1] || options.has(args[i])) throw new Error("MP-11: invalid option")
  options.set(args[i], args[i + 1])
}
const allowed = new Set(["--checkout", "--socket", "--session", "--agent", "--endpoint", "--minutes", "--chariox"])
for (const key of options.keys()) if (!allowed.has(key)) throw new Error("MP-11: unsupported option")
if (!["holder", "probe", "observe", "access-list", "tcp-check"].includes(mode) || !options.has("--checkout")) {
  throw new Error("MP-11: sudo-kit.mjs holder|probe|observe|access-list|tcp-check --checkout <source> --socket <socket> --session <id> [--agent <id> --minutes <n> --chariox <cli>] or observe|access-list|tcp-check --endpoint <loopback ws URL>")
}
const { LocalIpcClient } = await import(pathToFileURL(resolve(options.get("--checkout"), "packages/kernel-client/dist/ipc.js")))
const session = options.get("--session")
const agent = options.get("--agent")
const endpoint = ["observe", "access-list", "tcp-check"].includes(mode)
  ? options.get("--endpoint") : `ws+unix://${options.get("--socket") || ""}`
if (["observe", "access-list", "tcp-check"].includes(mode) && !/^ws:\/\/127\.0\.0\.1:\d+(?:\/kernel)?$/.test(endpoint || "")) {
  throw new Error("MP-11: observer needs the live local owner kernel endpoint")
}
if ((mode === "holder" || mode === "probe") && (!options.get("--socket")?.startsWith("/") || !session)) {
  throw new Error("MP-11: Unix clients need an absolute socket and session ID")
}
const client = new LocalIpcClient(endpoint, { controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 86400000 })
const emit = value => console.log(JSON.stringify({ mp: ["MP-08", "MP-10", "MP-11"], at: new Date().toISOString(), pid: process.pid, ...value }))
const fields = (value, names) => Object.fromEntries(names.filter(key => value?.[key] !== undefined).map(key => [key, value[key]]))
const turnFields = ["entry_id", "session_id", "agent_id", "terminal_id", "provider_run_id", "prompt_id"]
const grantFields = ["grant_id", "holder_pid", "holder_executable", "lifetime_minutes", "expires_at_ms"]
let attachment
let exiting = false
const children = new Set()
const input = createInterface({ input: process.stdin, terminal: false })

function errorReceipt(action, error) {
  const text = String(error)
  emit({ action, ok: false, code: typeof error?.code === "string" ? error.code : "request_refused",
    markers: ["kernel_access_denied", "PASSKEY_NOT_ACCEPTED", "PASSKEY_REQUIRED", "already answered", "refused", "expired", "Unix peer has no live authority"].filter(marker => text.includes(marker)) })
}

async function probe(target = session) {
  try {
    const response = await client.send({ GetSessionState: { session_id: target } })
    if (response.SessionState?.session?.id !== target) throw new Error("MP-11: wrong session response")
    emit({ action: "probe", ok: true, session_id: response.SessionState.session.id })
  } catch (error) { errorReceipt("probe", error); if (mode === "probe") process.exitCode = 1 }
}

async function childCommand(executable, argv) {
  const env = { ...process.env }
  for (const key of Object.keys(env)) if (/^CHARIOX_(KERNEL_LOCAL_AUTH|RELAY_TOKEN)/.test(key)) delete env[key]
  // Direct product CLI responses are reduced to exit status. No stdout/stderr dump.
  const child = spawn(executable, argv, { env, stdio: ["ignore", "ignore", "ignore"] })
  children.add(child)
  await new Promise((done, reject) => { child.once("error", reject); child.once("exit", (code, signal) => { emit({ action: "child", ok: code === 0, exit: code, signal }); done() }) })
  children.delete(child)
}

async function close() {
  if (exiting) return
  exiting = true
  input.close()
  for (const child of children) {
    const pid = child.pid
    if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error("MP-11: invalid cleanup PID")
    if (child.exitCode !== null || child.signalCode !== null) continue
    // Require the still-running OS process to remain our direct child.
    let parent
    try { parent = Number(execFileSync("/bin/ps", ["-o", "ppid=", "-p", String(pid)], { encoding: "utf8" }).trim()) } catch { continue }
    if (parent === process.pid) child.kill("SIGTERM")
  }
  await client.close()
  emit({ action: "client_closed" })
}
process.once("SIGINT", () => { void close() })
process.once("SIGTERM", () => { void close() })

try {
  client.onKernelEvent(event => {
    if (event.event === "passkey_prompts_changed") emit({ action: "popups", prompts: (event.prompts || []).map(prompt => fields(prompt,
      ["session_id", "interaction_id", "kind", "requested_at_ms", "expires_at_ms"])) })
    else if (event.event === "heartbeat" || event.event === "connection_state") emit({ action: "event", event: event.event })
    else if (attachment) emit({ action: "session_event", event: event.event })
  })
  if (mode === "tcp-check") {
    const WebSocket = createRequire(resolve(options.get("--checkout"), "packages/kernel-client/package.json"))("ws")
    for (const test of ["missing", "wrong"]) {
      const status = await new Promise((done, reject) => {
        const socket = new WebSocket(endpoint, { handshakeTimeout: 10000,
          ...(test === "wrong" ? { headers: { Authorization: "Bearer chx_kat_wrong" } } : {}) })
        socket.on("error", () => {})
        socket.once("open", () => { socket.close(); reject(new Error("MP-11: unauthenticated upgrade admitted")) })
        socket.once("unexpected-response", (_request, response) => { response.resume(); socket.terminate(); done(response.statusCode) })
        socket.once("error", error => reject(error))
      })
      emit({ action: "tcp-check", case: test, status, ok: status === 401 })
      if (status !== 401) throw new Error("MP-11: expected 401")
    }
  } else if (mode === "observe") {
    await client.subscribeToWaitingRoomInventory()
    emit({ action: "observing_owner_popups" })
  } else if (mode === "access-list") {
    const result = await client.send({ ListKernelAccessGrants: {} })
    emit({ action: "access-list", grants: (result.KernelAccessGrantsListed?.grants || []).map(grant => fields(grant, grantFields)),
      sudo_turns: (result.KernelAccessGrantsListed?.sudo_turns || []).map(turn => fields(turn, turnFields)) })
  } else if (mode === "holder") {
    const minutes = Number(options.get("--minutes") || 6)
    if (!Number.isSafeInteger(minutes) || minutes < 1) throw new Error("MP-11: invalid lifetime")
    emit({ action: "grant_requested", scope: "local_kernel", holder_pid: process.pid, minutes })
    const result = await client.send({ RequestKernelAccess: { holder_pid: process.pid, lifetime_minutes: minutes } })
    emit({ action: "granted", grant: fields(result.KernelAccessGranted?.grant, grantFields) })
  } else await probe()
  if (mode === "holder" || mode === "observe") {
    emit({ action: "ready", commands: mode === "holder" ? ["probe", "foreign-probe <session-id>", "list", "child", "subscribe", "unsubscribe", "request-sudo <prompt>", "cli-sudo <prompt>", "critical <interaction-id>", "exit"] : ["exit"] })
    for await (const line of input) {
      const [action, ...rest] = line.trim().split(" ")
      try {
        if (action === "exit") break
        if (mode !== "holder") throw new Error("MP-11: observer accepts exit only")
        if (action === "probe") await probe()
        else if (action === "foreign-probe") {
          if (rest.length !== 1 || !rest[0]) throw new Error("MP-11: foreign-probe requires a session ID")
          await probe(rest[0])
        } else if (action === "list") {
          const result = await client.send({ ListSessions: null })
          if (!Array.isArray(result.SessionsListed?.sessions)) throw new Error("MP-11: invalid session list")
          emit({ action, ok: true, session_ids: result.SessionsListed.sessions.map(value => value.id) })
        }
        else if (action === "child") await childCommand(process.execPath, [process.argv[1], "probe", "--checkout", options.get("--checkout"), "--socket", options.get("--socket"), "--session", session])
        else if (action === "subscribe") {
          const result = await client.send({ AttachToSession: { session_id: session, client_id: `sudo-kit-${process.pid}`, capability_level: "FullTerminal" } })
          attachment = result.SessionAttached?.attachment?.id
          if (!attachment) throw new Error("MP-11: no attachment")
          await client.subscribeToKernelEvents(session, attachment)
          emit({ action, ok: true })
        } else if (action === "unsubscribe") {
          await client.unsubscribeFromKernelEvents()
          attachment = undefined
          emit({ action, ok: true })
        } else if (action === "request-sudo") {
          if (!agent || !rest.join(" ")) throw new Error("MP-11: target agent and prompt required")
          emit({ action: "sudo_requested", agent_id: agent })
          const result = await client.send({ RequestKernelSudo: { agent_id: agent, prompt: rest.join(" ") } })
          emit({ action, ok: true, submission: fields(result.KernelSudoRequested, ["entry_id", "agent_id", "session_id"]) })
        } else if (action === "cli-sudo") {
          if (!agent || !options.get("--chariox") || !rest.join(" ")) throw new Error("MP-11: --chariox, target and prompt required")
          await childCommand(options.get("--chariox"), ["sudo", "request", "--socket", options.get("--socket"), "--agent", agent, "--prompt", rest.join(" ")])
        } else if (action === "critical") {
          if (rest.length !== 1 || !rest[0]) throw new Error("MP-11: critical needs an interaction ID")
          await client.send({ RespondToInteraction: { session_id: session, interaction_id: rest[0], choice_id: "approve" } })
          emit({ action, ok: true, interaction_id: rest[0] })
        } else throw new Error("MP-11: unsupported command; passkeys belong only in Chariox popups")
      } catch (error) { errorReceipt(["probe", "foreign-probe", "list", "child", "subscribe", "unsubscribe", "request-sudo", "cli-sudo", "critical"].includes(action) ? action : "unsupported_command", error) }
    }
  }
} catch (error) {
  errorReceipt(mode, error)
  process.exitCode = 1
} finally { await close() }
