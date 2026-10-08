#!/usr/bin/env node
// MP-08/MP-10/MP-11: exercise the Mac kit on an existing cfg(test) real kernel.
import assert from "node:assert/strict"
import { spawn, execFileSync } from "node:child_process"
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"
import { createRequire } from "node:module"

const source = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const binary = process.argv[2]
if (!binary?.startsWith("/")) throw new Error("MP-11: absolute source-bound test binary required")
const WebSocket = createRequire(join(source, "packages/kernel-client/package.json"))("ws")
const root = mkdtempSync(join(tmpdir(), "ks-"))
const processes = []
let socket
const delay = ms => new Promise(done => setTimeout(done, ms))
const emit = action => console.log(JSON.stringify({ mp: ["MP-08", "MP-10", "MP-11"], action, ok: true }))

async function until(read, predicate = Boolean) {
  const deadline = Date.now() + 15000
  while (Date.now() < deadline) {
    const value = read()
    if (predicate(value)) return value
    await delay(20)
  }
  throw new Error("MP-11: timed out waiting for fixture seam")
}
function child(executable, argv, extraEnv = {}) {
  const processChild = spawn(executable, argv, { env: { ...process.env, ...extraEnv }, stdio: ["pipe", "pipe", "pipe"] })
  processes.push(processChild)
  const records = []
  let buffer = ""
  processChild.stdout.on("data", data => {
    buffer += data.toString()
    let newline
    while ((newline = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, newline)
      buffer = buffer.slice(newline + 1)
      try { records.push(JSON.parse(line)) } catch {}
    }
  })
  // Drain without output; no raw subprocess payload is used as evidence.
  processChild.stderr.on("data", () => {})
  return { child: processChild, records, command: value => processChild.stdin.write(value + "\n"),
    wait: (action, from = 0) => until(() => records.slice(from).find(record => record.action === action)) }
}
let serial = 0
const responses = new Map()
let accessPrompts = []
async function request(command) {
  const id = `kit-${++serial}`
  socket.send(JSON.stringify({ type: "request", request_id: id, request: command }))
  return until(() => responses.get(id))
}
async function popup(suffix) {
  if (suffix !== "-sudo") {
    const prompt = await until(() => accessPrompts.find(item => item.interaction_id.endsWith(suffix)))
    return { ...prompt, id: prompt.interaction_id }
  }
  const deadline = Date.now() + 15000
  while (Date.now() < deadline) {
    const result = await request({ GetSessionState: { session_id: "access-session" } })
    const pending = result.response.SessionState.session.active_interactions.find(item => suffix === "-sudo" ? item.id.startsWith("sudo:") : item.id.endsWith(suffix))
    if (pending) return pending
    await delay(20)
  }
  throw new Error("MP-11: no fixture popup")
}
async function answer(id, choice, passkey) {
  // Exact synthetic test passkey from existing kernel_access_child_server.
  return request({ RespondToInteraction: { session_id: id.endsWith("-grant") || id.endsWith("-extension") ? "kernel-access" : "access-session", interaction_id: id, choice_id: choice,
    ...(passkey ? { passkey: "Access TEST Passkey" } : {}) } })
}
try {
  const kernel = child(binary, ["kernel_access_child_server", "--ignored", "--nocapture"], {
    CHARIOX_ACCESS_TEST_ROOT: root, CHARIOX_ACCESS_TEST_SUDO: "1", CHARIOX_HOME: join(root, "state"), HOME: join(root, "home"), TMPDIR: root })
  const address = await until(() => { try { return readFileSync(join(root, "ready"), "utf8") } catch { return null } })
  const endpoint = `ws://${address}`
  socket = new WebSocket(endpoint, { headers: { Authorization: "Bearer test-terminal" } })
  await new Promise((done, reject) => { socket.once("open", done); socket.once("error", reject) })
  socket.on("message", bytes => {
    const value = JSON.parse(bytes)
    if (value.request_id) responses.set(value.request_id, value)
    if (value.event?.event === "passkey_prompts_changed") accessPrompts = value.event.prompts || []
  })
  socket.send(JSON.stringify({ type: "subscribe", request_id: "kit-popups", session_id: "", attachment_id: "", subscription_scope: "waiting_room_inventory" }))
  assert.ok(!(await until(() => responses.get("kit-popups"))).error)
  const base = [join(source, "scripts/sudo-kit.mjs")]
  const publicOptions = ["--checkout", source, "--socket", join(root, "run/k.sock"), "--session", "access-session"]
  const observer = child(process.execPath, [...base, "observe", "--checkout", source, "--endpoint", endpoint], { CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: "test-terminal" })
  await observer.wait("observing_owner_popups")
  const tcp = child(process.execPath, [...base, "tcp-check", "--checkout", source, "--endpoint", endpoint])
  await until(() => tcp.child.exitCode !== null)
  assert.equal(tcp.child.exitCode, 0)
  assert.equal(tcp.records.filter(row => row.action === "tcp-check" && row.status === 401).length, 2)
  emit("kit_tokenless_wrong_401")

  const cli = join(root, "chariox-fixture.mjs")
  writeFileSync(cli, `#!/usr/bin/env node\nimport { runSudoCommand } from ${JSON.stringify(pathToFileURL(join(source, "apps/cli/dist/sudo-command.js")).href)};\ntry { await runSudoCommand(process.argv.slice(2)) } catch { process.exitCode = 1 }\n`, { mode: 0o700 })
  const holder = child(process.execPath, [...base, "holder", ...publicOptions, "--agent", "access-vault-agent", "--minutes", "6", "--chariox", cli])
  const prompt = await popup("-grant")
  assert.ok(prompt.message.includes(`pid ${holder.child.pid}`))
  await until(() => observer.records.some(row => row.action === "popups" && row.prompts.some(p => p.interaction_id === prompt.id)))
  assert.ok(!(await answer(prompt.id, "approve", true)).error)
  const granted = await holder.wait("granted")
  assert.equal(granted.grant.holder_pid, holder.child.pid)
  await holder.wait("ready")
  holder.command("do-not-retain-this-text")
  await holder.wait("unsupported_command")
  assert.ok(!JSON.stringify(holder.records).includes("do-not-retain-this-text"))
  emit("kit_grant_real_os_holder_and_owner_popup")

  const sibling = child(process.execPath, [...base, "probe", ...publicOptions])
  await until(() => sibling.child.exitCode !== null)
  assert.equal(sibling.child.exitCode, 1)
  assert.equal((await sibling.wait("probe")).ok, false)
  holder.command("probe")
  assert.equal((await holder.wait("probe")).ok, true)
  holder.command("list")
  assert.deepEqual((await holder.wait("list")).session_ids.sort(), ["access-session", "other-session"])
  const scopeFrom = holder.records.length
  holder.command("foreign-probe other-session")
  assert.equal((await holder.wait("probe", scopeFrom)).ok, true)
  holder.command("child")
  assert.equal((await holder.wait("child")).exit, 0)
  holder.command("subscribe")
  assert.equal((await holder.wait("subscribe")).ok, true)
  holder.command("critical critical-test")
  assert.equal((await holder.wait("critical")).ok, false)
  emit("kit_descendant_sibling_local_kernel_and_critical_boundary")

  for (const method of ["request-sudo", "cli-sudo"]) {
    const marker = `full kit prompt ${method} Ω`
    const from = holder.records.length
    holder.command(`${method} ${marker}`)
    const pending = await popup("-sudo")
    assert.ok(pending.message.includes(marker))
    assert.ok(pending.message.includes(`OS pid ${holder.child.pid}`))
    assert.ok(!(await answer(pending.id, "refuse", false)).error)
    const result = await holder.wait(method === "cli-sudo" ? "child" : method, from)
    assert.equal(result.ok, false)
  }
  emit("kit_external_sudo_client_and_product_cli_refusal")

  const revoked = await request({ RevokeKernelAccessGrant: { grant_id: granted.grant.grant_id } })
  assert.equal(revoked.response.KernelAccessRevoked.revoked, 1)
  const from = holder.records.length
  holder.command("probe")
  assert.equal((await holder.wait("probe", from)).ok, false)
  emit("kit_live_revoke_denies_same_holder")
  const findDb = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? findDb(join(directory, entry.name)) : entry.name === "state.db" ? [join(directory, entry.name)] : [])
  const databases = findDb(root)
  assert.equal(databases.length, 1)
  const receiptText = execFileSync("python3", [join(source, "scripts/sudo-kit-receipts.py"), "--database", databases[0]], { encoding: "utf8" })
  const receipts = receiptText.trim().split("\n").map(line => JSON.parse(line))
  assert.ok(receipts.some(row => row.kind === "kernel_access.grant" && row.outcome === "granted" && row["grant.holder_pid"] === holder.child.pid))
  assert.ok(receipts.some(row => row.kind === "kernel_access.sudo" && row.outcome === "refused_or_cancelled"))
  assert.ok(!receiptText.includes("Access TEST Passkey") && !receiptText.includes("test-terminal") && !receiptText.includes("full kit prompt"))
  emit("kit_public_receipt_projection")
  for (const processClient of [holder, observer]) {
    processClient.command("exit")
    await until(() => processClient.child.exitCode !== null)
    assert.equal(processClient.child.exitCode, 0)
  }
  writeFileSync(join(root, "command"), "stop")
  await until(() => kernel.child.exitCode !== null)
  assert.equal(kernel.child.exitCode, 0)
  emit("kit_graceful_cleanup")
} finally {
  if (socket) socket.close()
  for (const processChild of processes) {
    const pid = processChild.pid
    if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error("MP-11: reject invalid cleanup PID")
    if (processChild.exitCode !== null || processChild.signalCode !== null) continue
    let parent
    try { parent = Number(execFileSync("/bin/ps", ["-o", "ppid=", "-p", String(pid)], { encoding: "utf8" }).trim()) } catch { continue }
    if (parent !== process.pid) throw new Error("MP-11: lost owned child identity")
    processChild.kill("SIGTERM")
    await until(() => processChild.exitCode !== null || processChild.signalCode !== null)
  }
  rmSync(root, { recursive: true })
}
