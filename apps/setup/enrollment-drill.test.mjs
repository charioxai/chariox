// MP-07 / MP-08 / MP-11: signed mock release, real kernel device/ticket enrollment + relay.
import assert from "node:assert/strict"
import test from "node:test"
import { createServer } from "node:http"
import { spawn } from "node:child_process"
import { createHash, randomBytes } from "node:crypto"
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { createRequire } from "node:module"
import { tmpdir } from "node:os"
import { setupFixture } from "./fixture.mjs"
import { installLocal } from "./installer.mjs"
import { availablePort, stopOwned } from "../kernel/ssh-machine/ssh-target-fixture.mjs"
const { WebSocketServer, WebSocket } = createRequire(new URL("../../packages/kernel-client/package.json", import.meta.url))("ws")
const kernel = process.env.CHARIOX_BYOM_DRILL_KERNEL
if (!kernel?.startsWith("/")) throw new Error("MP-07/MP-08/MP-11: choose exact built CHARIOX_BYOM_DRILL_KERNEL")
const pause = () => new Promise(r => setTimeout(r, 100))
test("MP-07/MP-08/MP-11 real self-setup device approval, stdin ticket, idempotence, readiness and state-retaining uninstall", { timeout: 300_000 }, async t => {
  const dir = await mkdtemp(join(await realpath(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir()), "setup-live-"))
  const errors = [], operations = []
  let phase = "fixture"
  const children = new Set(), services = new Map(), registrations = new Map(), devices = new Map(), credentials = new Map(), calls = [], tickets = new Set()
  let cloud, release, relay
  t.after(async () => {
    t.diagnostic(`MP-07/MP-08/MP-11 seam=${phase}; calls=${calls.join(",")}; kernelOperations=${operations.join(",")}; ownChildren=${[...children].map(c => c.exitCode ?? "running").join(",")}; relayRegistrations=${registrations.size}; fixtureErrors=${errors.length}`)
    for (const child of children) await stopOwned(child)
    if (relay) { for (const socket of relay.clients) socket.terminate(); await new Promise(r => relay.close(r)) }
    for (const server of [cloud, release]) if (server?.listening) await new Promise(r => server.close(r))
    await rm(dir, { recursive: true, force: true })
    t.diagnostic("MP-07/MP-08/MP-11: own fixture state, runtime identities and kernel children removed")
  })
  relay = new WebSocketServer({ host: "127.0.0.1", port: 0 }); await new Promise(r => relay.once("listening", r))
  relay.on("connection", socket => socket.on("message", raw => {
    const frame = JSON.parse(raw)
    if (frame.kind === "daemon_register") registrations.set(frame.registration.daemon_id, frame.registration)
    if (frame.kind === "daemon_metadata_query") socket.send(JSON.stringify({ kind: "daemon_metadata_response", request_id: frame.request_id, kernels: [], machines: [], error: null }))
  }))
  const relayUrl = `ws://127.0.0.1:${relay.address().port}`
  let apiUrl, lastIdentity
  cloud = createServer(async (req, res) => {
    try {
    let raw = ""; for await (const chunk of req) raw += chunk
    const body = raw ? JSON.parse(raw) : {}
    const send = (status, value) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(value)) }
    if (req.url === "/auth/device/start") {
      assert.equal(body.enrollmentKind, "KERNEL")
      assert.ok(Object.keys(body).every(k => ["enrollmentKind","kernelId","machineId","publicKeyThumbprint","kernelAlias","machineAlias"].includes(k)))
      const deviceCode = randomBytes(24).toString("hex"); devices.set(deviceCode, { ...body, approved: false }); calls.push("device-start")
      return send(201, { deviceCode, userCode: "PUBLIC", verificationUrl: `${apiUrl}/approve`, expiresAt: new Date(Date.now() + 60_000).toISOString(), intervalSeconds: 1 })
    }
    if (req.url === "/auth/device/poll") {
      let identity
      if (body.ticket) {
        const allowed = ["ticket","kernelId","machineId","publicKeyThumbprint","kernelAlias"]
        if (Object.keys(body).some(k => !allowed.includes(k)) || (Object.hasOwn(body,"kernelAlias") && typeof body.kernelAlias !== "string")) return send(400,{})
        assert.match(body.publicKeyThumbprint,/^[a-f0-9]{64}$/)
        if (!tickets.delete(body.ticket)) return send(401, {})
        calls.push("ticket-redeem"); identity = body
      } else {
        const device = devices.get(body.deviceCode)
        if (!device?.approved) return send(200, { status: "authorization_pending", intervalSeconds: 1 })
        devices.delete(body.deviceCode); identity = device; calls.push("device-approved")
      }
      lastIdentity = { kernelId:identity.kernelId, machineId:identity.machineId }
      const credential = randomBytes(32).toString("hex"); credentials.set(credential, identity.kernelId)
      return send(200, { status: "approved", kernelCredential: credential, profile: { email: "fixture@example.test", accountId: "account-owner", userId: "owner", accountSlug: "owner", realmId: "owner", issuerId: "fixture", relayUrl, kernelId: identity.kernelId, machineId: identity.machineId, publicKeyThumbprint: identity.publicKeyThumbprint } })
    }
    if (req.url === "/relay/token") {
      calls.push("grant")
      await new Promise(r => setTimeout(r,1500))
      assert.equal(credentials.get(body.kernelCredential), body.subject)
      const payload = Buffer.from(JSON.stringify({ exp: Date.now() + 600_000, public_key_thumbprint: body.publicKeyThumbprint })).toString("base64url")
      return send(200, { token: `fixture.${payload}.fixture`, expiresAt: new Date(Date.now() + 600_000).toISOString() })
    }
    if (req.url === "/kernels/presence") return send(200, {})
    if (req.url.startsWith("/relay/targets")) return send(200, { kernels: [] })
    return send(404, {})
    } catch { calls.push("Cloud-assertion-refused"); errors.push("Cloud schema assertion failed"); res.writeHead(500); res.end("{}") }
  })
  await new Promise(r => cloud.listen(0, "127.0.0.1", r)); apiUrl = `http://127.0.0.1:${cloud.address().port}`
  const f = await setupFixture(dir, { kernel: await readFile(kernel) }), archive = await readFile(f.archive)
  release = createServer((req, res) => {
    const prefix = `/v${f.version}/chariox-${f.version}-linux-x64`
    const bytes = new Map([[`${prefix}.manifest.json`, f.manifest], [`${prefix}.manifest.sig`, f.signature], [`${prefix}.tar.gz`, archive]]).get(req.url)
    if (!bytes) { res.writeHead(404); return res.end() }; res.end(bytes)
  })
  await new Promise(r => release.listen(0, "127.0.0.1", r))
  const home = join(dir, "home"); await mkdir(home)
  const staging = join(home, ".chariox/dev/md-staging"); await mkdir(staging, { recursive: true }); await writeFile(join(staging, "sentinel"), "keep")
  const port = await availablePort(), port2 = await availablePort()
  const probe = async (id, selectedPort, expected) => {
    const authPath = join(home, `.chariox/dev/ssh-machines/${id}/state/kernel-local-auth/${selectedPort}.token`)
    let auth
    for (let i = 0; i < 50; i++) { try { auth = await readFile(authPath, "utf8"); break } catch { await pause() } }
    assert.ok(auth, "MP-11 own kernel must expose its ordinary authenticated status surface")
    const ws = new WebSocket(`ws://127.0.0.1:${selectedPort}/kernel`, { headers: { authorization: `Bearer ${auth.trim()}` } })
    const status = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { ws.terminate(); reject(new Error("MP-11 disconnected cache seed timed out")) }, 5000)
      ws.on("error", () => { clearTimeout(timer); reject(new Error("MP-11 disconnected cache seed failed")) })
      ws.on("open", () => ws.send(JSON.stringify({ type: "request", request_id: "byom-ready", request: { RelayStatus: null } })))
      ws.on("message", bytes => {
        try {
          const value = JSON.parse(bytes)
          if (value.request_id !== "byom-ready") return
          clearTimeout(timer); ws.close(); resolve(value.response?.RelayStatus?.status)
        } catch { clearTimeout(timer); ws.terminate(); reject(new Error("MP-11 invalid own status response")) }
      })
    })
    assert.equal(status?.connected, false, "delayed relay grants force an early disconnected poll")
    assert.equal(status?.daemon_id, expected.kernelId)
    assert.equal(status?.machine_id, expected.machineId)
    t.diagnostic("MP-11 authenticated disconnected cache seeded; kernel and machine bindings match")
  }
  const enabledServices = new Set()
  const serviceManager = async args => {
    const service = args.at(-1), id = service === "chariox-ssh-second.service" ? "second" : "local"
    if (args[0] === "show") {
      const path = join(home, ".config/systemd/user", args[1]); let exists = true
      try { await readFile(path) } catch { exists = false }
      return `LoadState=${exists ? "loaded" : "not-found"}\nFragmentPath=${exists ? path : ""}\nDropInPaths=\nActiveState=${services.has(id) ? "active" : "inactive"}\nUnitFileState=${enabledServices.has(id) ? "enabled" : "disabled"}\n`
    }
    if (args[0] === "enable") enabledServices.add(id)
    if (args[0] === "start" && !services.has(id)) {
      const selectedPort = id === "local" ? port : port2
      const env = { PATH: process.env.PATH, LANG: "C.UTF-8", HOME: home, CHARIOX_HOME: `${home}/.chariox/dev/ssh-machines/${id}`, CHARIOX_KERNEL_HOST: "127.0.0.1", CHARIOX_KERNEL_PORT: String(selectedPort), CHARIOX_MCP_HOST: "127.0.0.1", CHARIOX_MCP_PORT: String(selectedPort + 1), CHARIOX_PROVIDER_PROCESS_ORPHAN_TTL_MS: "18446744073709551615", CODEX_HOME: `${home}/.codex`, CLAUDE_CONFIG_DIR: `${home}/.claude`, OPENCODE_CONFIG_DIR: `${home}/.config/opencode`, XDG_CONFIG_HOME: `${home}/.config`, XDG_DATA_HOME: `${home}/.local/share`, XDG_STATE_HOME: `${home}/.local/state` }
      const child = spawn(`${home}/.local/share/chariox/ssh-machines/${id}/current/bin/chariox-kernel`, [], { env, stdio: ["ignore","ignore","pipe"] });
      child.stderr.on("data", chunk => { const match = chunk.toString().match(/operation: "([a-zA-Z0-9 _.-]{1,80})"/); if (match) operations.push(match[1]); for (const category of ["panicked","public key","token","credential","bind","Address already in use"]) if (chunk.toString().includes(category)) operations.push(`contains-${category.replaceAll(" ","-")}`) }); children.add(child); services.set(id, child); calls.push(`start-${id}`); t.diagnostic(`MP-07/MP-08/MP-11 start ${id}, public kernel port=${selectedPort}`); await probe(id, selectedPort, {...lastIdentity})
    }
    if (args[0] === "disable") enabledServices.delete(id)
    if ((args[0] === "stop" || (args[0] === "disable" && args.includes("--now"))) && services.has(id)) { await stopOwned(services.get(id)); services.delete(id); calls.push(`stop-${id}`) }
    return ""
  }
  const notices = [], options = { home, port, version: f.version, publicKeyHex: f.publicKeyHex, apiUrl, releaseBase: `http://127.0.0.1:${release.address().port}`, extractorSource: await readFile(new URL("../../deploy/managed-kernel/extract-release.py", import.meta.url), "utf8"), serviceManager, servicePersistence: async () => {}, notice: value => notices.push(value), openBrowser: async url => { assert.equal(url, `${apiUrl}/approve`); for (const device of devices.values()) device.approved = true; calls.push("open-browser") } }
  phase = "device install/start/readiness"
  const first = await installLocal(options)
  assert.equal(first.status, "ready"); assert.equal(first.userId, "owner")
  assert.ok(calls.indexOf("device-start") < calls.indexOf("open-browser"))
  assert.ok(calls.indexOf("open-browser") < calls.indexOf("device-approved"))
  const active = join(home, ".chariox/dev/ssh-machines/local/kernels/active")
  const records = await Promise.all((await readdir(active)).filter(name => name.endsWith(".json")).map(async name => JSON.parse(await readFile(join(active,name),"utf8"))))
  assert.ok(records.some(record => record.kernel_id === first.kernelId && Date.now() - record.heartbeat_at_ms < 30_000), "ordinary fresh presence supports repeat-login setup suppression")
  assert.ok(calls.indexOf("device-approved") < calls.indexOf("start-local"))
  for (let i = 0; i < 50 && !registrations.has(first.kernelId); i++) await pause()
  assert.ok(registrations.has(first.kernelId))
  phase = "device repair/readiness"
  assert.equal((await installLocal({ ...options, action: "repair", userId: "owner" })).kernelId, first.kernelId)
  assert.equal(calls.filter(v => v === "device-start").length, 1)
  // A browser code must not silently claim success over an already-enrolled owner's state.
  const ticket = randomBytes(24).toString("hex"); tickets.add(ticket)
  phase = "ambiguous browser code refusal"
  await assert.rejects(installLocal({ ...options, action: "repair", ticket }), /command failed/)
  assert.ok(tickets.has(ticket)); tickets.delete(ticket)
  phase = "first uninstall"
  const removed = await installLocal({ ...options, action: "remove" })
  assert.equal(removed.stateRetained, true)
  assert.deepEqual(await readdir(staging), ["sentinel"])
  // Fresh second install proves browser-code redemption independently of device flow.
  const secondTicket = randomBytes(24).toString("hex"); tickets.add(secondTicket)
  const secondOptions = { ...options, port: port2, installId: "second", ticket: secondTicket }
  phase = "fresh ticket install/start/readiness"
  const second = await installLocal(secondOptions)
  assert.equal(second.status, "ready"); assert.notEqual(second.kernelId, first.kernelId)
  assert.equal(secondOptions.ticket, ""); assert.equal(tickets.size, 0)
  phase = "second uninstall"
  await installLocal({ ...options, port: port2, installId: "second", action: "remove" })
  async function noSecrets(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) await noSecrets(path)
      else if (entry.isFile()) assert.equal((await readFile(path)).includes(Buffer.from(secondTicket)), false, "ticket absent from state/logs")
    }
  }
  await noSecrets(join(home, ".chariox/dev/ssh-machines"))
  assert.deepEqual(errors,[])
  assert.equal(notices.some(value => value.includes(secondTicket)), false)
  phase = "complete"
  t.diagnostic(`MP-07/MP-08/MP-11: seams=${calls.join(",")}; published identities=2; source kernel SHA256=${createHash("sha256").update(await readFile(kernel)).digest("hex")}`)
})
