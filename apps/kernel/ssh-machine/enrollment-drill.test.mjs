// MP-07 / MP-08 / MP-11: real kernels + actual SSH; mock Cloud and relay, surrogate user manager.
import assert from "node:assert/strict"
import test from "node:test"
import { createServer } from "node:http"
import { spawn } from "node:child_process"
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { createHash, randomBytes, randomUUID } from "node:crypto"
import { createRequire } from "node:module"
const { WebSocketServer, WebSocket } = createRequire(new URL("../../../packages/kernel-client/package.json", import.meta.url))("ws")
import { createFixture } from "./fixture.mjs"
import { sshTarget, availablePort, stopOwned } from "./ssh-target-fixture.mjs"

const kernel = process.env.CHARIOX_BYOM_DRILL_KERNEL
if (!kernel?.startsWith("/")) throw new Error("MP-07/MP-08/MP-11: CHARIOX_BYOM_DRILL_KERNEL must select the exact built binary")
const pause = () => new Promise(r => setTimeout(r, 150))
async function serve(server) { await new Promise(r => server.listen(0, "127.0.0.1", r)); return `http://127.0.0.1:${server.address().port}` }
async function bootCommand(env, input) {
  const child = spawn(kernel, ["--owner-managed-enroll-stdin"], { env, stdio: ["pipe", "pipe", "ignore"] })
  let out = ""; child.stdout.on("data", chunk => { out += chunk })
  child.stdin.end(JSON.stringify(input))
  const code = await new Promise((resolve, reject) => { child.once("error", reject); child.once("close", resolve) })
  return { code, out }
}
async function request(url, env, value) {
  const requestId = randomUUID()
  let token
  for (let i = 0; i < 600; i++) {
    try { token = await readFile(join(env.CHARIOX_HOME, "state/kernel-local-auth", `${env.CHARIOX_KERNEL_PORT}.token`), "utf8"); break } catch { await pause() }
  }
  if (!token) throw new Error("kernel did not publish local authentication")
  const socket = new WebSocket(url, { headers: { authorization: `Bearer ${token.trim()}` } })
  const result = new Promise((resolve, reject) => {
    socket.on("error", () => reject(new Error("kernel transport admission failed")))
    socket.on("open", () => socket.send(JSON.stringify({ type: "request", request_id: requestId, request: value })))
    socket.on("message", raw => {
      const frame = JSON.parse(raw)
      if (frame.request_id !== requestId) return
      socket.close()
      if (frame.error) reject(new Error(`kernel request failed (${frame.error.code})`))
      else resolve(frame.response)
    })
  })
  return result
}
test("MP-07/MP-08/MP-11 source-kernel add/remove, stdin redemption, replay refusal and relay registration", { timeout: 300_000 }, async t => {
  let cloud, serviceControl, relay
  const childOperations = []
  const children = new Set(), serviceChildren = new Map(), tickets = new Map(), credentials = new Map(), registrations = new Map()
  const calls = [], issued = [], errors = []
  const h = await sshTarget(t, { beforeCleanup: async () => {
    t.diagnostic(`MP-07/MP-08/MP-11 cleanup: own kernel exits=${[...children].map(c => c.exitCode ?? "running").join(",")}; Cloud seams=${calls.join(",")}; fixtureErrors=${errors.length}; childOperations=${childOperations.join(",")}`)
    for (const child of children) await stopOwned(child)
    if (relay) { for (const socket of relay.clients) socket.terminate(); await new Promise(r => relay.close(r)) }
    for (const server of [cloud, serviceControl]) if (server?.listening) await new Promise(r => server.close(r))
  } })
  relay = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise(r => relay.once("listening", r))
  relay.on("connection", socket => socket.on("message", raw => {
    const envelope = JSON.parse(raw)
    if (envelope.kind === "daemon_register") registrations.set(envelope.registration.daemon_id, envelope.registration)
    if (envelope.kind === "daemon_metadata_query") socket.send(JSON.stringify({ kind: "daemon_metadata_response", request_id: envelope.request_id, machines: [], kernels: [], error: null }))
  }))
  const relayUrl = `ws://127.0.0.1:${relay.address().port}`
  cloud = createServer(async (req, res) => {
    try {
      let raw = ""; for await (const chunk of req) raw += chunk
      const body = raw ? JSON.parse(raw) : {}
      const send = (status, value) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(value)) }
      if (req.url === "/v1/kernel-enrollment-tickets" && req.method === "POST") {
        assert.ok(credentials.has(req.headers["x-chariox-kernel-credential"]))
        assert.deepEqual(body, { purpose: "owner_managed_machine", machine_label: "byom-one", ttl_seconds: 600 })
        const ticket = randomBytes(24).toString("hex"), id = `fixture-${issued.length}`
        issued.push(ticket); tickets.set(ticket, id); calls.push("issue")
        send(200, { ticket, ticket_id: id, expires_at: new Date(Date.now() + 590_000).toISOString() }); return
      }
      if (req.url.startsWith("/v1/kernel-enrollment-tickets/") && req.method === "DELETE") {
        assert.ok(credentials.has(req.headers["x-chariox-kernel-credential"]))
        for (const [ticket,id] of tickets) if (req.url.endsWith(id)) tickets.delete(ticket)
        calls.push("revoke"); send(200, {}); return
      }
      if (req.url === "/auth/device/poll") {
        if (!tickets.has(body.ticket)) { calls.push("replay-refused"); send(401, {}); return }
        tickets.delete(body.ticket)
        const rawKey = Buffer.from(body.publicKey, "utf8")
        assert.equal(body.publicKeyThumbprint, createHash("sha256").update(rawKey).digest("hex"))
        const credential = randomBytes(32).toString("hex"); credentials.set(credential, body.kernelId)
        calls.push("redeem")
        send(200, { status: "approved", kernelCredential: credential, profile: { email: "fixture@example.test", accountId: "owner", userId: "owner", accountSlug: "owner", realmId: "owner", relayUrl, issuerId: "fixture", kernelId: body.kernelId, machineId: body.machineId, publicKeyThumbprint: body.publicKeyThumbprint } }); return
      }
      if (req.url === "/relay/token") {
        assert.equal(credentials.get(body.kernelCredential), body.subject)
        const payload = Buffer.from(JSON.stringify({ exp: Date.now() + 600_000, public_key_thumbprint: body.publicKeyThumbprint })).toString("base64url")
        send(200, { token: `fixture.${payload}.fixture`, expiresAt: new Date(Date.now() + 600_000).toISOString() }); return
      }
      if (req.url === "/kernels/presence") { send(200, {}); return }
      if (req.url.startsWith("/relay/targets")) { send(200, { kernels: [] }); return }
      errors.push("unexpected Cloud route"); send(404, {})
    } catch { errors.push("Cloud contract assertion failed"); res.writeHead(500); res.end("{}"); }
  })
  const apiUrl = await serve(cloud)
  const sourceHome = join(h.scratch, "source-home"), sourceState = join(h.scratch, "source-state")
  await mkdir(sourceHome); await mkdir(sourceState)
  const sourceEnv = { ...process.env, HOME: sourceHome, CHARIOX_HOME: sourceState, CHARIOX_KERNEL_HOST: "127.0.0.1", CHARIOX_KERNEL_PORT: String(await availablePort()), CHARIOX_MCP_HOST:"127.0.0.1", CHARIOX_MCP_PORT:String(await availablePort()) }
  Object.assign(sourceEnv, { CODEX_HOME:join(sourceHome,".codex"), CLAUDE_CONFIG_DIR:join(sourceHome,".claude"), OPENCODE_CONFIG_DIR:join(sourceHome,".config/opencode"), XDG_CONFIG_HOME:join(sourceHome,".config"), XDG_DATA_HOME:join(sourceHome,".local/share"), XDG_STATE_HOME:join(sourceHome,".local/state") })
  for (const key of Object.keys(sourceEnv)) if (key.startsWith("CHARIOX_") && !["CHARIOX_HOME", "CHARIOX_KERNEL_HOST", "CHARIOX_KERNEL_PORT", "CHARIOX_MCP_HOST", "CHARIOX_MCP_PORT"].includes(key)) delete sourceEnv[key]
  const sourceTicket = randomBytes(24).toString("hex"); tickets.set(sourceTicket, "source")
  const boot = await bootCommand(sourceEnv, { ticket: sourceTicket, apiUrl, userId: "owner" })
  t.diagnostic("MP-08 / MP-11 source bootstrap completed")
  assert.equal(boot.code, 0, "source enrolls through product identity generation")
  const sourceIdentity = JSON.parse(boot.out)
  const replayEnv = { ...sourceEnv, CHARIOX_HOME: join(h.scratch, "replay-state"), CHARIOX_KERNEL_PORT: String(await availablePort()), CHARIOX_MCP_HOST:"127.0.0.1", CHARIOX_MCP_PORT:String(await availablePort()) }
  await mkdir(replayEnv.CHARIOX_HOME)
  assert.equal((await bootCommand(replayEnv, { ticket: sourceTicket, apiUrl, userId: "owner" })).code, 1)
  const f = await createFixture(h.scratch, await readFile(kernel))
  // The catalogue carries only the admitted fields; release bytes/public pins stay separate.
  await writeFile(join(sourceState, "ssh-machine-releases.json"), JSON.stringify({ defaultRelease: "fixture", releases: [{ id: "fixture", archive:f.archive, releaseDigest:f.releaseDigest, releasePublicKey:f.releasePublicKey, releasePublicKeyFingerprint:f.releasePublicKeyFingerprint, builderPublicKey:f.builderPublicKey, builderPublicKeyFingerprint:f.builderPublicKeyFingerprint }] }))
  const targetPort = await availablePort()
  serviceControl = createServer(async (req, res) => {
    let raw = ""; for await (const chunk of req) raw += chunk
    const args = JSON.parse(raw)
    if (!args.every(a => !a.endsWith(".service") || a === "chariox-ssh-byom-one.service")) { res.writeHead(403); res.end(); return }
    let out = ""
    if (args[0] === "show") {
      const unit = join(h.home, ".config/systemd/user", args[1]); let exists = true
      try { await readFile(unit) } catch { exists = false }
      out = `LoadState=${exists ? "loaded" : "not-found"}\nFragmentPath=${exists ? unit : ""}\nDropInPaths=\n`
    }
    if (args[0] === "enable" && !serviceChildren.has("byom-one")) {
      const targetEnv = { ...sourceEnv, HOME: h.home, PATH: h.targetPath, CHARIOX_HOME: join(h.home, ".chariox/dev/ssh-machines/byom-one"), CHARIOX_KERNEL_PORT: String(targetPort), CHARIOX_MCP_PORT:String(targetPort + 1) }
      Object.assign(targetEnv, { CODEX_HOME:join(h.home,".codex"), CLAUDE_CONFIG_DIR:join(h.home,".claude"), OPENCODE_CONFIG_DIR:join(h.home,".config/opencode"), XDG_CONFIG_HOME:join(h.home,".config"), XDG_DATA_HOME:join(h.home,".local/share"), XDG_STATE_HOME:join(h.home,".local/state") })
      const child = spawn(join(h.home, ".local/share/chariox/ssh-machines/byom-one/current/usr/local/bin/chariox-kernel"), [], { env: targetEnv, stdio: "ignore" })
      children.add(child); serviceChildren.set("byom-one",child); calls.push("start")
    }
    if (args[0] === "disable") { const child = serviceChildren.get("byom-one"); if (child) { await stopOwned(child); serviceChildren.delete("byom-one") }; calls.push("stop") }
    res.end(out)
  })
  const controlUrl = await serve(serviceControl)
  await writeFile(join(h.bin, "systemctl"), `#!/usr/bin/env node\nconst args=process.argv.slice(2);if(args.shift()!=="--user")process.exit(2);fetch(${JSON.stringify(controlUrl)},{method:"POST",body:JSON.stringify(args)}).then(async r=>{if(!r.ok)process.exitCode=1;else process.stdout.write(await r.text())}).catch(()=>{process.exitCode=1})\n`, { mode: 0o700 })
  const source = spawn(kernel, [], { env:sourceEnv, stdio:["ignore", "ignore", "pipe"] }); children.add(source)
  source.stderr.on("data", chunk => {
    const match = chunk.toString().match(/operation: "([a-zA-Z0-9 _.-]{1,80})"/)
    if (match) childOperations.push(match[1])
    for (const category of ["panicked", "public key", "token", "required", "credential"]) if (chunk.toString().includes(category)) childOperations.push(`contains-${category.replaceAll(" ", "-")}`)
  })
  const url = `ws://127.0.0.1:${sourceEnv.CHARIOX_KERNEL_PORT}`
  const command = { AddSshMachine:{ host:"byom-local", install_id:"byom-one", port:targetPort, release:"fixture" } }
  t.diagnostic("MP-07 / MP-08 source request starting")
  const result = (await request(url, sourceEnv, command)).SshMachine.machine
  assert.equal(result.status, "ready")
  assert.notEqual(result.kernel_id, sourceIdentity.kernelId)
  for (let i=0;i<20 && !registrations.has(result.kernel_id);i++) await pause()
  assert.ok(registrations.has(result.kernel_id), "ordinary relay path sends registration")
  assert.ok(calls.indexOf("issue") < calls.lastIndexOf("redeem") && calls.lastIndexOf("redeem") < calls.indexOf("start"))
  const repeat = (await request(url, sourceEnv, command)).SshMachine.machine
  assert.equal(repeat.kernel_id, result.kernel_id)
  assert.ok(calls.includes("revoke"))
  const removed = (await request(url, sourceEnv, { RemoveSshMachine:{install_id:"byom-one"} })).SshMachine.machine
  assert.equal(removed.status,"removed"); assert.equal(removed.state_retained,true)
  assert.deepEqual(await readdir(join(h.home,".local/share/chariox/ssh-machines")),[])
  assert.ok(await readFile(join(h.home,".chariox/dev/ssh-machines/byom-one/ssh-install-owner.json")))
  async function ticketAbsent(dir) {
    for (const entry of await readdir(dir,{withFileTypes:true})) {
      const path = join(dir,entry.name)
      if (entry.isDirectory()) await ticketAbsent(path)
      else if (entry.isFile()) { const bytes = await readFile(path); assert.equal(issued.some(ticket => bytes.includes(Buffer.from(ticket))),false,"one-time ticket must never persist in target state or logs") }
    }
  }
  await ticketAbsent(join(h.home,".chariox/dev/ssh-machines/byom-one"))
  assert.deepEqual(errors,[])
  assert.equal(tickets.size,0,"issued and unused tickets consumed/revoked")
})
